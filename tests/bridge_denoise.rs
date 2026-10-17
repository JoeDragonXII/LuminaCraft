//! Synthetic cache-format and decoder tests; no model inference or AI-quality claim.
use luminacraft::{bridge::{Request,Entry,Purpose},engine::Source,project::Project};
use lightcraft_raw::*;
use serde_json::json;
use std::sync::{Arc,atomic::AtomicBool};

#[test]
fn small_plain_exports_and_crops_are_not_enlarged_to_the_render_limit(){
    let temp=tempfile::tempdir().unwrap();let input=temp.path().join("small.png");
    image::RgbImage::from_pixel(40,20,image::Rgb([120,80,40])).save(&input).unwrap();
    let (mut photo,_)=luminacraft::engine::import_photo(&input).unwrap();
    for (name,expected) in [("whole.jpg",(40,20)),("crop.jpg",(20,10))]{
        let output=temp.path().join(name);
        luminacraft::engine::export_photo(&photo,&output,false,false,&AtomicBool::new(false)).unwrap();
        assert_eq!(image::image_dimensions(output).unwrap(),expected);
        photo.edit.develop.crop.geometry.rect=lightcraft_geom::Rect::new(0.25,0.25,0.75,0.75);
    }
}

#[test]
fn raw_cached_product_roundtrips_blends_and_survives_disposable_cache_removal(){
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("synthetic.dng");
    let (w,h)=(32usize,24usize);
    let raw=RawImage{format:RawFormat::Dng,width:w,height:h,cpp:1,
        data:RawData::U16((0..w*h).map(|i|256+((i%w)*300+(i/w)*200)as u16).collect()),
        cfa:Some(Cfa::bayer("RGGB").unwrap()),bits:14,black:BlackLevel::uniform(256.0),white:vec![16000.0],
        active_area:Rect::new(0,0,w,h),crop:Rect::new(0,0,w,h),orientation:Orientation::Normal,
        color:ColorData{illuminant:[17,21],color_matrix:[Some(Mat3([[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]]));2],as_shot_neutral:Some([0.95047,1.0,1.08883]),..Default::default()},
        wb_multipliers:None,linearized:false,opcodes:OpcodeLists::default(),metadata:Default::default()};
    let bytes=write_dng(&raw,&DngWriteOptions::default()).unwrap();std::fs::write(&path,&bytes).unwrap();
    let mut cached=decode(&bytes).unwrap().develop(Method::Ahd).unwrap();
    // Deliberately darkened synthetic camera RGB makes loss of the product observable.
    for pixel in &mut cached.data{for channel in pixel{*channel*=0.4;}}
    let disposable=temp.path().join("model-cache.lcdn");let key="synthetic-resource-contract";
    lightcraft_denoise_core::product::write(&disposable,&cached,key).unwrap();
    let (mut photo,_)=luminacraft::engine::import_photo(&path).unwrap();assert!(photo.source.sensor);
    photo.edit.develop.enhance.denoise=60.0;photo.edit.develop.enhance.denoise_on=Some(true);
    assert!(Source::load_photo(&photo,&photo.edit.develop,64).is_err());
    let mut project=Project::default();project.photos.push(photo);
    let library=temp.path().join("library");let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&library,false).unwrap();
    let added=full.execute("library.import",&json!({"paths":[path],"mode":"add"})).unwrap();let id=added["imported"][0].as_u64().unwrap();
    let request=Request{schema:1,token:"denoise-resource".into(),library:library.clone(),purpose:Purpose::Develop,project:project.clone(),project_directory:None,saved_digest:None,selected:vec![id],entries:vec![Entry{id:Some(id),path:path.clone(),sha256:project.photos[0].source.sha256.clone(),develop:project.photos[0].edit.develop.clone()}]};
    request.apply_to(&mut full).unwrap();full.media.denoise.set(lightcraft_catalog::PhotoId(id),lightcraft_engine::media::DenoiseSpec{product:disposable.clone(),key:key.into(),make:None});
    let response=request.response(&mut full,None).unwrap();let mut merged=response.merge_existing(&request,&project).unwrap();
    let durable=merged.photos[0].cached_denoise.clone().unwrap();assert_ne!(durable.path,disposable);
    std::fs::remove_file(&disposable).unwrap();durable.verify(&merged.photos[0].source.sha256).unwrap();
    let spec=lightcraft_engine::media::DenoiseSpec{product:durable.path.clone(),key:key.into(),make:None};
    let (plain,twin,info)=lightcraft_engine::files::load_vec_pair(bytes.clone(),64,&spec).unwrap();
    let expected=lightcraft_engine::media::DecodedSource{image:Arc::new(plain),info:Some(info),camera_tone:None,denoised:Some(Arc::new(lightcraft_engine::media::Twin::new(Arc::new(twin.unwrap()))))};
    for amount in [0.0,60.0,100.0]{
        let mut settings=merged.photos[0].edit.develop.clone();settings.enhance.denoise=amount;
        let source=Source::load_photo(&merged.photos[0],&settings,64).unwrap();
        assert_eq!(source.pixels.data,expected.image_for((amount/100.0)as f32).data);
    }
    let photo=merged.photos[0].clone();let (tx,rx)=luminacraft::worker::start(egui::Context::default());
    let mut changed_resource=photo.clone();changed_resource.cached_denoise.as_mut().unwrap().key="another-model-product".into();
    assert_ne!(luminacraft::project::photo_edit_hash(&photo).unwrap(),luminacraft::project::photo_edit_hash(&changed_resource).unwrap());
    let mut images=Vec::new();
    for (id,before) in [(1,false),(2,true)]{
        tx.send(luminacraft::worker::Request{id,job:luminacraft::worker::Job::Preview(photo.clone(),before,64),cancel:Arc::new(AtomicBool::new(false))}).unwrap();
        match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap().result.unwrap(){luminacraft::worker::Outcome::Preview(out)=>images.push(out.image),_=>panic!("expected preview")}
    }
    assert_ne!(images[0],images[1],"before view must reload the plain source");
    let output=temp.path().join("export.jpg");luminacraft::engine::export_photo(&photo,&output,false,false,&AtomicBool::new(false)).unwrap();assert_eq!(image::image_dimensions(output).unwrap(),(32,24));
    let mut cropped=photo.clone();cropped.edit.develop.crop.geometry.rect=lightcraft_geom::Rect::new(0.25,0.25,0.75,0.75);
    let output=temp.path().join("cropped.jpg");luminacraft::engine::export_photo(&cropped,&output,false,false,&AtomicBool::new(false)).unwrap();assert_eq!(image::image_dimensions(output).unwrap(),(16,12));
    let directory=temp.path().join("project");merged.save(&directory).unwrap();let reopened=Project::open(&directory).unwrap();assert_eq!(reopened.photos[0].cached_denoise.as_ref(),Some(&durable));
    full.close_library().unwrap();drop(full);
    let lib=luminacraft::library::Library::open(&library).unwrap();let recovered=luminacraft::bridge::reconstruct(&request,&lib.catalog).unwrap();assert_eq!(recovered.photos[0].cached_denoise.as_ref(),Some(&durable));
    drop(lib);
    // Reopen the full engine with only the durable project asset, no model or disposable cache.
    let mut advanced=lightcraft_engine::Session::new().with_fs();advanced.open_library(&library,false).unwrap();
    let mut reopen_request=request.clone();reopen_request.project=reopened;
    reopen_request.apply_to(&mut advanced).unwrap();
    for _ in 0..3{advanced.execute("denoise.pump",&json!({})).unwrap();}
    advanced.execute("denoise.clear",&json!({})).unwrap();advanced.execute("denoise.pump",&json!({})).unwrap();
    assert!(advanced.media.denoise.handoff(lightcraft_catalog::PhotoId(id)).is_some(),"clearing disposable cache must preserve the project asset");
    assert!(matches!(advanced.denoise_photo_state(lightcraft_catalog::PhotoId(id)),lightcraft_engine::denoise::PhotoState::Ready));
    let record=advanced.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().clone();
    let source_ref=advanced.media.source_ref_uncached(&record,lightcraft_engine::media::SourceLevel::Full);
    let loaded=source_ref.load_source().unwrap();assert!(loaded.denoised.is_some());
    // Compare full-resolution AHD on both sides. The <=600 preview loader uses
    // bilinear demosaicing, so its ordinary contribution intentionally differs.
    let main_full=Source::load_photo(&photo,&photo.edit.develop,20000).unwrap();
    let worst=loaded.image_for(0.6).data.iter().zip(&main_full.pixels.data).flat_map(|(a,b)|a.iter().zip(b).map(|(a,b)|(a-b).abs())).fold(0.0f32,f32::max);
    assert!(worst<1e-6,"main/full-engine native blend differed: {worst}");
    let advanced_export=lightcraft_engine::export::export_photo(&mut advanced,lightcraft_catalog::PhotoId(id),&Default::default(),1).unwrap();
    assert_eq!((image::load_from_memory(&advanced_export.bytes).unwrap().width(),image::load_from_memory(&advanced_export.bytes).unwrap().height()),(32,24));
    // Switching libraries restores resources only for their original library and
    // preserves full-window edits rather than replaying the startup Request.
    advanced.execute("develop.set",&json!({"control":"light.exposure","value":1.25})).unwrap();advanced.persist().unwrap();
    let other=temp.path().join("other-library");advanced.open_library(&other,false).unwrap();
    let other_added=advanced.execute("library.import",&json!({"paths":[path],"mode":"add"})).unwrap();
    assert_eq!(other_added["imported"][0].as_u64(),Some(id));
    assert!(advanced.media.denoise.handoff(lightcraft_catalog::PhotoId(id)).is_none(),"same id and source in B must not inherit A's product");
    advanced.open_library(&library,false).unwrap();
    assert!(advanced.media.denoise.handoff(lightcraft_catalog::PhotoId(id)).is_some());
    assert_eq!(advanced.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().develop.light.exposure,1.25);
    advanced.execute("denoise.pump",&json!({})).unwrap();
    assert!(lightcraft_engine::export::export_photo(&mut advanced,lightcraft_catalog::PhotoId(id),&Default::default(),1).is_ok());
    // An explicit release (new model/requeue) must survive another round trip.
    assert!(advanced.media.denoise.release_handoff(lightcraft_catalog::PhotoId(id)));
    advanced.open_library(&other,false).unwrap();advanced.open_library(&library,false).unwrap();
    assert!(advanced.media.denoise.handoff(lightcraft_catalog::PhotoId(id)).is_none());
    advanced.media.denoise.set_handoff(lightcraft_catalog::PhotoId(id),durable.spec(path.clone()));advanced.media.forget(lightcraft_catalog::PhotoId(id));
    let mut wrong=photo.clone();wrong.cached_denoise.as_mut().unwrap().key="wrong-key".into();assert!(Source::load_photo(&wrong,&wrong.edit.develop,64).is_err());
    let guard=durable.spec(path.clone());let mut changed=bytes.clone();changed.push(0);std::fs::write(&path,changed).unwrap();
    assert!(guard.make.as_ref().unwrap()().is_err(),"a replaced original cannot reuse an old product");std::fs::write(&path,&bytes).unwrap();
    std::fs::write(&durable.path,b"damaged private test cache").unwrap();assert!(Source::load_photo(&photo,&photo.edit.develop,64).is_err());
    advanced.media.forget(lightcraft_catalog::PhotoId(id));
    assert!(lightcraft_engine::export::export_photo(&mut advanced,lightcraft_catalog::PhotoId(id),&Default::default(),1).is_err());
    advanced.open_library(temp.path().join("other-library"),false).unwrap();assert!(advanced.media.denoise.handoff(lightcraft_catalog::PhotoId(id)).is_none());
    assert_eq!(luminacraft::project::hash_file(&path).unwrap(),photo.source.sha256);
}
