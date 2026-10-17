use luminacraft::{engine,project::CatalogRef};
use serde_json::json;
use std::sync::atomic::AtomicBool;

#[test]
fn imported_lut_survives_a_cold_main_renderer_and_missing_resource_is_explicit(){
    let tmp=tempfile::tempdir().unwrap();let path=tmp.path().join("photo.png");
    image::RgbImage::from_pixel(32,24,image::Rgb([120,80,40])).save(&path).unwrap();
    let cube=tmp.path().join("handoff-resource.cube");
    // A constant red LUT makes omission observable, rather than using identity data.
    std::fs::write(&cube,"LUT_3D_SIZE 2\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n1 0 0\n").unwrap();
    let library=tmp.path().join("library");
    let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(&library,false).unwrap();
    let imported=full.execute("profile.import",&json!({"paths":[cube]})).unwrap();
    let id=imported["imported"][0]["id"].as_str().unwrap().to_string();
    let (mut photo,source)=engine::import_photo(&path).unwrap();
    photo.catalog_ref=Some(CatalogRef{library:library.clone(),id:1,copy_name:None});
    let cancel=AtomicBool::new(false);
    let ordinary=engine::render_photo(&source,&photo.edit,32,&cancel).unwrap();
    photo.edit.develop.profile.id=id.clone();photo.edit.develop.profile.amount=100.0;
    let advanced=engine::render_photo(&source,&photo.edit,32,&cancel).unwrap();
    assert_ne!(ordinary.image,advanced.image);
    lightcraft_pipeline::lut::unregister(&id);
    source.cache.clear();
    assert!(engine::render_photo(&source,&photo.edit,32,&cancel).is_err());
    assert!(engine::prepare_profile(&photo,&photo.edit).unwrap());
    let returned=engine::render_photo(&source,&photo.edit,32,&cancel).unwrap();
    assert_eq!(advanced.image,returned.image);
    // A warm registry must not hide a file that was deleted from this library.
    let prefs:serde_json::Value=serde_json::from_slice(&std::fs::read(library.join("prefs.json")).unwrap()).unwrap();
    let file=prefs["lutProfiles"][0]["file"].as_str().unwrap();std::fs::remove_file(file).unwrap();
    assert!(engine::prepare_profile(&photo,&photo.edit).is_err());
    lightcraft_pipeline::lut::unregister(&id);
}
