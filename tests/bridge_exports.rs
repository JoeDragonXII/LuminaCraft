use luminacraft::{bridge,project::Project};
use serde_json::json;
#[test]
fn multiple_completed_batches_remain_separate_verified_standalone_outputs(){
    let temp=tempfile::tempdir().unwrap();let request=temp.path().join("request.json");let dir=temp.path().join("Exports");std::fs::create_dir(&dir).unwrap();
    let mut files=vec![];
    for n in 0..2{let path=temp.path().join(format!("export-{n}.jpg"));image::RgbImage::from_pixel(20,20,image::Rgb([n*40+80,100,120])).save(&path).unwrap();let hash=luminacraft::project::hash_file(&path).unwrap();
        let batch=json!({"files":[{"path":path,"sha256":hash}]});bridge::write_new(&dir.join(format!("{n:05}.json")),&batch).unwrap();files.push(path);
    }
    let batches=bridge::export_batches(&request).unwrap();let mut project=Project::default();bridge::merge_export_receipts(&mut project,&batches).unwrap();
    assert_eq!(project.advanced_outputs.len(),2);assert!(project.advanced_outputs.iter().all(|r|r.origin.as_deref()==Some("advanced-export") && r.edit_sha256.is_empty()));
    bridge::merge_export_receipts(&mut project,&batches).unwrap();assert_eq!(project.advanced_outputs.len(),2);
    let target=temp.path().join("project");project.save(&target).unwrap();let reopened=Project::open(&target).unwrap();assert_eq!(reopened.advanced_outputs.len(),2);
    let before=project.digest().unwrap();std::fs::write(&files[0],"changed file").unwrap();bridge::merge_export_receipts(&mut project,&batches).unwrap();assert_eq!(project.digest().unwrap(),before);
    assert_ne!(luminacraft::project::hash_file(&files[0]).unwrap(),project.advanced_outputs[0].output_sha256);
    std::fs::remove_file(&files[0]).unwrap();bridge::merge_export_receipts(&mut project,&batches).unwrap();assert_eq!(project.advanced_outputs.len(),2);
}
#[test]
fn failed_and_skipped_files_are_not_publishable_receipts(){
    let value=json!({"files":[{"error":"decode failed","file":"missing.jpg"},{"skipped":"existing.jpg"}]});assert!(bridge::export_receipts(&value).unwrap().is_empty());
}
#[test]
fn two_synchronous_exports_before_a_host_frame_keep_both_completion_events(){
    let temp=tempfile::tempdir().unwrap();let source=temp.path().join("input.png");
    image::RgbImage::from_pixel(32,24,image::Rgb([120,80,40])).save(&source).unwrap();
    let mut session=lightcraft_engine::Session::new().with_fs();session.open_library(temp.path().join("library"),false).unwrap();
    let added=session.execute("library.import",&serde_json::json!({"paths":[source],"mode":"add"})).unwrap();let id=added["imported"][0].as_u64().unwrap();
    session.execute("library.select",&serde_json::json!({"ids":[id],"active":id})).unwrap();
    let mut services=lightcraft_ui_egui::Services::default();
    services.write=Some(Box::new(|path,bytes|{use std::io::Write;let mut f=std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e|e.to_string())?;f.write_all(bytes).map_err(|e|e.to_string())}));
    let mut app=lightcraft_ui_egui::LightcraftApp::new(session,services);app.export_events=Some(Default::default());
    for name in ["first.jpg","second.jpg"]{
        let path=temp.path().join(name);
        lightcraft_ui_egui::control::export_active(&mut app,&serde_json::json!({"path":path,"format":"jpeg","background":false})).unwrap();
        assert_eq!(image::image_dimensions(&path).unwrap(),(32,24));
    }
    let events=app.export_events.as_ref().unwrap();assert_eq!(events.len(),2);
    assert_ne!(events[0]["files"][0]["path"],events[1]["files"][0]["path"]);
    assert_eq!(app.last_export_result.as_ref().unwrap(),&events[1]);
}
