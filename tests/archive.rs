use luminacraft::{archive::{self,ImportMode,Layout,ImportOptions},library::{self,Library}};
use std::{fs,path::{Path,PathBuf},sync::atomic::{AtomicBool,Ordering}};
fn photo(root:&Path,name:&str,seed:u8)->PathBuf{
    fs::create_dir_all(root).unwrap();let path=root.join(name);
    image::RgbImage::from_pixel(20,16,image::Rgb([seed,80,110])).save(&path).unwrap();path
}
fn options(root:&Path,source:&Path,mode:ImportMode)->ImportOptions{ImportOptions{mode,destination:root.join("archive"),layout:Layout::Flat,restore:true,source_roots:vec![source.into()]}}
#[test]
fn copy_preserves_source_and_both_xmp_conventions_and_never_overwrites(){
    let tmp=tempfile::tempdir().unwrap();let src=tmp.path().join("source");let path=photo(&src,"a.png",10);
    fs::write(src.join("a.XMP"),"paired original").unwrap();fs::write(src.join("a.png.xmp"),"full-name original").unwrap();
    let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();let opts=options(tmp.path(),&src,ImportMode::Copy);
    fs::create_dir(&opts.destination).unwrap();fs::write(opts.destination.join("a.png"),"existing user file").unwrap();
    let hash=luminacraft::project::hash_file(&path).unwrap();let stamp=fs::metadata(&path).unwrap().modified().unwrap();
    let report=archive::run_import(&mut lib,vec![library::index_photo(&path).unwrap()],&opts,&AtomicBool::new(false),|_,_|{}).unwrap();
    assert_eq!(report.count("copied"),1);assert_eq!(fs::read_to_string(opts.destination.join("a.png")).unwrap(),"existing user file");
    assert!(opts.destination.join("a-1.png").is_file());assert_eq!(fs::read_to_string(opts.destination.join("a-1.XMP")).unwrap(),"paired original");
    assert_eq!(fs::read_to_string(opts.destination.join("a-1.png.xmp")).unwrap(),"full-name original");assert_eq!(luminacraft::project::hash_file(&path).unwrap(),hash);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(),stamp);assert_eq!(fs::read_to_string(src.join("a.XMP")).unwrap(),"paired original");
    let recorded=library::photo_path(lib.catalog.photos().next().unwrap()).unwrap();assert_eq!(Path::new(recorded),opts.destination.join("a-1.png").canonicalize().unwrap());
    assert!(report.report_path.unwrap().is_file());
}
#[test]
fn move_commits_target_then_removes_source_and_sidecar(){
    let tmp=tempfile::tempdir().unwrap();let src=tmp.path().join("source");let path=photo(&src,"a.png",20);fs::write(src.join("a.xmp"),"original").unwrap();
    let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();let opts=options(tmp.path(),&src,ImportMode::Move);
    let report=archive::run_import(&mut lib,vec![library::index_photo(&path).unwrap()],&opts,&AtomicBool::new(false),|_,_|{}).unwrap();
    assert_eq!(report.count("moved"),1);assert!(!path.exists());assert!(!src.join("a.xmp").exists());assert!(opts.destination.join("a.png").is_file());
    assert_eq!(fs::read_to_string(opts.destination.join("a.xmp")).unwrap(),"original");drop(lib);
    assert_eq!(Library::open(&tmp.path().join("catalog")).unwrap().catalog.photos().count(),1);
}
#[test]
fn shared_xmp_is_kept_and_duplicates_are_not_deleted_by_move(){
    let tmp=tempfile::tempdir().unwrap();let src=tmp.path().join("source");let path=photo(&src,"a.png",30);photo(&src,"a.jpg",40);fs::write(src.join("a.xmp"),"shared").unwrap();
    let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();let opts=options(tmp.path(),&src,ImportMode::Move);
    let preview=library::index_photo(&path).unwrap();let duplicate=src.join("b.png");fs::copy(&path,&duplicate).unwrap();
    let report=archive::run_import(&mut lib,vec![preview,library::index_photo(&duplicate).unwrap()],&opts,&AtomicBool::new(false),|_,_|{}).unwrap();
    assert_eq!(report.count("moved"),1);assert_eq!(report.count("skipped"),1);assert!(duplicate.is_file());assert!(src.join("a.xmp").is_file());assert!(report.items[0].detail.contains("共享"));
}
#[test]
fn catalog_write_failure_preserves_move_sources_and_rolls_back_created_targets(){
    let tmp=tempfile::tempdir().unwrap();let src=tmp.path().join("source");let path=photo(&src,"a.png",50);fs::write(src.join("a.xmp"),"original").unwrap();
    let catalog=tmp.path().join("catalog");let mut lib=Library::open(&catalog).unwrap();fs::create_dir(catalog.join("catalog.log")).unwrap();
    let opts=options(tmp.path(),&src,ImportMode::Move);
    let report=archive::run_import(&mut lib,vec![library::index_photo(&path).unwrap()],&opts,&AtomicBool::new(false),|_,_|{}).unwrap();
    assert_eq!(report.count("failed"),1);assert!(path.is_file());assert!(src.join("a.xmp").is_file());assert!(!opts.destination.join("a.png").exists());assert!(!opts.destination.join("a.xmp").exists());
}
#[test]
fn date_layout_cancel_and_overlap_checks(){
    let tmp=tempfile::tempdir().unwrap();let src=tmp.path().join("source");let a=photo(&src,"a.png",60);let b=photo(&src,"b.png",70);
    let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();let mut opts=options(tmp.path(),&src,ImportMode::Copy);opts.layout=Layout::Day;
    let mut first=library::index_photo(&a).unwrap();first.captured=Some("2024-05-06T12:00:00".into());
    // Use the actual filesystem date fallback for source previews; folder generation delegates
    // to LightCraft Organize and is tested independently of source EXIF parsing.
    let cancel=AtomicBool::new(false);let report=archive::run_import(&mut lib,vec![first,library::index_photo(&b).unwrap()],&opts,&cancel,|done,_|{if done==1{cancel.store(true,Ordering::Relaxed);}}).unwrap();
    assert!(report.cancelled);assert_eq!(report.count("copied"),1);assert!(a.is_file()&&b.is_file());assert_eq!(lib.catalog.photos().count(),1);
    let stored=PathBuf::from(library::photo_path(lib.catalog.photos().next().unwrap()).unwrap());assert_ne!(stored.parent().unwrap(),opts.destination);
    opts.destination=src.join("nested");assert!(archive::validate_options(&opts).is_err());assert!(!opts.destination.exists());
}
#[test]
fn source_changed_since_preview_is_not_moved(){
    let tmp=tempfile::tempdir().unwrap();let src=tmp.path().join("source");let path=photo(&src,"a.png",80);let preview=library::index_photo(&path).unwrap();photo(&src,"a.png",90);
    let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();let opts=options(tmp.path(),&src,ImportMode::Move);
    let report=archive::run_import(&mut lib,vec![preview],&opts,&AtomicBool::new(false),|_,_|{}).unwrap();assert_eq!(report.count("failed"),1);assert!(path.is_file());assert_eq!(lib.catalog.photos().count(),0);
}

