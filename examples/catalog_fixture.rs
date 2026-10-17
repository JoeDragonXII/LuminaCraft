//! Synthetic, isolated 128-version catalog for native export/open acceptance.
use luminacraft::{catalog_io::Export,library::Library,project::{Project,CatalogRef}};
fn main()->anyhow::Result<()> {
    let root=std::path::PathBuf::from(std::env::args().nth(1).ok_or_else(||anyhow::anyhow!("new fixture directory required"))?);
    anyhow::ensure!(!root.exists(),"fixture directory must not exist");std::fs::create_dir_all(root.join("source"))?;
    let root=root.canonicalize()?;let path=root.join("source/fixture.png");
    image::RgbImage::from_fn(480,320,|x,y|image::Rgb([(x/2) as u8,(y/2) as u8,100])).save(&path)?;
    let dir=root.join("library");let mut lib=Library::open(&dir)?;let (base,_)=luminacraft::engine::import_photo(&path)?;let indexed=luminacraft::library::index_photo(&path)?;let mut project=Project::default();
    for n in 0..128 {
        let mut native=indexed.clone();native.id=lightcraft_catalog::PhotoId(n+1);if n>0{native.copy_of=Some(lightcraft_catalog::PhotoId(1));native.copy_name=Some(format!("Copy {n}"));}
        let mut photo=base.clone();photo.catalog_ref=Some(CatalogRef{library:dir.clone(),id:n+1,copy_name:native.copy_name.clone()});photo.edit.develop.light.exposure=n as f64/128.0;photo.edit.retouch.smooth=0.2;
        lib.apply(vec![lightcraft_catalog::Op::AddPhoto{photo:Box::new(native)}])?;project.photos.push(photo);
    }
    Export{catalog:lib.catalog.clone(),library:dir,project:Some(project),selected:None,copy_originals:false}.run(&root.join("seed"),&std::sync::atomic::AtomicBool::new(false))?;
    println!("Synthetic catalog fixture: 128 versions, one original.");Ok(())
}
