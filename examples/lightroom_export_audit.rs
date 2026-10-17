use anyhow::{Context, Result};
use lightcraft_engine::export::{ExportFormat, ExportOptions, Resize};
use serde_json::json;
use std::{fs, io::Write, path::PathBuf};
fn main() -> Result<()> {
    let base = PathBuf::from(std::env::args().nth(1).context("isolated audit directory required")?);
    let mut session = lightcraft_engine::Session::new().with_fs();
    session.open_library(base.join("library"), false)?;
    let rating_counts = (0..=5).map(|rating| (rating.to_string(), session.catalog.photos().filter(|p|p.rating == rating).count())).collect::<std::collections::BTreeMap<_,_>>();
    let albums = session.catalog.albums().map(|a|json!({"name":a.name,"smart":a.is_smart(),"folder":a.folder,"members":a.photos.len()})).collect::<Vec<_>>();
    let raw = session.catalog.photos().find(|p|p.format.eq_ignore_ascii_case("ARW") && p.rating > 0 && luminacraft::library::photo_path(p).is_some_and(|p|std::path::Path::new(p).is_file())).context("no available rated RAW")?.clone();
    let jpeg = session.catalog.photos().find(|p|p.format.eq_ignore_ascii_case("JPEG") || p.format.eq_ignore_ascii_case("JPG")).context("no JPEG")?.clone();
    let source = luminacraft::library::photo_path(&raw).context("RAW path")?;
    let hash = luminacraft::project::hash_file(std::path::Path::new(source))?;
    let time = fs::metadata(source)?.modified()?;
    let original = lightcraft_engine::export::export_photo(&mut session, raw.id, &ExportOptions {format:ExportFormat::Original, ..Default::default()}, 1).map_err(anyhow::Error::msg)?;
    let dest = base.join("exports"); fs::create_dir_all(&dest)?;
    let copied_path = dest.join(&original.file_name);
    write_new(&copied_path, &original.bytes)?;
    let packet = original.sidecars.iter().find(|(ext,_)|*ext == "xmp").context("no XMP sidecar")?;
    let xmp_path = copied_path.with_extension("xmp"); write_new(&xmp_path, &packet.1)?;
    let xmp = std::str::from_utf8(&packet.1)?;
    let parsed = lightcraft_engine::sidecar::parse_sidecar(xmp, lightcraft_engine::crs::Target::for_photo(&raw)).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(parsed.rating == Some(raw.rating), "XMP rating differs");
    anyhow::ensure!(hash == luminacraft::project::hash_file(&copied_path)?, "copied raw bytes differ");
    anyhow::ensure!(hash == luminacraft::project::hash_file(std::path::Path::new(source))? && time == fs::metadata(source)?.modified()?, "RAW source changed");
    let rendered = lightcraft_engine::export::export_photo(&mut session, jpeg.id, &ExportOptions {format:ExportFormat::Jpeg, resize:Some(Resize::long_edge(800)), ..Default::default()}, 1).map_err(anyhow::Error::msg)?;
    let jpg_path = dest.join("rendered-preview.jpg"); write_new(&jpg_path, &rendered.bytes)?;
    let dims = image::image_dimensions(&jpg_path)?;
    let report = json!({"ratingsAfterImport":rating_counts,"albums":albums,"rawOriginalExport":{"path":copied_path,"byteIdentical":true,"sourceUnchanged":true,"xmp":xmp_path,"rating":parsed.rating,"hasAdobeCrsNamespace":xmp.contains("camera-raw-settings"),"lightroomAppReimportVerified":false},"jpegExport":{"path":jpg_path,"width":dims.0,"height":dims.1},"lrcatExportImplemented":false});
    write_new(&base.join("export-audit.json"), &serde_json::to_vec_pretty(&report)?)?;
    println!("{report}"); session.close_library()?; Ok(())
}
fn write_new(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)?; f.sync_all()?; Ok(())
}
