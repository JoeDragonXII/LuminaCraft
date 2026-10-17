//! Isolated source-preserving folder UI acceptance fixture.
fn main()->anyhow::Result<()>{
    let root=std::path::PathBuf::from(std::env::args().nth(1).ok_or_else(||anyhow::anyhow!("new fixture directory required"))?);anyhow::ensure!(!root.exists(),"fixture directory must not exist");std::fs::create_dir_all(&root)?;let root=root.canonicalize()?;
    let mut photos=vec![];
    for (n,name,date) in [(0,"photos/2025/A/one.png","2025-08-01T10:00:00"),(1,"photos/2025/A/two.png","2025-08-02T10:00:00"),(2,"photos/2026/B/three.png","2026-06-01T10:00:00"),(3,"photos/2026/B/four.png","2026-06-02T10:00:00")]{
        let path=root.join(name);std::fs::create_dir_all(path.parent().ok_or_else(||anyhow::anyhow!("no parent"))?)?;let mut image=image::RgbImage::new(480,320);
        for (x,y,p) in image.enumerate_pixels_mut(){*p=image::Rgb([((x/2+n*40)%256)as u8,((y/2+n*20)%256)as u8,if (x/60+y/60)%2==0{80}else{170}]);}image.save(&path)?;
        let mut photo=luminacraft::library::index_photo(&path)?;photo.captured=Some(date.into());photo.meta.keywords=vec!["旅途|北京".into()];photos.push(photo);
    }
    image::RgbImage::from_pixel(480,320,image::Rgb([150,100,70])).save(root.join("candidate.png"))?;
    let mut lib=luminacraft::library::Library::open(&root.join("library"))?;lib.add(photos)?;
    println!("Isolated library fixture: four synthetic photos in two folders.");Ok(())
}
