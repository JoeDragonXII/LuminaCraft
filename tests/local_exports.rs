//! Temporary fixtures; no original-photo mutation, production API or real AI model.
use luminacraft::{engine,project::{Project,PublishRecord,CatalogRef,hash_file,photo_edit_hash},worker::{self,Job,Request},task_gate::CommitGate};
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},time::Duration};
fn photo(root:&std::path::Path)->luminacraft::project::Photo{let p=root.join("source.png");image::RgbImage::from_pixel(48,32,image::Rgb([90,70,50])).save(&p).unwrap();engine::import_photo(&p).unwrap().0}
fn receipt(photo:&luminacraft::project::Photo,path:&std::path::Path,hash:String)->PublishRecord{PublishRecord{lumina_base:None,source_sha256:photo.source.sha256.clone(),edit_sha256:hash,output:path.into(),output_sha256:hash_file(path).unwrap(),asset_id:None,collection_id:None,origin:None}}
#[test]fn cancelling_project_before_commit_preserves_the_saved_file(){
    let tmp=tempfile::tempdir().unwrap();let mut p=Project::default();p.photos.push(photo(tmp.path()));let dir=tmp.path().join("project");p.save(&dir).unwrap();let old=std::fs::read(dir.join("project.json")).unwrap();p.photos[0].edit.develop.light.exposure=1.0;
    let gate=CommitGate::default();assert!(gate.request_cancel());assert!(p.save_with_commit(&dir,&||gate.begin_commit()).is_err());assert_eq!(std::fs::read(dir.join("project.json")).unwrap(),old);assert_eq!(std::fs::read_dir(&dir).unwrap().count(),1);
}
#[test]fn commit_wins_over_late_cancellation_and_keeps_the_complete_output(){
    let tmp=tempfile::tempdir().unwrap();let p=photo(tmp.path());let out=tmp.path().join("result.jpg");let gate=CommitGate::default();let flag=AtomicBool::new(false);
    let report=engine::export_photo_with_commit(&p,&out,false,false,&flag,&||{gate.begin_commit()?;assert!(!gate.request_cancel());flag.store(true,Ordering::Relaxed);Ok(())}).unwrap();
    assert_eq!(image::image_dimensions(&out).unwrap(),(48,32));assert_eq!(report["edit_sha256"],photo_edit_hash(&p).unwrap());
    let bytes=std::fs::read(&out).unwrap();assert!(engine::export_photo(&p,&out,false,false,&AtomicBool::new(false)).is_err());assert_eq!(std::fs::read(&out).unwrap(),bytes);
}
#[test]fn cancelled_queued_worker_job_acknowledges_without_creating_files(){
    let tmp=tempfile::tempdir().unwrap();let mut p=Project::default();p.photos.push(photo(tmp.path()));let dest=tmp.path().join("cancelled");let gate=Arc::new(CommitGate::default());gate.request_cancel();let(tx,rx)=worker::start(eframe::egui::Context::default());
    tx.send(Request{id:17,job:Job::Managed(Box::new(Job::Save(p,dest.clone())),gate),cancel:Arc::new(AtomicBool::new(true))}).unwrap();let result=rx.recv_timeout(Duration::from_secs(5)).unwrap();assert_eq!(result.id,17);assert!(result.result.is_err());assert!(!dest.join("project.json").exists());
}
#[test]fn sidecar_metadata_is_exported_and_location_is_removed_after_merging(){
    let tmp=tempfile::tempdir().unwrap();let initial=photo(tmp.path());let xmp=initial.source.path.with_extension("xmp");let meta=lightcraft_meta::Metadata{title:Some("Fixture title".into()),artist:Some("Fixture author".into()),keywords:vec!["fixture-keyword".into()],city:Some("Fixture city".into()),sublocation:Some("Fixture place".into()),gps:Some(lightcraft_meta::Gps{latitude:22.5,longitude:113.5,altitude:Some(10.0)}),..Default::default()};std::fs::write(&xmp,lightcraft_meta::write_xmp(&meta,None)).unwrap();let p=engine::import_photo(&initial.source.path).unwrap().0;let source_hash=hash_file(&p.source.path).unwrap();let xmp_hash=hash_file(&xmp).unwrap();
    for(keep,ext,tiff)in [(false,"jpg",false),(true,"jpg",false),(false,"tif",true),(true,"tif",true)]{let out=tmp.path().join(format!("result-{keep}.{ext}"));engine::export_photo(&p,&out,tiff,keep,&AtomicBool::new(false)).unwrap();let exported=lightcraft_meta::extract(&std::fs::read(out).unwrap());assert_eq!(exported.title,meta.title);assert_eq!(exported.artist,meta.artist);assert_eq!(exported.keywords,meta.keywords);assert_eq!(exported.width,Some(48));assert_eq!(exported.height,Some(32));if keep{assert_eq!(exported.gps,meta.gps);assert_eq!(exported.city,meta.city);}else{assert!(exported.gps.is_none()&&exported.city.is_none()&&exported.sublocation.is_none());}}
    assert_eq!(hash_file(&p.source.path).unwrap(),source_hash);assert_eq!(hash_file(&xmp).unwrap(),xmp_hash);
}
#[test]fn upstream_encoder_preserves_retouch_full_resolution_depth_and_icc(){
    use image::ImageDecoder;
    use lightcraft_geom::Point;
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("portrait-fixture.png");
    image::RgbImage::from_fn(48,32,|x,y|{let v=if (x+y)%2==0{12}else{0};image::Rgb([120+v,85+v,65+v])}).save(&path).unwrap();
    let(mut p,source)=engine::import_photo(&path).unwrap();let cancel=AtomicBool::new(false);
    let plain=engine::render_photo(&source,&p.edit,20000,&cancel).unwrap();
    p.edit.retouch.smooth=1.0;p.edit.retouch.mask.push(luminacraft::project::MaskDab{center:Point::new(0.5,0.5),radius:0.2,erase:false});
    let edited=engine::render_photo(&source,&p.edit,20000,&cancel).unwrap();assert_ne!(plain.image.data,edited.image.data,"fixture must exercise retouch");
    for(tiff,ext,color)in [(false,"jpg",image::ColorType::Rgb8),(true,"tif",image::ColorType::Rgb16)]{
        let out=tmp.path().join(format!("edited.{ext}"));engine::export_photo(&p,&out,tiff,false,&cancel).unwrap();
        // Inspect metadata on the direct decoder before ImageReader's
        // limits/decoding setup, which interferes with TIFF tag retrieval.
        let reader=std::io::BufReader::new(std::fs::File::open(&out).unwrap());
        let mut decoder:Box<dyn ImageDecoder>=if tiff{Box::new(image::codecs::tiff::TiffDecoder::new(reader).unwrap())}else{Box::new(image::codecs::jpeg::JpegDecoder::new(reader).unwrap())};
        assert_eq!(decoder.dimensions(),(48,32));assert_eq!(decoder.color_type(),color);let icc=decoder.icc_profile().unwrap();assert!(icc.as_ref().is_some_and(|icc|icc.len()>128),"{ext} ICC missing or invalid: {:?}",icc.as_ref().map(Vec::len));
        if tiff{let lightcraft_pipeline::DeepSamples::U16(expected)=&edited.deep.samples else{panic!("16-bit render required")};assert_eq!(image::open(&out).unwrap().to_rgb16().as_raw(),expected);}
        else{let jpg=image::open(&out).unwrap().to_rgb8();let error=|reference:&lightcraft_raster::Rgba8|jpg.pixels().zip(&reference.data).flat_map(|(a,b)|(0..3).map(move|c|a[c].abs_diff(b[c])as u64)).sum::<u64>();assert!(error(&edited.image)<error(&plain.image),"JPEG must retain retouch");}
    }
}
#[test]fn upstream_exif_and_xmp_never_restore_filtered_serial_numbers(){
    let tmp=tempfile::tempdir().unwrap();let initial=photo(tmp.path());
    let metadata=lightcraft_meta::Metadata{serial_number:Some("private-body-serial".into()),lens_serial_number:Some("private-lens-serial".into()),..Default::default()};
    let xmp=initial.source.path.with_extension("xmp");std::fs::write(&xmp,lightcraft_meta::write_xmp(&metadata,None)).unwrap();
    let p=engine::import_photo(&initial.source.path).unwrap().0;
    for(tiff,ext)in [(false,"jpg"),(true,"tif")]{
        let out=tmp.path().join(format!("filtered.{ext}"));engine::export_photo(&p,&out,tiff,true,&AtomicBool::new(false)).unwrap();let bytes=std::fs::read(&out).unwrap();
        let m=lightcraft_meta::extract(&bytes);assert!(m.serial_number.is_none()&&m.lens_serial_number.is_none());
        for private in ["private-body-serial","private-lens-serial"]{assert!(!bytes.windows(private.len()).any(|w|w==private.as_bytes()));}
    }
}
#[test]fn history_survives_reopen_and_copy_never_overwrites_or_accepts_changed_bytes(){
    let tmp=tempfile::tempdir().unwrap();let p=photo(tmp.path());let out=tmp.path().join("first.jpg");let report=engine::export_photo(&p,&out,false,false,&AtomicBool::new(false)).unwrap();let r=receipt(&p,&out,report["edit_sha256"].as_str().unwrap().into());let mut project=Project::default();project.photos.push(p);project.publishes.push(r.clone());let dir=tmp.path().join("project");project.save(&dir).unwrap();let reopened=Project::open(&dir).unwrap();assert_eq!(reopened.publishes.len(),1);assert_eq!(reopened.publishes[0].preview().unwrap().dimensions(),(48,32));let copy=tmp.path().join("copy.jpg");r.copy_with_commit(&copy,&||Ok(())).unwrap();assert_eq!(hash_file(&copy).unwrap(),r.output_sha256);let original=std::fs::read(&copy).unwrap();assert!(r.copy_with_commit(&copy,&||Ok(())).is_err());assert_eq!(std::fs::read(&copy).unwrap(),original);std::fs::write(&out,b"changed fixture").unwrap();assert!(r.preview().is_err());assert!(r.copy_with_commit(&tmp.path().join("bad.jpg"),&||Ok(())).is_err());
}
#[test]fn changing_lut_contents_changes_the_edit_fingerprint_and_old_receipt_stays_frozen(){
    let tmp=tempfile::tempdir().unwrap();let mut p=photo(tmp.path());let library=tmp.path().join("library");std::fs::create_dir_all(&library).unwrap();let cube=library.join("fixture.cube");let data=|color:&str|format!("LUT_3D_SIZE 2\n{}",format!("{color}\n").repeat(8));std::fs::write(&cube,data("1 0 0")).unwrap();let id=format!("lut:fixture-{}",luminacraft::ai_style::new_id());std::fs::write(library.join("prefs.json"),serde_json::to_vec(&serde_json::json!({"lutProfiles":[{"id":id,"name":"fixture","group":"fixture","file":cube}]})).unwrap()).unwrap();p.catalog_ref=Some(CatalogRef{library,id:1,copy_name:None});p.edit.develop.profile.id=id.clone();p.edit.develop.profile.amount=100.0;
    let first=photo_edit_hash(&p).unwrap();let out=tmp.path().join("red.jpg");let report=engine::export_photo_with_commit(&p,&out,false,false,&AtomicBool::new(false),&||{std::fs::write(&cube,data("0 1 0"))?;Ok(())}).unwrap();assert_eq!(report["edit_sha256"],first);assert_ne!(photo_edit_hash(&p).unwrap(),first);let red=image::open(&out).unwrap().to_rgb8();let green_path=tmp.path().join("green.jpg");engine::export_photo(&p,&green_path,false,false,&AtomicBool::new(false)).unwrap();assert_ne!(red,image::open(green_path).unwrap().to_rgb8());std::fs::remove_file(&cube).unwrap();assert!(photo_edit_hash(&p).is_err());lightcraft_pipeline::lut::unregister(&id);
}

