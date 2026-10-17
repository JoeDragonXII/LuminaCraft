use luminacraft::{bridge::{Request,Entry,Purpose},library::{self,Library},project::Project};
use std::fs;
use serde_json::json;

#[test]
fn shared_catalog_handoff_returns_edits_keeps_retouch_and_releases_each_lock() {
    let tmp=tempfile::tempdir().unwrap();let source=tmp.path().join("photo.png");
    image::RgbImage::from_pixel(30,20,image::Rgb([120,80,40])).save(&source).unwrap();
    let (mut photo,_)=luminacraft::engine::import_photo(&source).unwrap();
    photo.edit.develop.light.exposure=0.5;photo.edit.retouch.smooth=0.3;
    let mut project=Project::default();project.photos.push(photo);
    let dir=tmp.path().join("library");let mut lib=Library::open(&dir).unwrap();
    lib.add(vec![library::index_photo(&source).unwrap()]).unwrap();
    let id=lib.catalog.photos().next().unwrap().id.0;
    let request=Request{schema:1,token:"test-session".into(),library:dir.clone(),purpose:Purpose::Develop,
        entries:vec![Entry{id:Some(id),path:source.clone(),sha256:project.photos[0].source.sha256.clone(),develop:project.photos[0].edit.develop.clone()}],
        selected:vec![id],project:project.clone(),project_directory:None,saved_digest:None};
    let request_path=tmp.path().join("bridge/request.json");luminacraft::bridge::write_new(&request_path,&request).unwrap();
    assert!(luminacraft::bridge::write_new(&request_path,&request).is_err());
    assert!(Library::open(&dir).is_err());drop(lib);
    let mut advanced=lightcraft_engine::Session::new().with_fs();advanced.open_library(&dir,false).unwrap();
    request.apply_to(&mut advanced).unwrap();
    assert_eq!(advanced.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().develop.light.exposure,0.5);
    advanced.execute("develop.set",&json!({"control":"light.exposure","value":1.25})).unwrap();
    let response=request.response(&mut advanced,None).unwrap();
    let merged=response.merge_existing(&request,&project).unwrap();
    assert_eq!(merged.photos[0].edit.develop.light.exposure,1.25);
    assert_eq!(merged.photos[0].edit.retouch.smooth,0.3);
    assert_eq!(project.photos[0].edit.develop.light.exposure,0.5);
    assert!(Library::open(&dir).is_err());advanced.close_library().unwrap();drop(advanced);
    let reopened=Library::open(&dir).unwrap();assert_eq!(reopened.catalog.len(),1);
    assert_eq!(reopened.catalog.photos().next().unwrap().develop.light.exposure,1.25);
    assert_eq!(luminacraft::project::hash_file(&source).unwrap(),project.photos[0].source.sha256);
}

#[test]
fn stale_or_changed_return_is_rejected_without_partial_project_updates() {
    let tmp=tempfile::tempdir().unwrap();let source=tmp.path().join("photo.png");
    image::RgbImage::from_pixel(30,20,image::Rgb([120,80,40])).save(&source).unwrap();
    let (photo,_)=luminacraft::engine::import_photo(&source).unwrap();let mut project=Project::default();project.photos.push(photo);
    let request=Request{schema:1,token:"valid".into(),library:tmp.path().join("library"),purpose:Purpose::Develop,
        project:project.clone(),project_directory:None,saved_digest:None,selected:vec![],entries:vec![]};
    let mut response=luminacraft::bridge::Response{schema:1,token:"other".into(),project_id:project.id.clone(),library:request.library.clone(),photos:vec![],selected:vec![],error:None,exports:None,browse:None};
    assert!(response.merge_existing(&request,&project).is_err());response.token="valid".into();
    response.photos.push(luminacraft::bridge::ReturnedPhoto{id:1,path:source.clone(),sha256:project.photos[0].source.sha256.clone(),develop:project.photos[0].edit.develop.clone(),origin:None,copy_name:None,cached_denoise:None});
    let before=project.digest().unwrap();fs::write(source,"changed source").unwrap();
    assert!(response.merge_existing(&request,&project).is_err());assert_eq!(project.digest().unwrap(),before);
}

#[test]
fn importing_many_photos_returns_to_the_library_without_a_fifty_photo_limit() {
    use lightcraft_catalog::{Photo,PhotoId,Source,Op};
    let tmp=tempfile::tempdir().unwrap();let dir=tmp.path().join("library");
    let mut session=lightcraft_engine::Session::new();session.open_library(&dir,false).unwrap();
    let mut ids=vec![];let mut ops=vec![];
    for n in 1..=80 {
        let id=PhotoId(n);ids.push(id);
        ops.push(Op::AddPhoto{photo:Box::new(Photo::new(id,Source::File{path:tmp.path().join(format!("{n}.jpg")).to_string_lossy().into()},&format!("{n}.jpg"),"JPEG",30,20,"2026-10-14"))});
    }
    session.commit("temporary catalog fixture",Op::Batch{ops}).unwrap();
    session.selection=lightcraft_engine::Selection{active:ids.first().copied(),ids};
    let request=Request{schema:1,token:"library-return".into(),library:dir,purpose:Purpose::Import,project:Project::default(),project_directory:None,saved_digest:None,entries:vec![],selected:vec![]};
    let response=request.response(&mut session,None).unwrap();
    assert!(response.photos.is_empty());assert_eq!(response.selected.len(),80);assert_eq!(session.catalog.len(),80);
}


