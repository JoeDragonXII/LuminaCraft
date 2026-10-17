//! Read-only use of a user-selected real schema, with synthetic photo fixtures only.
use anyhow::{Result,Context,ensure};
use lightcraft_catalog::{Album,AlbumId,Op,Flag};
use luminacraft::{library::{Library,index_photo},lightroom_export::Export,project::hash_file};
use std::{path::PathBuf,fs,sync::atomic::AtomicBool};
fn main()->Result<()> {
    let template=PathBuf::from(std::env::args().nth(1).context("template.lrcat required")?);
    let out=PathBuf::from(std::env::args().nth(2).context("new private evidence directory required")?);fs::create_dir(&out)?;
    let before=hash_file(&template)?;let modified=fs::metadata(&template)?.modified()?;
    let mut lib=Library::open(&out.join("source-library"))?;
    for n in 0..3 {let p=out.join(format!("测试原片-{n}.png"));image::RgbImage::from_pixel(24,16,image::Rgb([n*40+20,70,120])).save(&p)?;lib.add(vec![index_photo(&p)?])?;}
    let mut originals:Vec<_>=lib.catalog.photos().cloned().collect();originals.sort_by_key(|p|p.id);
    let original_hashes:Vec<_>=originals.iter().map(|p|{let path=PathBuf::from(luminacraft::library::photo_path(p).unwrap());(path.clone(),hash_file(&path).unwrap())}).collect();
    let master=originals[0].id;let mut ops=vec![];
    for n in 0..125 {let mut p=(*originals[0]).clone();p.id=lib.catalog.alloc_photo_id();p.copy_of=Some(master);p.copy_name=Some(format!("虚拟版本 {n}"));p.rating=(n%6) as u8;p.flag=if n%2==0{Flag::Pick}else{Flag::Reject};p.label=Some(lightcraft_catalog::ColorLabel::Blue);p.meta.keywords=vec![format!("旅行|北京|景色{}",n%3)];ops.push(Op::AddPhoto{photo:Box::new(p)});}
    lib.apply(ops)?;
    let mut folder=Album::new(AlbumId(1),"旅程文件夹");folder.folder=true;
    let mut album=Album::new(AlbumId(2),"北京精选");album.parent=Some(folder.id);album.photos=lib.catalog.photos().filter(|p|p.rating>2).map(|p|p.id).collect();
    lib.apply(vec![Op::AddAlbum{album:folder},Op::AddAlbum{album}])?;
    let expected=lib.catalog.clone();let output=out.join("新目录.lrcat");let export=Export{catalog:expected.clone(),template:template.clone(),selected:None};
    let report=export.run(&output,&AtomicBool::new(false))?;ensure!(report.photos==128&&report.files==3,"fixture export mismatch");
    let mut full=lightcraft_engine::Session::new().with_fs().with_system_clock();full.open_library(out.join("readback-library"),false)?;
    let imported=full.execute("library.importLightroom",&serde_json::json!({"path":output}))?;
    ensure!(full.catalog.len()==128,"full engine import count");ensure!(full.catalog.photos().filter(|p|p.copy_of.is_some()).count()==125,"virtual copies lost");
    for p in expected.photos(){let q=full.catalog.photos().find(|q|q.file_name==p.file_name&&q.copy_name==p.copy_name).context("missing full-engine photo")?;ensure!(q.rating==p.rating&&q.flag==p.flag&&q.label==p.label&&q.meta.keywords==p.meta.keywords,"full engine metadata differs");if let Some(master)=q.copy_of{ensure!(full.catalog.photo(master).is_some_and(|m|m.copy_of.is_none()),"master linkage");}}
    let album=full.catalog.albums().find(|a|a.name=="北京精选").context("missing album")?;ensure!(album.photos.len()==expected.album(AlbumId(2)).unwrap().photos.len(),"album member mismatch");ensure!(full.catalog.album(album.parent.context("missing collection folder")?).unwrap().name=="旅程文件夹","album hierarchy");
    full.close_library()?;drop(full);let mut reopened=lightcraft_engine::Session::new().with_fs();reopened.open_library(out.join("readback-library"),false)?;ensure!(reopened.catalog.len()==128,"persistence mismatch");reopened.close_library()?;
    for (path,sha) in original_hashes{ensure!(hash_file(&path)?==sha,"synthetic original changed");}
    ensure!(hash_file(&template)?==before&&fs::metadata(&template)?.modified()?==modified,"template changed");
    let result=serde_json::json!({"export":report,"engineImport":imported,"persistedPhotos":128,"templateUnchanged":true,"originalsUnchanged":true,"fullEngineMetadataCompared":true});
    fs::write(out.join("roundtrip-report.json"),serde_json::to_vec_pretty(&result)?)?;println!("{}",serde_json::to_string(&result)?);Ok(())
}
