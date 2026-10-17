use luminacraft::{bridge::{self,Request,Entry,Purpose},project::{Project,CatalogRef}};
use serde_json::json;
#[test]
fn switching_libraries_returns_original_edits_and_separate_browse_identity(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("shared.png");
    image::RgbImage::from_pixel(30,20,image::Rgb([120,80,40])).save(&path).unwrap();
    let a=tmp.path().join("a");let b=tmp.path().join("b");
    let (mut photo,_)=luminacraft::engine::import_photo(&path).unwrap();photo.edit.retouch.smooth=0.3;
    let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&a,false).unwrap();
    let added=full.execute("library.import",&json!({"paths":[path],"mode":"add"})).unwrap();let id=added["imported"][0].as_u64().unwrap();
    photo.catalog_ref=Some(CatalogRef{library:a.clone(),id,copy_name:None});
    let mut foreign=photo.clone();foreign.catalog_ref.as_mut().unwrap().library=b.clone();foreign.edit.develop.light.exposure=0.4;
    let mut project=Project::default();project.photos=vec![photo,foreign];
    let request=Request{schema:1,token:"switch".into(),library:a.clone(),purpose:Purpose::Develop,project:project.clone(),project_directory:None,saved_digest:None,selected:vec![id],entries:vec![Entry{id:Some(id),path:path.clone(),sha256:project.photos[0].source.sha256.clone(),develop:project.photos[0].edit.develop.clone()}]};
    request.apply_to(&mut full).unwrap();full.execute("develop.set",&json!({"control":"light.exposure","value":1.5})).unwrap();
    full.open_library(&b,false).unwrap();let added_b=full.execute("library.import",&json!({"paths":[path],"mode":"add"})).unwrap();let b_id=added_b["imported"][0].as_u64().unwrap();
    assert_eq!(id,b_id); // Even identical source bytes and IDs belong to different libraries.
    full.execute("library.select",&json!({"ids":[b_id],"active":b_id})).unwrap();full.execute("develop.set",&json!({"control":"light.exposure","value":3.0})).unwrap();
    let response=request.response(&mut full,None).unwrap();response.validate_for(&request).unwrap();
    assert_eq!(response.library,a);assert_eq!(response.browse.as_ref().unwrap().library,b);assert_eq!(response.browse.as_ref().unwrap().selected,vec![b_id]);assert!(response.selected.is_empty());
    let merged=response.merge_existing(&request,&project).unwrap();
    assert_eq!(merged.photos[0].edit.develop.light.exposure,1.5);assert_eq!(merged.photos[0].edit.retouch.smooth,0.3);
    assert_eq!(merged.photos[1].edit.develop.light.exposure,0.4);assert_eq!(merged.photos[1].catalog_ref.as_ref().unwrap().library,b);
    assert!(response.import_queue(&merged).unwrap().is_empty());
    full.open_library(&b,false).unwrap();assert_eq!(full.catalog.photo(lightcraft_catalog::PhotoId(b_id)).unwrap().develop.light.exposure,3.0);
}
#[test]
fn library_and_import_handoffs_sync_loaded_edits_without_changing_grid_selection(){
    for purpose in [Purpose::Library,Purpose::Import] {
        let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("photo.png");
        image::RgbImage::from_pixel(30,20,image::Rgb([120,80,40])).save(&path).unwrap();
        let (mut photo,_)=luminacraft::engine::import_photo(&path).unwrap();
        photo.edit.develop.light.exposure=0.7;photo.edit.retouch.smooth=0.3;
        let dir=tmp.path().join("library");let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&dir,false).unwrap();
        let added=full.execute("library.import",&json!({"paths":[path],"mode":"add"})).unwrap();let id=added["imported"][0].as_u64().unwrap();
        photo.catalog_ref=Some(CatalogRef{library:dir.clone(),id,copy_name:None});
        let mut project=Project::default();project.photos.push(photo);
        let request=Request{schema:1,token:"library-sync".into(),library:dir,purpose,project:project.clone(),project_directory:None,saved_digest:None,selected:vec![],entries:vec![Entry{id:Some(id),path:path.clone(),sha256:project.photos[0].source.sha256.clone(),develop:project.photos[0].edit.develop.clone()}]};
        request.apply_to(&mut full).unwrap();assert!(full.selection.ids.is_empty());
        assert_eq!(full.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().develop.light.exposure,0.7);
        full.execute("library.select",&json!({"ids":[id],"active":id})).unwrap();
        full.execute("develop.set",&json!({"control":"light.exposure","value":1.5})).unwrap();
        let response=request.response(&mut full,None).unwrap();
        let merged=response.merge_existing(&request,&project).unwrap();
        assert_eq!(merged.photos.len(),1);assert_eq!(merged.photos[0].edit.develop.light.exposure,1.5);
        assert_eq!(merged.photos[0].edit.retouch.smooth,0.3);
    }
}
#[test]
fn virtual_copy_is_a_distinct_editable_project_identity_and_reopens_that_copy(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("photo.png");
    image::RgbImage::from_pixel(30,20,image::Rgb([120,80,40])).save(&path).unwrap();
    let (mut master,_)=luminacraft::engine::import_photo(&path).unwrap();master.edit.retouch.smooth=0.35;master.edit.develop.light.exposure=0.1;
    let dir=tmp.path().join("library");let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&dir,false).unwrap();
    let added=full.execute("library.import",&json!({"paths":[path],"mode":"add"})).unwrap();let master_id=added["imported"][0].as_u64().unwrap();
    master.catalog_ref=Some(CatalogRef{library:dir.clone(),id:master_id,copy_name:None});
    let mut project=Project::default();project.photos.push(master);
    let mut request=Request{schema:1,token:"versions".into(),library:dir.clone(),purpose:Purpose::Develop,project:project.clone(),project_directory:None,saved_digest:None,selected:vec![master_id],entries:vec![Entry{id:Some(master_id),path:path.clone(),sha256:project.photos[0].source.sha256.clone(),develop:project.photos[0].edit.develop.clone()}]};
    request.apply_to(&mut full).unwrap();let made=full.execute("photo.virtualCopy",&json!({})).unwrap();let copy_id=made["ids"][0].as_u64().unwrap();
    full.execute("develop.set",&json!({"control":"light.exposure","value":1.9})).unwrap();
    let response=request.response(&mut full,None).unwrap();let mut merged=response.merge_existing(&request,&project).unwrap();
    assert!((merged.photos[0].edit.develop.light.exposure-0.1).abs()<1e-9);
    let queue=response.import_queue(&merged).unwrap();assert_eq!(queue.len(),1);assert_eq!(queue[0].id,copy_id);
    let (mut copy,_)=luminacraft::engine::import_photo(&path).unwrap();bridge::bind_imported(&mut copy,&queue[0],&dir).unwrap();
    assert!(!copy.same_identity(&merged.photos[0]));assert_ne!(copy.thumbnail_key(),merged.photos[0].thumbnail_key());
    merged.photos.push(copy);assert_eq!(merged.photos.len(),2);
    let project_dir=tmp.path().join("project");merged.save(&project_dir).unwrap();let reopened=Project::open(&project_dir).unwrap();
    assert_eq!(reopened.photos[1].catalog_ref.as_ref().unwrap().id,copy_id);assert!((reopened.photos[1].edit.develop.light.exposure-1.9).abs()<1e-9);
    request.project=reopened.clone();request.entries=vec![Entry{id:Some(copy_id),path:path.clone(),sha256:reopened.photos[1].source.sha256.clone(),develop:reopened.photos[1].edit.develop.clone()}];
    // A stale grid selection of the master must not become the active advanced photo.
    request.selected=vec![master_id];request.apply_to(&mut full).unwrap();assert_eq!(full.active().unwrap().0,copy_id);
    full.execute("develop.set",&json!({"control":"light.exposure","value":2.25})).unwrap();
    let again=request.response(&mut full,None).unwrap().merge_existing(&request,&reopened).unwrap();
    assert!((again.photos[0].edit.develop.light.exposure-0.1).abs()<1e-9);assert_eq!(again.photos[1].edit.develop.light.exposure,2.25);
    // A removed copy cannot silently transfer its edits to the same-source master.
    full.commit("remove copy",lightcraft_catalog::Op::SetDeleted{id:lightcraft_catalog::PhotoId(copy_id),deleted:true}).unwrap();
    request.entries[0].develop.light.exposure=4.0;
    assert!(request.apply_to(&mut full).is_err());
    assert_eq!(full.catalog.photo(lightcraft_catalog::PhotoId(master_id)).unwrap().develop.light.exposure,0.1);
    assert_eq!(full.catalog.photo(lightcraft_catalog::PhotoId(copy_id)).unwrap().develop.light.exposure,2.25);
    // Validate the whole batch before applying an earlier valid entry.
    request.entries.insert(0,Entry{id:Some(master_id),path:path.clone(),sha256:reopened.photos[0].source.sha256.clone(),develop:request.entries[0].develop.clone()});
    request.entries[1].id=Some(u64::MAX);
    assert!(request.apply_to(&mut full).is_err());
    assert_eq!(full.catalog.photo(lightcraft_catalog::PhotoId(master_id)).unwrap().develop.light.exposure,0.1);
}
#[test]
fn legacy_photos_serialize_without_new_fields_and_more_than_fifty_can_return(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("photo.png");image::RgbImage::from_pixel(30,20,image::Rgb([100,80,50])).save(&path).unwrap();
    let (photo,_)=luminacraft::engine::import_photo(&path).unwrap();let value=serde_json::to_value(&photo).unwrap();assert!(value.get("catalog_ref").is_none());
    let legacy:luminacraft::project::Photo=serde_json::from_value(value).unwrap();assert!(legacy.catalog_ref.is_none());
    let mut project=Project::default();project.photos=vec![photo.clone();50];
    let returned=bridge::ReturnedPhoto{id:99,path,sha256:photo.source.sha256.clone(),develop:photo.edit.develop.clone(),origin:None,copy_name:Some("Copy 1".into()),cached_denoise:None};
    let response=bridge::Response{schema:1,token:"capacity".into(),project_id:project.id.clone(),library:tmp.path().join("library"),photos:vec![returned],selected:vec![99],error:None,exports:None,browse:None};
    assert_eq!(response.import_queue(&project).unwrap().len(),1);assert_eq!(response.photos.len(),1);
}
