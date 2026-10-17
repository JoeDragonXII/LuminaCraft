use std::{fs,io::Write,path::Path,sync::atomic::AtomicBool};
use luminacraft::{catalog_file as file,catalog_io::Export,library::{Library,index_photo,photo_path},project::{Project,CatalogRef,hash_file}};
use lightcraft_catalog::Op;
fn fixture(dir:&Path)->(Library,Project){let source=dir.join("original.png");image::RgbImage::from_pixel(32,24,image::Rgb([80,90,100])).save(&source).unwrap();let mut lib=Library::open(&dir.join("library")).unwrap();lib.add(vec![index_photo(&source).unwrap()]).unwrap();let id=lib.catalog.photos().next().unwrap().id;let (mut p,_)=luminacraft::engine::import_photo(&source).unwrap();p.edit.retouch.smooth=0.3;p.catalog_ref=Some(CatalogRef{library:lib.directory.clone(),id:id.0,copy_name:None});let mut project=Project::default();project.photos.push(p);(lib,project)}
fn export(lib:&Library,p:Project)->Export{Export{catalog:lib.catalog.clone(),library:lib.directory.clone(),project:Some(p),selected:None,copy_originals:false}}

#[test]
fn one_file_opens_saves_and_moves_without_copying_originals(){
    let temp=tempfile::tempdir().unwrap();let (lib,p)=fixture(temp.path());let source=p.photos[0].source.path.clone();let hash=hash_file(&source).unwrap();let target=temp.path().join("旅行.lumcat");
    file::export(export(&lib,p),&target,&AtomicBool::new(false)).unwrap();assert!(target.is_file());assert!(!temp.path().join("旅行").exists());
    let mut zip=zip::ZipArchive::new(fs::File::open(&target).unwrap()).unwrap();assert!((0..zip.len()).all(|n|!zip.by_index(n).unwrap().name().starts_with("Originals/")));drop(zip);
    let cache=temp.path().join("cache");let dir=file::prepare_in(&target,&cache,&AtomicBool::new(false)).unwrap();let mut opened=Library::open(&dir).unwrap();let id=opened.catalog.photos().next().unwrap().id;assert_eq!(Path::new(photo_path(opened.catalog.photo(id).unwrap()).unwrap()),source);
    opened.apply(vec![Op::SetRating{id,rating:5}]).unwrap();let mut project=luminacraft::catalog_io::load_project(&dir,&opened).unwrap();project.photos[0].edit.develop.light.exposure=1.5;luminacraft::catalog_io::save_project_develop(&project,&mut opened).unwrap();project.save(&dir).unwrap();assert!(opened.save_file().unwrap());drop(opened);
    // A different cache simulates a second computer: only the catalog file is transported.
    let moved=temp.path().join("renamed.lumcat");fs::rename(&target,&moved).unwrap();let dir2=file::prepare_in(&moved,&temp.path().join("cache-two"),&AtomicBool::new(false)).unwrap();let opened=Library::open(&dir2).unwrap();assert_eq!(opened.catalog.photo(id).unwrap().rating,5);let p=luminacraft::catalog_io::load_project(&dir2,&opened).unwrap();assert_eq!(p.photos[0].edit.develop.light.exposure,1.5);assert_eq!(p.photos[0].edit.retouch.smooth,0.3);assert_eq!(p.photos[0].source.path,source);assert_eq!(hash_file(&source).unwrap(),hash);
}

#[test]
fn crash_workspace_is_recovered_and_external_changes_are_never_overwritten(){
    let temp=tempfile::tempdir().unwrap();let (lib,p)=fixture(temp.path());let target=temp.path().join("work.lumcat");file::export(export(&lib,p),&target,&AtomicBool::new(false)).unwrap();let cache=temp.path().join("cache");let dir=file::prepare_in(&target,&cache,&AtomicBool::new(false)).unwrap();let mut opened=Library::open(&dir).unwrap();let id=opened.catalog.photos().next().unwrap().id;opened.apply(vec![Op::SetRating{id,rating:4}]).unwrap();drop(opened);
    assert_eq!(file::prepare_in(&target,&cache,&AtomicBool::new(false)).unwrap(),dir);let mut opened=Library::open(&dir).unwrap();assert_eq!(opened.catalog.photo(id).unwrap().rating,4);
    fs::OpenOptions::new().append(true).open(&target).unwrap().write_all(b"external change").unwrap();let changed=hash_file(&target).unwrap();assert!(opened.save_file().is_err());assert_eq!(hash_file(&target).unwrap(),changed);drop(opened);assert!(file::prepare_in(&target,&cache,&AtomicBool::new(false)).is_err());assert!(dir.join("catalog.snap").exists());
}

#[test]
fn active_single_file_cannot_be_opened_by_another_native_writer(){
    let temp=tempfile::tempdir().unwrap();let (lib,p)=fixture(temp.path());let target=temp.path().join("work.lumcat");file::export(export(&lib,p),&target,&AtomicBool::new(false)).unwrap();let cache=temp.path().join("cache");let dir=file::prepare_in(&target,&cache,&AtomicBool::new(false)).unwrap();let held=Library::open(&dir).unwrap();let second=file::prepare_in(&target,&cache,&AtomicBool::new(false)).unwrap();assert!(Library::open(&second).is_err());drop(held);
}

#[test]
fn cancelled_exports_and_existing_files_stay_intact(){
    let temp=tempfile::tempdir().unwrap();let (lib,p)=fixture(temp.path());let target=temp.path().join("work.lumcat");file::export(export(&lib,p.clone()),&target,&AtomicBool::new(false)).unwrap();let hash=hash_file(&target).unwrap();assert!(file::export(export(&lib,p.clone()),&target,&AtomicBool::new(false)).is_err());assert_eq!(hash_file(&target).unwrap(),hash);let cancelled=temp.path().join("cancelled.lumcat");assert!(file::export(export(&lib,p),&cancelled,&AtomicBool::new(true)).is_err());assert!(!cancelled.exists());assert!(!fs::read_dir(temp.path()).unwrap().any(|e|e.unwrap().file_name().to_string_lossy().starts_with(".lumcat-")));
}

#[test]
fn invalid_or_traversing_archive_is_rejected_without_escaping_cache(){
    let temp=tempfile::tempdir().unwrap();let target=temp.path().join("hostile.lumcat");let mut zip=zip::ZipWriter::new(fs::File::create(&target).unwrap());let opt=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);zip.start_file("lumina-file.json",opt).unwrap();zip.write_all(br#"{"format":"LuminaCraft catalog file","version":1}"#).unwrap();zip.start_file("../escape.txt",opt).unwrap();zip.write_all(b"invalid").unwrap();zip.finish().unwrap();assert!(file::prepare_in(&target,&temp.path().join("cache"),&AtomicBool::new(false)).is_err());assert!(!temp.path().join("escape.txt").exists());
    fs::write(&target,"not a catalog file").unwrap();assert!(file::prepare_in(&target,&temp.path().join("other-cache"),&AtomicBool::new(false)).is_err());
}

#[test]
fn portable_option_and_crc_checks_keep_embedded_originals_safe(){
    use std::io::{Seek,SeekFrom,Read};
    let temp=tempfile::tempdir().unwrap();let (lib,p)=fixture(temp.path());let source=p.photos[0].source.path.clone();let target=temp.path().join("portable.lumcat");let mut e=export(&lib,p);e.copy_originals=true;file::export(e,&target,&AtomicBool::new(false)).unwrap();fs::rename(&source,temp.path().join("offline.png")).unwrap();
    let dir=file::prepare_in(&target,&temp.path().join("cache"),&AtomicBool::new(false)).unwrap();let opened=Library::open(&dir).unwrap();let p=luminacraft::catalog_io::load_project(&dir,&opened).unwrap();assert!(p.photos[0].source.path.is_file());assert!(p.photos[0].source.path.starts_with(&dir));drop(opened);
    let offset={let mut zip=zip::ZipArchive::new(fs::File::open(&target).unwrap()).unwrap();zip.by_name("catalog.snap").unwrap().data_start()};let mut f=fs::OpenOptions::new().read(true).write(true).open(&target).unwrap();f.seek(SeekFrom::Start(offset)).unwrap();let mut byte=[0];f.read_exact(&mut byte).unwrap();f.seek(SeekFrom::Start(offset)).unwrap();f.write_all(&[byte[0]^1]).unwrap();drop(f);assert!(file::prepare_in(&target,&temp.path().join("other-cache"),&AtomicBool::new(false)).is_err());
}

#[test]
fn metadata_only_catalog_keeps_edits_when_originals_are_temporarily_offline(){
    let temp=tempfile::tempdir().unwrap();let (lib,p)=fixture(temp.path());let source=p.photos[0].source.path.clone();fs::rename(source,temp.path().join("offline.png")).unwrap();let target=temp.path().join("offline.lumcat");file::export(export(&lib,p),&target,&AtomicBool::new(false)).unwrap();let dir=file::prepare_in(&target,&temp.path().join("cache"),&AtomicBool::new(false)).unwrap();let mut opened=Library::open(&dir).unwrap();let p=luminacraft::catalog_io::load_project(&dir,&opened).unwrap();assert_eq!(p.photos.len(),1);assert_eq!(p.photos[0].edit.retouch.smooth,0.3);assert!(!p.photos[0].source.path.exists());let id=opened.catalog.photos().next().unwrap().id;opened.apply(vec![Op::SetRating{id,rating:3}]).unwrap();opened.save_file().unwrap();
}

