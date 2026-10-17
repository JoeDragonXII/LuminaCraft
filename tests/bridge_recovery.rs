use luminacraft::{bridge::{self,Request,Entry,Purpose,HandoffLease},project::Project,library::{self,Library}};
use std::fs;
use serde_json::json;
fn fixture()->(tempfile::TempDir,Request,std::path::PathBuf){
    let temp=tempfile::tempdir().unwrap();let source=temp.path().join("photo.png");
    image::RgbImage::from_pixel(30,20,image::Rgb([120,80,40])).save(&source).unwrap();
    let (mut photo,_)=luminacraft::engine::import_photo(&source).unwrap();photo.edit.retouch.smooth=0.3;
    let mut project=Project::default();project.photos.push(photo);
    let library=temp.path().join("library");let mut lib=Library::open(&library).unwrap();lib.add(vec![library::index_photo(&source).unwrap()]).unwrap();
    let id=lib.catalog.photos().next().unwrap().id.0;
    let req=Request{schema:1,token:"recover-test".into(),library:library.clone(),purpose:Purpose::Develop,project:project.clone(),project_directory:None,saved_digest:None,
        entries:vec![Entry{id:Some(id),path:source,sha256:project.photos[0].source.sha256.clone(),develop:project.photos[0].edit.develop.clone()}],selected:vec![id]};
    let path=library.join("Bridge/recover-test/request.json");bridge::write_new(&path,&req).unwrap();drop(lib);(temp,req,path)
}
#[test]
fn switched_library_finds_original_recovery_and_restores_separate_browse_state(){
    let (temp,req,path)=fixture();let b=temp.path().join("other-library");drop(Library::open(&b).unwrap());
    bridge::write_new(&path.with_file_name("resolved-request.json"),&req).unwrap();
    let browse=bridge::BrowseReturn{library:b.clone(),selected:vec![1]};
    bridge::record_browse(&path,&req,&browse).unwrap();
    assert_eq!(bridge::pending(&b).unwrap(),vec![path.clone()]);
    let full=HandoffLease::participate(&path).unwrap();assert!(bridge::live_handoff(&b).unwrap());drop(full);
    let lease=HandoffLease::recover(&path).unwrap();let lib=Library::open(&req.library).unwrap();
    let (_,response,project,_)=bridge::recover_project(&path,&req.library,&lib.catalog).unwrap();
    assert_eq!(response.library,req.library);assert_eq!(response.browse.as_ref().unwrap().library,b);
    assert_eq!(response.browse.as_ref().unwrap().selected,vec![1]);assert_eq!(project.photos[0].edit.retouch.smooth,0.3);
    drop(lib);drop(lease);
    bridge::write_new(&path.with_file_name("project-saved.json"),&json!({})).unwrap();assert!(bridge::pending(&b).unwrap().is_empty());
    let foreign=bridge::Request{token:"different".into(),..req.clone()};
    assert!(bridge::latest_browse(&path,&foreign).is_err());
}
#[test]
fn a_live_participant_prevents_recovery_and_leases_release_on_drop(){
    let (_temp,req,path)=fixture();
    let main=HandoffLease::participate(&path).unwrap();let full=HandoffLease::participate(&path).unwrap();
    assert!(bridge::live_handoff(&req.library).unwrap());assert!(HandoffLease::recover(&path).is_err());assert!(main.upgrade().is_err());
    drop(full);main.upgrade().unwrap();assert!(HandoffLease::participate(&path).is_err());drop(main);
    let recovery=HandoffLease::recover(&path).unwrap();drop(recovery);assert!(!bridge::live_handoff(&req.library).unwrap());
}
#[test]
fn missing_return_file_rebuilds_saved_native_edits_and_keeps_unsaved_retouch(){
    let (_temp,req,path)=fixture();
    let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&req.library,false).unwrap();req.apply_to(&mut full).unwrap();
    full.execute("develop.set",&json!({"control":"light.exposure","value":1.5})).unwrap();full.close_library().unwrap();drop(full);
    bridge::write_new(&path.with_file_name("resolved-request.json"),&req).unwrap();
    assert!(!path.with_file_name("result.json").exists());
    let _lease=HandoffLease::recover(&path).unwrap();let lib=Library::open(&req.library).unwrap();
    let (_,response,project,_)=bridge::recover_project(&path,&req.library,&lib.catalog).unwrap();
    assert_eq!(response.photos.len(),1);assert_eq!(project.photos[0].edit.develop.light.exposure,1.5);assert_eq!(project.photos[0].edit.retouch.smooth,0.3);
    bridge::checkpoint(&path,&req,&project,None).unwrap();assert!(path.with_file_name("applied.json").exists());
    // Applied but not saved to a user project remains recoverable after a main-window crash.
    assert_eq!(bridge::pending(&req.library).unwrap(),vec![path.clone()]);
    bridge::write_new(&path.with_file_name("project-saved.json"),&json!({"saved":true})).unwrap();assert!(bridge::pending(&req.library).unwrap().is_empty());
}
#[test]
fn a_crash_before_preparation_restores_main_edits_not_older_catalog_values(){
    let (_temp,mut req,path)=fixture();req.project.photos[0].edit.develop.light.exposure=2.0;
    // Write a separate immutable request whose preparation never happened.
    let path=path.with_file_name("before-start.json");bridge::write_new(&path,&req).unwrap();
    let lib=Library::open(&req.library).unwrap();let (_,_,project,_)=bridge::recover_project(&path,&req.library,&lib.catalog).unwrap();
    assert_eq!(project.photos[0].edit.develop.light.exposure,2.0);assert_eq!(project.photos[0].edit.retouch.smooth,0.3);
}
#[test]
fn result_and_checkpoint_survive_restart_and_invalid_result_does_not_fall_back_silently(){
    let (_temp,req,path)=fixture();let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&req.library,false).unwrap();
    req.apply_to(&mut full).unwrap();full.execute("develop.set",&json!({"control":"light.exposure","value":0.8})).unwrap();
    let response=req.response(&mut full,None).unwrap();bridge::write_new(&path.with_file_name("result.json"),&response).unwrap();full.close_library().unwrap();drop(full);
    let lib=Library::open(&req.library).unwrap();let (_,_,project,_)=bridge::recover_project(&path,&req.library,&lib.catalog).unwrap();
    bridge::checkpoint(&path,&req,&project,None).unwrap();let (_,_,again,_)=bridge::recover_project(&path,&req.library,&lib.catalog).unwrap();assert_eq!(again.digest().unwrap(),project.digest().unwrap());
    fs::write(path.with_file_name("result.json"),"damaged result").unwrap();assert!(bridge::recover_project(&path,&req.library,&lib.catalog).is_err());
    assert!(path.with_file_name("project-return.json").exists());
}
