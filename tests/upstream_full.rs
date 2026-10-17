use lightcraft_engine::{Session, Selection};
use lightcraft_catalog::PhotoId;
use serde_json::json;
use std::{fs, path::Path};

fn png(path: &Path, value: u8) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbImage::from_pixel(24, 18, image::Rgb([value, 50, 120])).save(path).unwrap();
}

#[test]
fn full_engine_import_copy_and_move_preserve_sidecars_and_catalog_persistence() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("card");
    let first = source.join("copy.png");
    let second = source.join("move.png");
    png(&first, 30); png(&second, 100);
    fs::write(source.join("copy.XMP"), "<original>unchanged</original>").unwrap();
    fs::write(source.join("copy.png.xmp"), "<original>full name</original>").unwrap();
    fs::write(source.join("move.xmp"), "<original>moved</original>").unwrap();
    let original = fs::read(&first).unwrap();
    let library = temp.path().join("library");
    let target = temp.path().join("archive");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("copy.XMP"), "existing unrelated sidecar").unwrap();
    let mut session = Session::new().with_fs().with_system_clock();
    session.open_library(&library, false).unwrap();
    let copied = session.execute("library.import", &json!({"paths":[first], "mode":"copy", "destination":target, "organize":"flat"})).unwrap();
    assert_eq!(copied["imported"].as_array().unwrap().len(), 1, "{copied}");
    assert_eq!(fs::read(&first).unwrap(), original);
    assert_eq!(fs::read_to_string(target.join("copy.XMP")).unwrap(), "existing unrelated sidecar");
    assert_eq!(fs::read(target.join("copy-1.png")).unwrap(), original);
    assert_eq!(fs::read(target.join("copy-1.XMP")).unwrap(), fs::read(source.join("copy.XMP")).unwrap());
    assert_eq!(fs::read(target.join("copy-1.png.xmp")).unwrap(), fs::read(source.join("copy.png.xmp")).unwrap());
    let moved = session.execute("library.import", &json!({"paths":[second], "mode":"move", "destination":target, "organize":"flat"})).unwrap();
    assert_eq!(moved["moved"].as_array().unwrap().len(), 1, "{moved}");
    assert!(!second.exists()); assert!(!source.join("move.xmp").exists());
    assert!(target.join("move.png").exists()); assert!(target.join("move.xmp").exists());
    session.close_library().unwrap();
    drop(session);
    let mut reopened = Session::new().with_fs();
    reopened.open_library(&library, false).unwrap();
    assert_eq!(reopened.catalog.len(), 2);
    reopened.close_library().unwrap();
}

#[test]
fn full_catalog_stacks_virtual_copies_and_original_protection() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("photo.png"); png(&path, 100);
    let xmp = temp.path().join("photo.xmp"); fs::write(&xmp, "original XMP").unwrap();
    let mut session = Session::new().with_fs();
    let report = session.execute("library.import", &json!({"paths":[path], "mode":"add"})).unwrap();
    let id = PhotoId(report["imported"][0].as_u64().unwrap());
    session.selection = Selection::single(id);
    let copy = session.execute("photo.virtualCopy", &json!({})).unwrap();
    assert_eq!(copy["ids"].as_array().unwrap().len(), 1);
    assert_eq!(session.catalog.len(), 2);
    assert_eq!(session.catalog.stacks().count(), 1);
    for command in ["photo.saveMetadataToFile", "library.toggleAutoWriteXmp", "photo.rename", "folder.move"] {
        assert!(session.execute(command, &json!({})).is_err(), "{command}");
        assert!(!session.commands().iter().find(|c| c.id == command).unwrap().enabled);
    }
    session.xmp.auto_write = true;
    session.execute("photo.rate", &json!({"rating":4})).unwrap();
    assert!(!session.xmp.auto_write);
    assert_eq!(fs::read_to_string(xmp).unwrap(), "original XMP");
    assert!(path.exists());
    assert!(session.commands().len() > 200, "complete registry is linked");
}
