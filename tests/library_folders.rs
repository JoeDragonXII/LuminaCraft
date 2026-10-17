use luminacraft::{library::{self,Library,FolderChoices},project::{Project,CatalogRef,hash_file}};
use lightcraft_catalog::{Op,Source,ColorLabel};
use std::{path::Path,sync::{Arc,atomic::AtomicBool}};

fn picture(path:&Path,color:u8){std::fs::create_dir_all(path.parent().unwrap()).unwrap();image::RgbImage::from_pixel(36,24,image::Rgb([color,80,120])).save(path).unwrap();}
#[test]
fn folder_removal_is_recursive_catalog_only_and_one_undo_transaction(){
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path().join("photos");let a=root.join("a.png");let b=root.join("nested/b.png");let other=tmp.path().join("other/c.png");
    for (p,n) in [(&a,10),(&b,20),(&other,30)]{picture(p,n);}
    let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();lib.add([&a,&b,&other].into_iter().map(|p|library::index_photo(p).unwrap()).collect()).unwrap();
    lib.apply(vec![lib.catalog.folder_label_op(root.to_str().unwrap(),Some(ColorLabel::Blue))]).unwrap();
    let ops=library::folder_remove_ops(&lib.catalog,root.to_str().unwrap()).unwrap();assert_eq!(ops.len(),2);
    let undo=lib.transact(ops).unwrap();assert_eq!(lib.catalog.photos().filter(|p|p.deleted).count(),2);assert!(a.exists()&&b.exists()&&other.exists());
    lib.apply(undo).unwrap();assert_eq!(lib.catalog.photos().filter(|p|p.deleted).count(),0);
    assert!(library::folder_remove_ops(&lib.catalog,"C:\\").is_err());
    let directory=lib.directory.clone();drop(lib);let reopened=Library::open(&directory).unwrap();assert_eq!(reopened.catalog.folder_color_label(root.to_str().unwrap()),Some(ColorLabel::Blue));
}
#[test]
fn sync_preview_matches_moved_files_and_preserves_metadata_and_virtual_copies(){
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path().join("photos");let a=root.join("a.png");let missing=root.join("missing.png");picture(&a,30);picture(&missing,50);
    let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();lib.add(vec![library::index_photo(&a).unwrap(),library::index_photo(&missing).unwrap()]).unwrap();
    let aid=lib.catalog.photos().find(|p|p.file_name=="a.png").unwrap().id;
    let missing_id=lib.catalog.photos().find(|p|p.file_name=="missing.png").unwrap().id;
    let mut allocation=lib.catalog.clone();let copy_id=allocation.alloc_photo_id();let mut copy=(**lib.catalog.photo(aid).unwrap()).clone();copy.id=copy_id;copy.copy_of=Some(aid);copy.copy_name=Some("Copy 1".into());
    lib.apply(vec![Op::AddPhoto{photo:Box::new(copy)},Op::SetRating{id:aid,rating:5}]).unwrap();
    let moved=root.join("nested/moved.png");std::fs::create_dir_all(moved.parent().unwrap()).unwrap();std::fs::rename(&a,&moved).unwrap();std::fs::remove_file(&missing).unwrap();let new=root.join("new.png");picture(&new,90);
    let plan=library::scan_folder(&lib.catalog,root.to_str().unwrap(),&lib.directory,Arc::new(AtomicBool::new(false))).unwrap();
    assert_eq!(plan.new_photos.len(),2);assert_eq!(plan.relinks.len(),2);assert_eq!(plan.missing.len(),3);
    let choices=FolderChoices{remove_missing:true,..Default::default()};let ops=plan.ops(&lib.catalog,choices).unwrap();
    let undo=lib.transact(ops).unwrap();assert_eq!(lib.catalog.photo(aid).unwrap().rating,5);assert_eq!(lib.catalog.photo(copy_id).unwrap().copy_name.as_deref(),Some("Copy 1"));
    assert!(lib.catalog.photo(missing_id).unwrap().deleted);assert_eq!(lib.catalog.photos().count(),4);
    for id in [aid,copy_id]{assert!(library::photo_path(lib.catalog.photo(id).unwrap()).unwrap().ends_with("moved.png"));}
    assert!(moved.exists()&&new.exists());assert!(!a.exists());lib.apply(undo).unwrap();assert!(!lib.catalog.photo(missing_id).unwrap().deleted);assert_eq!(lib.catalog.photos().count(),3);assert!(moved.exists());
    let fresh=library::scan_folder(&lib.catalog,root.to_str().unwrap(),&lib.directory,Arc::new(AtomicBool::new(false))).unwrap();
    lib.apply(fresh.ops(&lib.catalog,FolderChoices{relink_moved:false,..Default::default()}).unwrap()).unwrap();
    assert_eq!(lib.catalog.photos().count(),5,"unchecking relink can import the new paths separately");assert!(!lib.catalog.photo(aid).unwrap().deleted);
    assert!(library::scan_folder(&lib.catalog,root.to_str().unwrap(),&lib.directory,Arc::new(AtomicBool::new(true))).is_err());
}
#[test]
fn relink_checks_content_updates_project_and_keeps_original_xmp(){
    let tmp=tempfile::tempdir().unwrap();let old=tmp.path().join("old");let input=old.join("nested/a.png");picture(&input,70);
    let xmp=input.with_extension("xmp");std::fs::write(&xmp,b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"></x:xmpmeta>").unwrap();
    let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();lib.add(vec![library::index_photo(&input).unwrap()]).unwrap();let id=lib.catalog.photos().next().unwrap().id;
    let (mut photo,_)=luminacraft::engine::import_photo(&input).unwrap();photo.catalog_ref=Some(CatalogRef{library:lib.directory.clone(),id:id.0,copy_name:None});photo.edit.develop.light.exposure=1.25;
    let mut project=Project::default();project.photos.push(photo);project.version(0,"original edit");let before=project.photos[0].source.sha256.clone();let xmp_hash=hash_file(&xmp).unwrap();
    let destination=tmp.path().join("moved");std::fs::rename(&old,&destination).unwrap();
    let plan=library::locate_folder(&lib.catalog,old.to_str().unwrap(),&destination,&AtomicBool::new(false)).unwrap();assert_eq!(plan.relinks.len(),1);lib.apply(plan.ops(&lib.catalog,FolderChoices::default()).unwrap()).unwrap();
    assert_eq!(library::follow_catalog_paths(&mut project,&lib).unwrap(),1);assert_eq!(project.photos[0].source.sha256,before);assert_eq!(project.photos[0].edit.develop.light.exposure,1.25);assert_eq!(project.versions.len(),1);
    assert_eq!(hash_file(project.photos[0].source.xmp_path.as_ref().unwrap()).unwrap(),xmp_hash);assert!(!old.exists());
    // A stale scan and a replacement file cannot silently rewrite identity.
    let again=library::scan_folder(&lib.catalog,destination.to_str().unwrap(),&lib.directory,Arc::new(AtomicBool::new(false))).unwrap();lib.apply(vec![Op::SetRating{id,rating:3}]).unwrap();assert!(again.ops(&lib.catalog,FolderChoices::default()).is_err());
    let wrong=tmp.path().join("wrong");picture(&wrong.join("nested/a.png"),1);
    lib.apply(vec![Op::Relink{id,file_name:"a.png".into(),source:Source::File{path:input.to_string_lossy().into_owned()},format:None}]).unwrap();
    let bad=library::locate_folder(&lib.catalog,old.to_str().unwrap(),&wrong,&AtomicBool::new(false)).unwrap();assert!(bad.relinks.is_empty());assert_eq!(bad.errors.len(),1);
}
#[test]
fn scan_commit_revalidates_files_and_new_folder_names_are_scoped(){
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path().join("photos");let input=root.join("a.png");picture(&input,10);
    let mut lib=Library::open(&tmp.path().join("catalog")).unwrap();let plan=library::scan_folder(&lib.catalog,root.to_str().unwrap(),&lib.directory,Arc::new(AtomicBool::new(false))).unwrap();picture(&input,11);
    assert!(plan.ops(&lib.catalog,FolderChoices::default()).is_err());assert_eq!(lib.catalog.photos().count(),0);
    let child=library::create_child_folder(&root,"新照片").unwrap();assert_eq!(child.parent().unwrap(),root.canonicalize().unwrap());assert!(library::create_child_folder(&root,"新照片").is_err());
    for name in ["..","../outside","C:\\outside","trailing.","CON","LPT1.txt","a/b"]{assert!(library::create_child_folder(&root,name).is_err(),"{name}");}
    lib.add(vec![library::index_photo(&input).unwrap()]).unwrap();picture(&input,12);let changed=library::scan_folder(&lib.catalog,root.to_str().unwrap(),&lib.directory,Arc::new(AtomicBool::new(false))).unwrap();assert_eq!(changed.errors.len(),1);assert!(changed.ops(&lib.catalog,FolderChoices{remove_missing:true,..Default::default()}).is_err());
}
