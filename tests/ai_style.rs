//! Local protocol fixtures only. These tests do not run a remote image model.
use luminacraft::{ai_style::*, project::{Project,hash_file}};
use base64::{Engine as _,engine::general_purpose::STANDARD};
use std::{io::{Read,Write,Cursor},net::TcpListener,sync::{Arc,Mutex,atomic::AtomicBool},time::Duration};
use anyhow::Result;

struct MemorySecrets;
impl Secrets for MemorySecrets {
    fn get(&self,_:&str)->Result<String>{Ok("fixture-secret-never-persist".into())}
    fn set(&self,_:&str,_:&str)->Result<()>{Ok(())}
    fn delete(&self,_:&str)->Result<()>{Ok(())}
}
fn png()->Vec<u8>{let mut b=Cursor::new(vec![]);image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(48,32,image::Rgb([20,130,210]))).write_to(&mut b,image::ImageFormat::Png).unwrap();b.into_inner()}
fn server(status:u16,body:String,delay:Duration)->(Service,Arc<Mutex<Vec<u8>>>,std::thread::JoinHandle<()>) {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let addr=listener.local_addr().unwrap();let capture=Arc::new(Mutex::new(vec![]));let copied=capture.clone();
    let thread=std::thread::spawn(move||{
        let (mut stream,_)=listener.accept().unwrap();stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut bytes=vec![];let mut buffer=[0u8;8192];
        loop {let n=stream.read(&mut buffer).unwrap();assert!(n>0);bytes.extend_from_slice(&buffer[..n]);
            if let Some(end)=bytes.windows(4).position(|v|v==b"\r\n\r\n"){
                let headers=String::from_utf8_lossy(&bytes[..end]);let len=headers.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|s|s.trim().parse::<usize>().ok())).unwrap_or(0);
                if bytes.len()>=end+4+len{break;}
            }
        }
        *copied.lock().unwrap()=bytes;std::thread::sleep(delay);
        let _=write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
    });(Service{base_url:format!("http://{addr}/v1"),..Default::default()},capture,thread)
}
#[test]
fn templates_and_profiles_roundtrip_without_credentials(){
    let dir=tempfile::tempdir().unwrap();let service=Service::default();
    let t=Template{id:new_id(),name:"成熟提示词".into(),prompt:"  保留换行\n水彩风格\r\n请勿裁切  ".into(),service_id:Some(service.id.clone())};
    let settings=Settings{default_service:Some(service.id.clone()),services:vec![service],templates:vec![t.clone()]};settings.save(dir.path()).unwrap();
    let loaded=Settings::load(dir.path()).unwrap();assert_eq!(loaded.templates[0].prompt,t.prompt);assert!(loaded.service_for(&loaded.templates[0]).is_some());
    let json=String::from_utf8(std::fs::read(dir.path().join("settings.json")).unwrap()).unwrap();assert!(!json.contains("secret"));assert!(!json.contains("api_key"));
    let mut changed=loaded.services[0].clone();changed.base_url="https://example.com/v1".into();assert_ne!(changed.credential_id(),loaded.services[0].credential_id());
    for address in ["https://key@example.com/v1","https://example.com/v1?key=secret","http://example.com/v1"]{changed.base_url=address.into();assert!(changed.validate().is_err());}
    std::fs::write(dir.path().join("settings.json"),b"broken configuration").unwrap();assert!(Settings::load(dir.path()).is_err());assert_eq!(std::fs::read(dir.path().join("settings.json")).unwrap(),b"broken configuration");
}
#[test]
fn generation_sends_exact_prompt_current_render_and_recovers_unsaved_receipt(){
    let temp=tempfile::tempdir().unwrap();let input=temp.path().join("original.png");image::RgbImage::from_pixel(48,32,image::Rgb([90,70,50])).save(&input).unwrap();
    let (photo,_)=luminacraft::engine::import_photo(&input).unwrap();let original=hash_file(&input).unwrap();let mut project=Project::default();project.photos.push(photo);let dir=temp.path().join("project");project.save(&dir).unwrap();
    // Submit an unsaved edit snapshot. The saved catalog's old parameters must not be used.
    project.photos[0].edit.develop.light.exposure=1.0;
    let prompt="  Exact prompt 中文\r\nline two\n  ";let bytes=png();
    let (service,capture,thread)=server(200,serde_json::json!({"data":[{"b64_json":STANDARD.encode(&bytes)}]}).to_string(),Duration::ZERO);
    let stages=Mutex::new(vec![]);
    let record=generate(Generate{project_id:project.id.clone(),directory:dir.clone(),photo:project.photos[0].clone(),service,template_name:"水彩".into(),prompt:prompt.into()},&MemorySecrets,&AtomicBool::new(false),&|s|stages.lock().unwrap().push(s)).unwrap();thread.join().unwrap();
    assert_eq!(*stages.lock().unwrap(),vec![GenerationStage::Preparing,GenerationStage::Generating,GenerationStage::Saving]);
    let request=capture.lock().unwrap();let text=String::from_utf8_lossy(&request);assert!(text.starts_with("POST /v1/images/edits "));assert!(text.contains(&format!("name=\"prompt\"\r\n\r\n{prompt}\r\n--")));assert!(text.contains("Bearer fixture-secret-never-persist"));
    assert_eq!(record.prompt,prompt);assert_eq!(std::fs::read(&record.output).unwrap(),bytes);assert_eq!(record.edit_sha256,luminacraft::project::photo_edit_hash(&project.photos[0]).unwrap());
    let transmitted=std::fs::read(&record.input).unwrap();assert!(request.windows(transmitted.len()).any(|w|w==transmitted));
    let source=luminacraft::engine::Source::load_photo(&project.photos[0],&project.photos[0].edit.develop,2048).unwrap();let rendered=luminacraft::engine::render_photo(&source,&project.photos[0].edit,2048,&AtomicBool::new(false)).unwrap();
    let decoded=image::load_from_memory(&transmitted).unwrap().into_rgba8();assert_eq!(decoded.as_raw(),&rendered.image.data.iter().flatten().copied().collect::<Vec<u8>>());
    assert_eq!(hash_file(&input).unwrap(),original);assert!(record.matches(&project.photos[0]));
    // UI can crash before appending/saving the record; durable receipt is discovered on open.
    let mut reopened=Project::open(&dir).unwrap();assert_eq!(reopened.style_outputs.len(),1);assert_ne!(reopened.digest().unwrap(),*reopened.saved_digest.as_ref().unwrap());
    reopened.save(&dir).unwrap();assert_eq!(Project::open(&dir).unwrap().style_outputs.len(),1);
    let export=temp.path().join("result.png");record.export(&export,false).unwrap();let old=std::fs::read(&export).unwrap();assert!(record.export(&export,false).is_err());assert_eq!(std::fs::read(export).unwrap(),old);
    let jpg=temp.path().join("result.jpg");record.export(&jpg,true).unwrap();assert_eq!(image::image_dimensions(jpg).unwrap(),(48,32));
    assert!(!std::fs::read_to_string(record.output.parent().unwrap().join("result.json")).unwrap().contains("fixture-secret"));
    std::fs::write(&record.output,b"corrupted").unwrap();assert!(record.preview(false).is_err());assert!(record.export(&temp.path().join("bad.png"),false).is_err());
    assert!(Project::open(&dir).is_ok(),"missing/changed outputs must not prevent editing original photos");
    std::fs::write(record.output.parent().unwrap().join("result.json"),b"broken receipt").unwrap();
    assert!(Project::open(&dir).is_ok(),"damaged recovery receipts must not prevent opening the saved project");
}
#[test]
fn generation_retains_the_submitted_lut_fingerprint_when_the_resource_changes_while_waiting(){
    let temp=tempfile::tempdir().unwrap();let input=temp.path().join("photo.png");image::RgbImage::from_pixel(48,32,image::Rgb([90,70,50])).save(&input).unwrap();let(mut photo,_)=luminacraft::engine::import_photo(&input).unwrap();let library=temp.path().join("library");std::fs::create_dir_all(&library).unwrap();let cube=library.join("style.cube");let data=|color:&str|format!("LUT_3D_SIZE 2\n{}",format!("{color}\n").repeat(8));std::fs::write(&cube,data("1 0 0")).unwrap();let id=format!("lut:style-{}",new_id());std::fs::write(library.join("prefs.json"),serde_json::to_vec(&serde_json::json!({"lutProfiles":[{"id":id,"name":"fixture","group":"fixture","file":cube}]})).unwrap()).unwrap();photo.catalog_ref=Some(luminacraft::project::CatalogRef{library,id:1,copy_name:None});photo.edit.develop.profile.id=id.clone();photo.edit.develop.profile.amount=100.0;
    let submitted=luminacraft::project::photo_edit_hash(&photo).unwrap();let mut project=Project::default();project.photos.push(photo.clone());let dir=temp.path().join("project");project.save(&dir).unwrap();let(service,_,thread)=server(200,serde_json::json!({"data":[{"b64_json":STANDARD.encode(png())}]}).to_string(),Duration::ZERO);
    let record=generate(Generate{project_id:project.id,directory:dir,photo:photo.clone(),service,template_name:"fixture".into(),prompt:"fixture".into()},&MemorySecrets,&AtomicBool::new(false),&|stage|{if stage==GenerationStage::Generating{std::fs::write(&cube,data("0 1 0")).unwrap();}}).unwrap();thread.join().unwrap();assert_eq!(record.edit_sha256,submitted);assert_ne!(record.edit_sha256,luminacraft::project::photo_edit_hash(&photo).unwrap());let decoded=image::open(record.input).unwrap().to_rgb8();let pixel=decoded.get_pixel(0,0);assert!(pixel[0]>pixel[1]);lightcraft_pipeline::lut::unregister(&id);
}
#[test]
fn http_failures_are_redacted_and_never_retried(){
    for (status,message) in [(401,"认证"),(429,"限流"),(500,"HTTP 500"),(302,"HTTP 302")]{
        let (s,_,t)=server(status,"private provider body with fixture-secret".into(),Duration::ZERO);
        let error=edit_image(&s,"fixture-secret","prompt",&png(),&AtomicBool::new(false),Duration::from_secs(3)).unwrap_err().to_string();t.join().unwrap();
        assert!(error.contains(message),"{error}");assert!(!error.contains("fixture-secret"));
    }
    let (s,_,t)=server(200,"not json".into(),Duration::ZERO);assert!(edit_image(&s,"key","p",&png(),&AtomicBool::new(false),Duration::from_secs(3)).is_err());t.join().unwrap();
    let (s,_,t)=server(200,serde_json::json!({"data":[{"b64_json":STANDARD.encode(b"not a picture")}]}).to_string(),Duration::ZERO);
    assert!(edit_image(&s,"key","p",&png(),&AtomicBool::new(false),Duration::from_secs(3)).is_err());t.join().unwrap();
}
#[test]
fn cancellation_and_timeouts_do_not_submit_retries(){
    let cancelled=AtomicBool::new(true);let s=Service::default();assert!(edit_image(&s,"key","p",&png(),&cancelled,Duration::from_secs(1)).unwrap_err().to_string().contains("取消"));
    let (s,_,thread)=server(200,"{}".into(),Duration::from_millis(200));assert!(edit_image(&s,"key","p",&png(),&AtomicBool::new(false),Duration::from_millis(50)).unwrap_err().to_string().contains("超时"));thread.join().unwrap();
    let (s,capture,thread)=server(200,serde_json::json!({"data":[{"b64_json":STANDARD.encode(png())}]}).to_string(),Duration::from_millis(100));
    let flag=Arc::new(AtomicBool::new(false));let f=flag.clone();let canceller=std::thread::spawn(move||{while capture.lock().unwrap().is_empty(){std::thread::sleep(Duration::from_millis(5));}f.store(true,std::sync::atomic::Ordering::Relaxed);});
    assert!(edit_image(&s,"key","p",&png(),&flag,Duration::from_secs(3)).unwrap_err().to_string().contains("取消"));thread.join().unwrap();canceller.join().unwrap();
}
#[test]
fn windows_credential_roundtrip_uses_only_a_new_fixture_entry(){
    let id=new_id();WindowsSecrets.set(&id,"temporary-fixture-key").unwrap();assert_eq!(WindowsSecrets.get(&id).unwrap(),"temporary-fixture-key");WindowsSecrets.delete(&id).unwrap();assert!(WindowsSecrets.get(&id).is_err());
}
#[test]
fn connection_test_reads_only_model_metadata(){
    let (service,capture,thread)=server(200,serde_json::json!({"data":[{"id":"gpt-image-2.5-sunburst"}]}).to_string(),Duration::ZERO);
    test_connection(&service,&MemorySecrets).unwrap();thread.join().unwrap();
    let data=capture.lock().unwrap();let request=String::from_utf8_lossy(&data);assert!(request.starts_with("GET /v1/models "));assert!(!request.contains("image/png"));
}

#[test]
fn builtin_map_blocks_missing_location_and_sends_fixed_policy_for_approximate_area(){
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("no-gps.png");
    image::RgbImage::from_pixel(48,32,image::Rgb([90,70,50])).save(&path).unwrap();
    let (photo,_)=luminacraft::engine::import_photo(&path).unwrap();let mut p=Project::default();p.photos.push(photo.clone());
    let dir=temp.path().join("project");p.save(&dir).unwrap();let hash=hash_file(&path).unwrap();
    // No listener exists: missing location must fail locally before any HTTP request.
    let request=Generate{project_id:p.id.clone(),directory:dir.clone(),photo:photo.clone(),service:Service{base_url:"http://127.0.0.1:1/v1".into(),..Default::default()},template_name:luminacraft::map_style::NAME.into(),prompt:String::new()};
    assert!(generate(request,&MemorySecrets,&AtomicBool::new(false),&|_|{}).unwrap_err().to_string().contains("没有读取到 GPS"));
    assert!(!dir.join("ai-style").exists());
    let (service,capture,thread)=server(200,serde_json::json!({"data":[{"b64_json":STANDARD.encode(png())}]}).to_string(),Duration::ZERO);
    let output=generate(Generate{project_id:p.id,directory:dir,photo,service,template_name:luminacraft::map_style::NAME.into(),prompt:"广州某村".into()},&MemorySecrets,&AtomicBool::new(false),&|_|{}).unwrap();thread.join().unwrap();
    assert!(output.prompt.starts_with(luminacraft::map_style::RULES));assert!(output.prompt.contains("Approximate Area"));
    assert!(String::from_utf8_lossy(&capture.lock().unwrap()).contains("广州某村"));assert_eq!(hash_file(&path).unwrap(),hash);
}
