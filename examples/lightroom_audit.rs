use anyhow::{Context, Result};
use serde_json::json;
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let source = PathBuf::from(args.get(0).context("catalog path required")?);
    let out = PathBuf::from(args.get(1).context("isolated output directory required")?);
    fs::create_dir_all(&out)?;
    let hash_before = luminacraft::project::hash_file(&source)?;
    let time_before = fs::metadata(&source)?.modified()?;
    let data = lightcraft_engine::lightroom_catalog::read(&source).map_err(anyhow::Error::msg)?;
    let mut ratings = BTreeMap::<String, usize>::new();
    let mut extensions = BTreeMap::<String, usize>::new();
    let mut exists = 0;
    let mut settings_bytes = 0;
    let mut xmp_bytes = 0;
    let mut virtual_copies = 0;
    for photo in &data.photos {
        *ratings.entry(photo.image.get("rating").map(ToString::to_string).unwrap_or_else(|| "null".into())).or_default() += 1;
        *extensions.entry(PathBuf::from(&photo.path).extension().map(|e|e.to_string_lossy().to_lowercase()).unwrap_or_default()).or_default() += 1;
        exists += usize::from(std::path::Path::new(&photo.path).is_file());
        settings_bytes += photo.settings.len(); xmp_bytes += photo.xmp.len();
        virtual_copies += usize::from(photo.image.get("masterImage").and_then(|v|v.as_i64()).unwrap_or(0) != 0);
    }
    let summary = json!({"photos":data.photos.len(),"existingPaths":exists,"missingPaths":data.photos.len()-exists,
        "collections":data.collections.len(),"memberships":data.members.len(),"virtualCopies":virtual_copies,
        "historySteps":data.photos.iter().map(|p|p.history.len()).sum::<usize>(),
        "snapshots":data.photos.iter().map(|p|p.snapshots.len()).sum::<usize>(),
        "ratings":ratings,"formats":extensions,"settingsBytes":settings_bytes,"decodedXmpBytes":xmp_bytes,"warnings":data.warnings});
    println!("{}", serde_json::to_string(&summary)?);
    write_new(out.join("inspection.json"), &serde_json::to_vec_pretty(&summary)?)?;
    let raw = serde_json::to_vec(&data)?;
    write_new(out.join("source-records.json"), &raw)?;
    let restored: lightcraft_engine::lightroom_catalog::CatalogImport = serde_json::from_slice(&fs::read(out.join("source-records.json"))?)?;
    anyhow::ensure!(serde_json::to_value(&data)? == serde_json::to_value(restored)?, "source-record JSON roundtrip differs");
    if args.iter().any(|a| a == "--import") {
        println!("Importing into isolated catalog; originals are referenced only.");
        let mut session = lightcraft_engine::Session::new().with_fs().with_system_clock();
        session.open_library(out.join("library"), false)?;
        let report = session.execute("library.importLightroom", &json!({"path":source}))?;
        write_new(out.join("import-report.json"), &serde_json::to_vec_pretty(&report)?)?;
        println!("{}", serde_json::to_string(&json!({"photos":report["photos"],"imported":report["imported"],"missing":report["missing"],"collections":report["collections"],"unmappedCount":report["unmapped"].as_array().map(Vec::len),"warningsCount":report["warnings"].as_array().map(Vec::len),"failed":report["failed"],"archive":report["archive"]}))?);
        session.close_library()?; drop(session);
        let mut reopened = lightcraft_engine::Session::new().with_fs();
        reopened.open_library(out.join("library"), false)?;
        let persisted = json!({"photos":reopened.catalog.len(),"albums":reopened.catalog.albums().count(),"sourceRecordJsonRoundtrip":true});
        write_new(out.join("persistence.json"), &serde_json::to_vec_pretty(&persisted)?)?;
        println!("{persisted}");
        reopened.close_library()?;
    }
    anyhow::ensure!(hash_before == luminacraft::project::hash_file(&source)? && time_before == fs::metadata(&source)?.modified()?, "source catalog changed during audit");
    write_new(out.join("source-unchanged.json"), &serde_json::to_vec_pretty(&json!({"sha256":hash_before,"unchanged":true}))?)?;
    Ok(())
}
fn write_new(path: PathBuf, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?; file.sync_all()?; Ok(())
}
