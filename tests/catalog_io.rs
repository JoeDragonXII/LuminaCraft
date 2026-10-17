use std::{collections::HashSet, fs, path::Path, sync::atomic::AtomicBool};
use lightcraft_catalog::{Album,AlbumId,Op,PhotoId,Source};
use luminacraft::{catalog_io::{Export,MARKER},library::{Library,index_photo,photo_path},project::{Project,CatalogRef,hash_file}};

fn picture(path:&Path,n:u8){fs::create_dir_all(path.parent().unwrap()).unwrap();image::RgbImage::from_pixel(24,16,image::Rgb([n,50,90])).save(path).unwrap();}
fn export(lib:&Library,project:Option<Project>,copy_originals:bool)->Export{Export{catalog:lib.catalog.clone(),library:lib.directory.clone(),project,selected:None,copy_originals}}

#[test]
fn portable_native_directory_keeps_edits_resources_and_opens_in_full_engine(){
    let tmp=tempfile::tempdir().unwrap();let sources=tmp.path().join("sources");let path=sources.join("原片.png");picture(&path,70);
    let xmp=path.with_extension("xmp");fs::write(&xmp,"<x:xmpmeta xmlns:x='adobe:ns:meta/'/>").unwrap();
    let sha=hash_file(&path).unwrap();let before=fs::metadata(&path).unwrap().modified().unwrap();
    let dir=tmp.path().join("native");let mut lib=Library::open(&dir).unwrap();lib.add(vec![index_photo(&path).unwrap()]).unwrap();let id=lib.catalog.photos().next().unwrap().id;
    lib.apply(vec![Op::SetRating{id,rating:4},Op::AddAlbum{album:Album::new(AlbumId(1),"旅程")},Op::SetAlbumPhotos{id:AlbumId(1),photos:vec![id]}]).unwrap();
    let (mut photo,_)=luminacraft::engine::import_photo(&path).unwrap();photo.catalog_ref=Some(CatalogRef{library:dir.clone(),id:id.0,copy_name:None});photo.edit.retouch.smooth=0.3;photo.edit.develop.light.exposure=1.25;
    let resource=tmp.path().join("denoise.lcdn");fs::write(&resource,"synthetic verified denoise resource").unwrap();
    photo.cached_denoise=Some(luminacraft::resources::CachedDenoise{path:resource.clone(),key:"fixture".into(),sha256:hash_file(&resource).unwrap(),source_sha256:sha.clone()});
    let mut project=Project::default();project.photos.push(photo);project.version(0,"编辑版本");
    let ai_input=tmp.path().join("input.png");let ai_output=tmp.path().join("output.png");picture(&ai_input,60);picture(&ai_output,150);
    project.style_outputs.push(luminacraft::ai_style::StyleOutput{id:"style-one".into(),project_id:project.id.clone(),source_sha256:sha.clone(),source_path:path.canonicalize().unwrap(),catalog_ref:project.photos[0].catalog_ref.clone(),edit_sha256:luminacraft::project::photo_edit_hash(&project.photos[0]).unwrap(),template_name:"暖色".into(),prompt:"warm color".into(),service_name:"fixture".into(),model:"fixture".into(),input:ai_input.clone(),input_sha256:hash_file(&ai_input).unwrap(),output:ai_output.clone(),output_sha256:hash_file(&ai_output).unwrap()});
    let out=tmp.path().join("export");let report=export(&lib,Some(project),true).run(&out,&AtomicBool::new(false)).unwrap();assert_eq!(report.photos,1);assert!(out.join(MARKER).is_file());
    assert_eq!(hash_file(&path).unwrap(),sha);assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(),before);
    // Simulate another computer: all originals and external resources disappear.
    fs::rename(&sources,tmp.path().join("sources-offline")).unwrap();fs::remove_file(resource).unwrap();fs::remove_file(ai_input).unwrap();fs::remove_file(ai_output).unwrap();
    let moved=tmp.path().join("moved");fs::rename(&out,&moved).unwrap();
    let moved=luminacraft::catalog_io::resolve(&moved.join(MARKER)).unwrap();
    let restored=Library::open(&moved).unwrap();assert_eq!(restored.catalog.photo(id).unwrap().rating,4);assert_eq!(restored.catalog.album(AlbumId(1)).unwrap().photos,vec![id]);
    assert!(Path::new(photo_path(restored.catalog.photo(id).unwrap()).unwrap()).starts_with(&moved));drop(restored);
    let p=Project::open(&moved).unwrap();assert_eq!(p.photos[0].edit.retouch.smooth,0.3);assert_eq!(p.photos[0].edit.develop.light.exposure,1.25);assert_eq!(p.versions.len(),1);assert!(p.style_outputs[0].matches(&p.photos[0]));p.photos[0].cached_denoise.as_ref().unwrap().verify(&sha).unwrap();p.style_outputs[0].preview(false).unwrap();assert_eq!(hash_file(p.photos[0].source.xmp_path.as_ref().unwrap()).unwrap(),hash_file(&tmp.path().join("sources-offline/原片.xmp")).unwrap());
    let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&moved,false).unwrap();assert_eq!(full.catalog.len(),1);assert_eq!(full.catalog.photo(id).unwrap().develop.light.exposure,1.25);full.close_library().unwrap();drop(full);
    // Repeat open is stable and accepts the advanced engine's later persistence.
    assert_eq!(Library::open(&moved).unwrap().catalog.len(),1);
}

#[test]
fn selected_export_keeps_master_keywords_and_album_membership_without_touching_source(){
    let tmp=tempfile::tempdir().unwrap();let a=tmp.path().join("a.png");let b=tmp.path().join("b.png");picture(&a,20);picture(&b,80);
    let mut lib=Library::open(&tmp.path().join("lib")).unwrap();lib.add(vec![index_photo(&a).unwrap(),index_photo(&b).unwrap()]).unwrap();
    let master=lib.catalog.photos().find(|p|p.file_name=="a.png").unwrap().id;let mut copy=(**lib.catalog.photo(master).unwrap()).clone();copy.id=lib.catalog.alloc_photo_id();let copy_id=copy.id;copy.copy_of=Some(master);copy.copy_name=Some("Copy 1".into());copy.meta.keywords=vec!["旅行|北京".into()];
    lib.apply(vec![Op::AddPhoto{photo:Box::new(copy)},Op::AddAlbum{album:Album::new(AlbumId(1),"相册")},Op::SetAlbumPhotos{id:AlbumId(1),photos:lib.catalog.photos().map(|p|p.id).chain([copy_id]).collect()}]).unwrap();
    let before=lib.catalog.to_snapshot();let mut e=export(&lib,None,false);e.selected=Some(HashSet::from([copy_id]));let out=tmp.path().join("subset");assert_eq!(e.run(&out,&AtomicBool::new(false)).unwrap().photos,2);
    let selected=Library::open(&out).unwrap();assert_eq!(selected.catalog.photo(copy_id).unwrap().copy_of,Some(master));assert_eq!(selected.catalog.photo(copy_id).unwrap().meta.keywords,vec!["旅行|北京"]);assert_eq!(selected.catalog.album(AlbumId(1)).unwrap().photos.len(),2);assert_eq!(lib.catalog.to_snapshot(),before);assert!(a.exists()&&b.exists());
}

#[test]
fn missing_reference_is_preserved_but_copy_cancel_and_overwrite_fail_cleanly(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("a.png");picture(&path,25);let mut lib=Library::open(&tmp.path().join("lib")).unwrap();lib.add(vec![index_photo(&path).unwrap()]).unwrap();fs::remove_file(&path).unwrap();
    let reference=tmp.path().join("reference");export(&lib,None,false).run(&reference,&AtomicBool::new(false)).unwrap();assert_eq!(Library::open(&reference).unwrap().catalog.len(),1);
    let copy=tmp.path().join("copy");assert!(export(&lib,None,true).run(&copy,&AtomicBool::new(false)).is_err());assert!(!copy.exists());
    assert!(export(&lib,None,false).run(&reference,&AtomicBool::new(false)).is_err());assert!(reference.join(MARKER).exists());
    let cancelled=tmp.path().join("cancelled");assert!(export(&lib,None,false).run(&cancelled,&AtomicBool::new(true)).is_err());assert!(!cancelled.exists());
    assert!(!fs::read_dir(tmp.path()).unwrap().any(|e|e.unwrap().file_name().to_string_lossy().starts_with(".luminacraft-export-")));
}

#[test]
fn moved_catalog_rejects_corruption_and_path_traversal_before_changes(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("a.png");picture(&path,25);let mut lib=Library::open(&tmp.path().join("lib")).unwrap();lib.add(vec![index_photo(&path).unwrap()]).unwrap();let out=tmp.path().join("out");export(&lib,None,true).run(&out,&AtomicBool::new(false)).unwrap();
    let moved=tmp.path().join("moved");fs::rename(&out,&moved).unwrap();let marker=moved.join(MARKER);let mut m:serde_json::Value=serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();let snap=hash_file(&moved.join("catalog.snap")).unwrap();
    let asset=moved.join(m["assets"][0]["relative"].as_str().unwrap());fs::write(&asset,"changed").unwrap();assert!(Library::open(&moved).is_err());assert_eq!(hash_file(&moved.join("catalog.snap")).unwrap(),snap);
    m["assets"][0]["relative"]="../outside.png".into();fs::write(&marker,serde_json::to_vec(&m).unwrap()).unwrap();assert!(Library::open(&moved).is_err());assert_eq!(hash_file(&moved.join("catalog.snap")).unwrap(),snap);
}

#[test]
fn projects_over_fifty_save_open_and_handoff_without_truncation(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("a.png");picture(&path,40);let (photo,_)=luminacraft::engine::import_photo(&path).unwrap();let mut project=Project::default();
    for n in 0..128 {let mut p=photo.clone();p.catalog_ref=Some(CatalogRef{library:tmp.path().join("lib"),id:n+1,copy_name:Some(format!("Copy {n}"))});p.edit.develop.light.exposure=n as f64/128.0;project.photos.push(p);}
    let saved=tmp.path().join("project");project.save(&saved).unwrap();let p=Project::open(&saved).unwrap();assert_eq!(p.photos.len(),128);
    let req=luminacraft::bridge::Request{schema:1,token:"large".into(),library:tmp.path().join("lib"),purpose:luminacraft::bridge::Purpose::Develop,project:p.clone(),project_directory:Some(saved),saved_digest:None,entries:(1..=128).map(|id|luminacraft::bridge::Entry{id:Some(id),path:photo.source.path.clone(),sha256:photo.source.sha256.clone(),develop:p.photos[(id-1) as usize].edit.develop.clone()}).collect(),selected:(1..=128).collect()};req.validate().unwrap();
    let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&req.library,false).unwrap();for id in 1..=128 {let mut p=index_photo(&path).unwrap();p.id=PhotoId(id);p.source=Source::File{path:photo.source.path.to_string_lossy().into_owned()};full.catalog.apply(Op::AddPhoto{photo:Box::new(p)}).unwrap();}
    req.apply_to(&mut full).unwrap();full.selection.ids=(1..=128).map(PhotoId).collect();let response=req.response(&mut full,None).unwrap();assert_eq!(response.photos.len(),128);response.validate_for(&req).unwrap();let merged=response.merge_existing(&req,&req.project).unwrap();assert_eq!(merged.photos.len(),128);assert_eq!(merged.photos[127].edit.develop.light.exposure,127.0/128.0);
}

#[test]
fn native_main_and_advanced_saves_keep_develop_but_preserve_main_retouch(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("a.png");picture(&path,30);let mut lib=Library::open(&tmp.path().join("lib")).unwrap();lib.add(vec![index_photo(&path).unwrap()]).unwrap();let id=lib.catalog.photos().next().unwrap().id;
    let (mut p,_)=luminacraft::engine::import_photo(&path).unwrap();p.catalog_ref=Some(CatalogRef{library:lib.directory.clone(),id:id.0,copy_name:None});p.edit.retouch.smooth=0.4;let mut project=Project::default();project.photos.push(p);
    let out=tmp.path().join("out");export(&lib,Some(project),false).run(&out,&AtomicBool::new(false)).unwrap();let mut native=Library::open(&out).unwrap();let mut p=luminacraft::catalog_io::load_project(&out,&native).unwrap();p.photos[0].edit.develop.light.exposure=1.75;
    luminacraft::catalog_io::save_project_develop(&p,&mut native).unwrap();p.save(&out).unwrap();drop(native);
    let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&out,false).unwrap();assert_eq!(full.catalog.photo(id).unwrap().develop.light.exposure,1.75);full.execute("library.select",&serde_json::json!({"ids":[id.0],"active":id.0})).unwrap();full.execute("develop.set",&serde_json::json!({"control":"light.exposure","value":2.25})).unwrap();full.close_library().unwrap();drop(full);
    let native=Library::open(&out).unwrap();let p=luminacraft::catalog_io::load_project(&out,&native).unwrap();assert_eq!(p.photos[0].edit.develop.light.exposure,2.25);assert_eq!(p.photos[0].edit.retouch.smooth,0.4);
}

#[test]
fn creative_lut_survives_export_and_directory_relocation(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("a.png");picture(&path,40);let mut lib=Library::open(&tmp.path().join("lib")).unwrap();let mut native=index_photo(&path).unwrap();
    native.history.push(lightcraft_catalog::HistoryStep{label:"原有历史".into(),settings:native.develop.clone()});native.versions.push(lightcraft_catalog::Version{name:"原有版本".into(),created:"2026-10-15".into(),settings:native.develop.clone(),auto:false});lib.add(vec![native]).unwrap();let id=lib.catalog.photos().next().unwrap().id;
    let cube=tmp.path().join("test.cube");fs::write(&cube,"LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n").unwrap();fs::write(lib.directory.join("prefs.json"),serde_json::to_vec(&serde_json::json!({"lutProfiles":[{"id":"lut:portable-fixture","name":"fixture","group":"test","file":cube}]})).unwrap()).unwrap();
    let (mut p,_)=luminacraft::engine::import_photo(&path).unwrap();p.catalog_ref=Some(CatalogRef{library:lib.directory.clone(),id:id.0,copy_name:None});p.edit.develop.profile.id="lut:portable-fixture".into();let mut project=Project::default();project.photos.push(p);
    let out=tmp.path().join("out");export(&lib,Some(project),true).run(&out,&AtomicBool::new(false)).unwrap();let moved=tmp.path().join("moved");fs::rename(out,&moved).unwrap();fs::remove_file(cube).unwrap();
    let native=Library::open(&moved).unwrap();assert_eq!(native.catalog.photo(id).unwrap().history.len(),1);assert_eq!(native.catalog.photo(id).unwrap().versions.len(),1);let p=Project::open(&moved).unwrap();luminacraft::engine::prepare_profile(&p.photos[0],&p.photos[0].edit).unwrap();
    let source=luminacraft::engine::Source::load_photo(&p.photos[0],&p.photos[0].edit.develop,64).unwrap();let rendered=luminacraft::engine::render_photo(&source,&p.photos[0].edit,64,&AtomicBool::new(false)).unwrap();assert!(rendered.image.width>0);
}

#[test]
fn whole_catalog_exports_verified_denoise_even_without_a_loaded_main_project(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("a.png");picture(&path,60);let hash=hash_file(&path).unwrap();let mut lib=Library::open(&tmp.path().join("lib")).unwrap();lib.add(vec![index_photo(&path).unwrap()]).unwrap();let id=lib.catalog.photos().next().unwrap().id;
    let product=tmp.path().join("existing.lcdn");fs::write(&product,"synthetic decoder product").unwrap();let cached=luminacraft::resources::CachedDenoise{path:product,sha256:String::new(),source_sha256:hash.clone(),key:"fixture".into()};let cached=luminacraft::resources::CachedDenoise{sha256:hash_file(&cached.path).unwrap(),..cached};
    let index=lib.directory.join("BridgeResources/DenoiseIndex").join(id.0.to_string());fs::create_dir_all(&index).unwrap();fs::write(index.join("001.json"),serde_json::to_vec(&cached).unwrap()).unwrap();
    let out=tmp.path().join("out");export(&lib,None,true).run(&out,&AtomicBool::new(false)).unwrap();let moved=tmp.path().join("moved");fs::rename(out,&moved).unwrap();let restored=Library::open(&moved).unwrap();let cached=luminacraft::resources::latest(&restored.directory,id.0,&hash).unwrap().unwrap();cached.verify(&hash).unwrap();assert!(cached.path.starts_with(&moved));
}
