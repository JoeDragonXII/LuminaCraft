use lightcraft_catalog::{PhotoId,Op,Album,AlbumId};
use lightcraft_ui_egui::{LightcraftApp,Services,state::Dialog};
use std::fs;
fn app(base:&std::path::Path)->LightcraftApp {
    let mut s=lightcraft_engine::Session::new().with_fs();s.open_library(base.join("library"),false).unwrap();
    for n in 1..=2{let p=base.join(format!("{n}.png"));image::RgbImage::from_pixel(12,8,image::Rgb([20,70,100])).save(&p).unwrap();let mut photo=luminacraft::library::index_photo(&p).unwrap();photo.id=PhotoId(n);s.commit("fixture",Op::AddPhoto{photo:Box::new(photo)}).unwrap();}
    s.persist().unwrap();s.execute("library.select",&serde_json::json!({"ids":[1],"active":1})).unwrap();
    LightcraftApp::new(s,Services{delete_from_disk:Some(luminacraft::deletion::advanced),..Default::default()})
}
#[test]fn advanced_delete_choice_freezes_ids_and_album_scope(){let t=tempfile::tempdir().unwrap();let mut a=app(t.path());let mut album=Album::new(AlbumId(1),"fixture");album.photos=vec![PhotoId(1),PhotoId(2)];a.session.commit("album",Op::AddAlbum{album}).unwrap();a.session.source=lightcraft_engine::LibrarySource::Album(AlbumId(1));assert!(lightcraft_ui_egui::menus::confirm_delete(&mut a));a.session.execute("library.select",&serde_json::json!({"ids":[2],"active":2})).unwrap();match &a.ui.dialog{Some(Dialog::DeleteChoice{ids,album})=>{assert_eq!(ids,&vec![PhotoId(1)]);assert_eq!(*album,Some(1));},_=>panic!("missing delete choice")};assert!(t.path().join("1.png").exists());assert!(t.path().join("2.png").exists());}
#[test]fn disk_preview_does_not_delete_and_changed_original_cannot_be_confirmed(){let t=tempfile::tempdir().unwrap();let mut a=app(t.path());a.run("app.deleteFromDisk",serde_json::json!({"ids":[1]})).unwrap();let d=a.ui.dialog.clone().unwrap();assert!(matches!(&d,Dialog::ConfirmDiskDelete{..}));assert!(t.path().join("1.png").exists());fs::write(t.path().join("1.png"),"changed fixture").unwrap();let error=lightcraft_ui_egui::panels::dialogs::confirm_dialog(&mut a,&d).unwrap_err();assert!(error.contains("已变化"),"{error}");assert!(t.path().join("1.png").exists());assert_eq!(a.session.catalog.len(),2);}
