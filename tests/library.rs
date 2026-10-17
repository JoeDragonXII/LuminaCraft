use lightcraft_catalog::{Album, AlbumId, ColorLabel, Filter, Flag, Op, PhotoId, Sort};
use luminacraft::library::{self, Library, ScanEvent};
use std::{fs, sync::{Arc, atomic::{AtomicBool, Ordering}}};

fn fixture(dir:&std::path::Path,name:&str)->std::path::PathBuf {
    let path=dir.join(name);
    image::RgbaImage::from_fn(32,24,|x,y|image::Rgba([x as u8*5,y as u8*7,80,255])).save(&path).unwrap();
    path
}

#[test]
fn library_reuses_lightcraft_and_persists_management_without_touching_sources(){
    let tmp=tempfile::tempdir().unwrap();
    let source=tmp.path().join("photos");fs::create_dir(&source).unwrap();
    let path=fixture(&source,"照片.png");let xmp=path.with_extension("xmp");fs::write(&xmp,"read-only original XMP").unwrap();
    let original=fs::read(&path).unwrap();let stamp=fs::metadata(&path).unwrap().modified().unwrap();
    let dir=tmp.path().join("catalog");
    let mut lib=Library::open(&dir).unwrap();
    let photo=library::index_photo(&path).unwrap();
    assert_eq!(lib.add(vec![photo.clone(),photo.clone()]).unwrap(),1);
    assert_eq!(lib.add(vec![photo]).unwrap(),0);
    let id=lib.catalog.photos().next().unwrap().id;
    lib.apply(vec![Op::SetRating{id,rating:4},Op::SetFlag{id,flag:Flag::Pick},Op::SetLabel{id,label:Some(ColorLabel::Green)},
        Op::AddAlbum{album:Album::new(AlbumId(1),"旅行")},Op::SetAlbumPhotos{id:AlbumId(1),photos:vec![id]}]).unwrap();
    let filter=Filter{text:"照片".into(),rating:4,flag:Some(Flag::Pick),label:Some(ColorLabel::Green),album:Some(AlbumId(1)),..Default::default()};
    assert_eq!(lib.catalog.query(&filter,&Sort::default()),vec![id]);
    assert!(Library::open(&dir).is_err(),"a second writer must not open the same catalog");
    let thumb=library::thumbnail(path.to_str().unwrap()).unwrap();assert!(thumb.width()<=320);
    drop(lib);
    let mut lib=Library::open(&dir).unwrap();
    assert_eq!(lib.catalog.query(&filter,&Sort::default()),vec![id]);
    assert_eq!(lib.add(vec![library::index_photo(&path).unwrap()]).unwrap(),0);
    assert_eq!(fs::read(&path).unwrap(),original);assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(),stamp);
    assert_eq!(fs::read_to_string(&xmp).unwrap(),"read-only original XMP");
    assert_eq!(fs::read_dir(&source).unwrap().count(),2);
}

#[test]
fn invalid_transaction_is_not_partly_saved(){
    let tmp=tempfile::tempdir().unwrap();let path=fixture(tmp.path(),"test.png");let dir=tmp.path().join("library");
    let mut lib=Library::open(&dir).unwrap();lib.add(vec![library::index_photo(&path).unwrap()]).unwrap();
    let id=lib.catalog.photos().next().unwrap().id;
    assert!(lib.apply(vec![Op::SetRating{id,rating:3},Op::SetRating{id:PhotoId(999),rating:2}]).is_err());
    assert_eq!(lib.catalog.photo(id).unwrap().rating,0);drop(lib);
    assert_eq!(Library::open(&dir).unwrap().catalog.photo(id).unwrap().rating,0);
}

#[test]
fn scan_supports_partial_success_subfolders_and_cancellation(){
    let tmp=tempfile::tempdir().unwrap();let source=tmp.path().join("photos");fs::create_dir_all(source.join("nested")).unwrap();
    fixture(&source,"one.png");fixture(&source.join("nested"),"two.png");fs::write(source.join("bad.arw"),b"not a photo").unwrap();
    let cancel=Arc::new(AtomicBool::new(false));let mut photos=vec![];let mut errors=vec![];let mut complete=false;
    library::scan(vec![source.clone()],tmp.path().join("library"),cancel,|event|{
        match event{ScanEvent::Photo(p)=>photos.push(*p),ScanEvent::Error(e)=>errors.push(e),ScanEvent::Done{cancelled}=>complete=!cancelled} true
    });
    assert!(complete);assert_eq!(photos.len(),2);assert_eq!(errors.len(),1);
    let cancel=Arc::new(AtomicBool::new(true));let mut count=0;let mut cancelled=false;
    library::scan(vec![source],tmp.path().join("library"),cancel,|event|{match event{ScanEvent::Photo(_)=>count+=1,ScanEvent::Done{cancelled:c}=>cancelled=c,_=>{}}true});
    assert_eq!(count,0);assert!(cancelled);
}

#[test]
fn catalog_has_no_fifty_photo_project_limit_and_skips_its_own_directory(){
    let tmp=tempfile::tempdir().unwrap();let source=tmp.path().join("photos");fs::create_dir(&source).unwrap();
    let dir=source.join("catalog");let mut lib=Library::open(&dir).unwrap();fixture(&dir,"cache.png");
    let mut photos=vec![];for i in 0..60{photos.push(library::index_photo(&fixture(&source,&format!("{i}.png"))).unwrap());}
    assert_eq!(lib.add(photos).unwrap(),60);
    let cancel=Arc::new(AtomicBool::new(false));let mut count=0;
    library::scan(vec![source],dir,cancel.clone(),|event|{if let ScanEvent::Photo(_)=event{count+=1;if count==2{cancel.store(true,Ordering::Relaxed);}}true});
    assert_eq!(count,2);
    drop(lib);
    assert_eq!(Library::open(&tmp.path().join("photos/catalog")).unwrap().catalog.photos().count(),60);
}

#[test]
fn selected_photo_can_enter_editor_and_project_remains_separate(){
    let tmp=tempfile::tempdir().unwrap();let path=fixture(tmp.path(),"edit.png");
    let catalog=tmp.path().join("catalog");let mut lib=Library::open(&catalog).unwrap();
    lib.add(vec![library::index_photo(&path).unwrap()]).unwrap();let id=lib.catalog.photos().next().unwrap().id;
    lib.apply(vec![Op::SetRating{id,rating:5}]).unwrap();
    let (mut photo,source)=luminacraft::engine::import_photo(&path).unwrap();
    luminacraft::project::execute(&mut photo,&luminacraft::project::Command::Set{control:"light.exposure".into(),value:0.5}).unwrap();
    let out=luminacraft::engine::render_photo(&source,&photo.edit,320,&AtomicBool::new(false)).unwrap();assert!(out.image.width>0);
    let mut project=luminacraft::project::Project::default();project.photos.push(photo);
    project.save(&tmp.path().join("project")).unwrap();
    let reopened=luminacraft::project::Project::open(&tmp.path().join("project")).unwrap();assert_eq!(reopened.photos[0].edit.develop.light.exposure,0.5);
    assert_eq!(lib.catalog.photo(id).unwrap().rating,5);
    lib.apply(vec![Op::SetDeleted{id,deleted:true}]).unwrap();assert!(lib.catalog.query(&Filter::default(),&Sort::default()).is_empty());
    assert_eq!(lib.catalog.query(&Filter{deleted:true,..Default::default()},&Sort::default()),vec![id]);
    lib.apply(vec![Op::SetDeleted{id,deleted:false}]).unwrap();assert!(path.is_file());
}

#[test]
fn editor_batch_keeps_successes_and_reports_each_failure(){
    use luminacraft::worker::{self,Job,Outcome,Request};
    let tmp=tempfile::tempdir().unwrap();let good=fixture(tmp.path(),"good.png");let bad=tmp.path().join("bad.arw");fs::write(&bad,"invalid RAW").unwrap();
    let (tx,rx)=worker::start(eframe::egui::Context::default());
    tx.send(Request{id:7,job:Job::Import(vec![bad,good]),cancel:Arc::new(AtomicBool::new(false))}).unwrap();
    let response=rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap();assert_eq!(response.id,7);
    match response.result.unwrap(){Outcome::Imported(photos,errors)=>{assert_eq!(photos.len(),1);assert_eq!(errors.len(),1);assert!(errors[0].contains("bad.arw"));},_=>panic!("wrong response")}
}

#[test]
#[ignore = "requires an explicitly selected read-only sample via LUMINACRAFT_READONLY_SAMPLE"]
fn real_raw_library_probe_and_thumbnail_are_read_only(){
    let path=std::path::PathBuf::from(std::env::var_os("LUMINACRAFT_READONLY_SAMPLE").expect("explicit sample path required"));
    let hash=luminacraft::project::hash_file(&path).unwrap();let stamp=fs::metadata(&path).unwrap().modified().unwrap();
    let xmp=path.with_extension("xmp");let xmp_hash=xmp.is_file().then(||luminacraft::project::hash_file(&xmp).unwrap());
    let indexed=library::index_photo(&path).unwrap();assert_eq!(indexed.kind,lightcraft_catalog::MediaKind::Raw);
    let thumb=library::thumbnail(path.to_str().unwrap()).unwrap();assert!(thumb.width()>0&&thumb.height()>0&&thumb.width().max(thumb.height())<=320);
    assert_eq!(luminacraft::project::hash_file(&path).unwrap(),hash);assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(),stamp);
    if let Some(hash)=xmp_hash{assert_eq!(luminacraft::project::hash_file(&xmp).unwrap(),hash);}
    println!("RAW index {}x{}, browsing thumbnail {}x{}, source unchanged",indexed.width,indexed.height,thumb.width(),thumb.height());
}

#[test]
fn reimport_can_restore_removed_record_without_losing_ratings_and_albums(){
    let tmp=tempfile::tempdir().unwrap();let path=fixture(tmp.path(),"test.png");let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();
    lib.add(vec![library::index_photo(&path).unwrap()]).unwrap();let id=lib.catalog.photos().next().unwrap().id;
    let mut album=Album::new(AlbumId(1),"keep");album.photos=vec![id];
    lib.apply(vec![Op::AddAlbum{album},Op::SetRating{id,rating:4},Op::SetDeleted{id,deleted:true}]).unwrap();
    assert_eq!(lib.add_with_restore(vec![library::index_photo(&path).unwrap()],false).unwrap(),(0,0));assert!(lib.catalog.photo(id).unwrap().deleted);
    assert_eq!(lib.add_with_restore(vec![library::index_photo(&path).unwrap()],true).unwrap(),(0,1));
    assert!(!lib.catalog.photo(id).unwrap().deleted);assert_eq!(lib.catalog.photo(id).unwrap().rating,4);assert_eq!(lib.catalog.album_photos(AlbumId(1)),vec![id]);
    assert_eq!(lib.catalog.photos().count(),1);
}
