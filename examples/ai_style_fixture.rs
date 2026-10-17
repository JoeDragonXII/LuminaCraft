//! Explicit synthetic native QA fixture. Never reads user keys/photos.
use luminacraft::{ai_style::{self,Secrets},project::Project};
fn main()->anyhow::Result<()>{
    let args=std::env::args().skip(1).collect::<Vec<_>>();anyhow::ensure!(args.len()>=2,"seed ROOT URL | cleanup ROOT");
    let root=std::path::PathBuf::from(&args[1]);
    if args[0]=="cleanup" {
        let s=ai_style::Settings::load(&root.join("ai-settings"))?;
        anyhow::ensure!(s.services.iter().all(|s|s.name=="Native protocol fixture" && s.base_url.starts_with("http://127.0.0.1:")),"not QA fixture settings");
        for service in s.services{ai_style::WindowsSecrets.delete(&service.credential_id())?;}return Ok(());
    }
    anyhow::ensure!(args[0]=="seed" && args.len()==3 && !root.exists(),"new isolated fixture directory required");
    std::fs::create_dir_all(&root)?;let root=root.canonicalize()?;
    let source=root.join("synthetic.png");let mut image=image::RgbImage::new(640,420);
    for (x,y,p) in image.enumerate_pixels_mut(){*p=image::Rgb([((x*255)/640)as u8,((y*255)/420)as u8,if (x/70+y/70)%2==0{65}else{190}]);}
    image.save(&source)?;
    for (_,_,p) in image.enumerate_pixels_mut(){p.0=[p[2],p[0],p[1]];}image.save(root.join("fixture-output.png"))?;
    let (photo,_)=luminacraft::engine::import_photo(&source)?;let mut project=Project::default();project.title="AI 协议夹具 · 非模型效果".into();project.photos.push(photo);
    let second=root.join("synthetic-second.png");image::imageops::rotate90(&image).save(&second)?;let (second,_)=luminacraft::engine::import_photo(&second)?;project.photos.push(second);
    project.save(&root.join("project"))?;
    let service=ai_style::Service{name:"Native protocol fixture".into(),base_url:args[2].clone(),..Default::default()};
    let template=ai_style::Template{id:ai_style::new_id(),name:"协议夹具（非 AI 效果）".into(),prompt:"  Preserve this exact prompt.\n水彩风格；不改写。  ".into(),service_id:Some(service.id.clone())};
    let settings=ai_style::Settings{default_service:Some(service.id.clone()),services:vec![service.clone()],templates:vec![template.clone()]};
    settings.save(&root.join("ai-settings"))?;ai_style::WindowsSecrets.set(&service.credential_id(),"native-fixture-not-a-real-key")?;
    std::fs::write(root.join("fixture.json"),serde_json::to_vec_pretty(&serde_json::json!({"project":root.join("project"),"template":template.id,"source":source}))?)?;
    println!("Synthetic fixture created; no remote inference.");Ok(())
}
