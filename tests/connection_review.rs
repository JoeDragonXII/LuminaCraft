//! Local HTTP and temporary filesystem fixtures only; never contacts a user service.
use anyhow::Result;
use luminacraft::{ai_style::{Settings,Service,Secrets},project::{PublishRecord,hash_file},worker::{self,Job,Outcome,Request}};
use std::{collections::HashMap,io::{Read,Write},net::TcpListener,sync::{Mutex,Arc,atomic::AtomicBool},time::Duration};

#[derive(Default)]struct Store(Mutex<HashMap<String,String>>);
impl Secrets for Store{
    fn get(&self,id:&str)->Result<String>{self.get_optional(id)?.ok_or_else(||anyhow::anyhow!("missing"))}
    fn get_optional(&self,id:&str)->Result<Option<String>>{Ok(self.0.lock().unwrap().get(id).cloned())}
    fn set(&self,id:&str,key:&str)->Result<()>{self.0.lock().unwrap().insert(id.into(),key.into());anyhow::ensure!(key!="fail-after-write","fixture write failed");Ok(())}
    fn delete(&self,id:&str)->Result<()>{self.0.lock().unwrap().remove(id);Ok(())}
}
fn settings(services:Vec<Service>)->Settings{Settings{default_service:services.first().map(|s|s.id.clone()),services,templates:vec![]}}
#[test]fn settings_commit_failure_restores_keys_and_removes_new_keys(){
    let temp=tempfile::tempdir().unwrap();let old_service=Service::default();let new_service=Service::default();let old=settings(vec![old_service.clone()]);let new=settings(vec![old_service.clone(),new_service.clone()]);let store=Store::default();
    store.set(&old_service.credential_id(),"original").unwrap();
    let blocked=temp.path().join("not-a-directory");std::fs::write(&blocked,b"fixture").unwrap();
    let keys=HashMap::from([(old_service.id.clone(),"replacement".into()),(new_service.id.clone(),"new".into())]);
    assert!(new.save_with_secrets(&blocked,&old,&keys,&store).is_err());
    assert_eq!(store.get(&old_service.credential_id()).unwrap(),"original");assert!(store.get_optional(&new_service.credential_id()).unwrap().is_none());
}
#[test]fn credential_failure_rolls_back_all_attempted_writes(){
    let temp=tempfile::tempdir().unwrap();let a=Service::default();let b=Service::default();let old=settings(vec![a.clone(),b.clone()]);old.save(temp.path()).unwrap();let before=std::fs::read(temp.path().join("settings.json")).unwrap();let store=Store::default();
    store.set(&a.credential_id(),"old-a").unwrap();store.set(&b.credential_id(),"old-b").unwrap();
    assert!(old.save_with_secrets(temp.path(),&old,&HashMap::from([(a.id.clone(),"new-a".into()),(b.id.clone(),"fail-after-write".into())]),&store).is_err());
    assert_eq!(store.get(&a.credential_id()).unwrap(),"old-a");assert_eq!(store.get(&b.credential_id()).unwrap(),"old-b");assert_eq!(std::fs::read(temp.path().join("settings.json")).unwrap(),before);
}
#[test]fn all_configuration_is_validated_before_any_key_changes(){
    let temp=tempfile::tempdir().unwrap();let a=Service::default();let b=Service::default();let old=settings(vec![a.clone(),b.clone()]);let mut new=old.clone();new.services[1].base_url="https://example.org/v1".into();let store=Store::default();store.set(&a.credential_id(),"original").unwrap();
    assert!(new.save_with_secrets(temp.path(),&old,&HashMap::from([(a.id.clone(),"replacement".into())]),&store).is_err());assert_eq!(store.get(&a.credential_id()).unwrap(),"original");
}
fn record(path:std::path::PathBuf)->PublishRecord{PublishRecord{lumina_base:None,source_sha256:String::new(),edit_sha256:String::new(),output_sha256:hash_file(&path).unwrap(),output:path,asset_id:None,collection_id:None,origin:None}}
#[test]fn remote_receipts_are_bound_to_a_site_and_old_unknown_ids_are_rejected(){
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("image.jpg");std::fs::write(&path,[255,216,255]).unwrap();let mut r=record(path);r.bind_lumina("http://127.0.0.1:49865/").unwrap();r.asset_id=Some("site-a-asset".into());r.collection_id=Some("site-a-collection".into());
    r.bind_lumina("http://127.0.0.1:49865").unwrap();assert!(r.asset_id.is_some());
    r.bind_lumina("http://127.0.0.1:49866").unwrap();assert!(r.asset_id.is_none()&&r.collection_id.is_none());
    r.asset_id=Some("legacy".into());r.lumina_base=None;assert!(r.bind_lumina("http://127.0.0.1:49865").is_err());
    let mut json=serde_json::to_value(&r).unwrap();json.as_object_mut().unwrap().remove("lumina_base");assert!(serde_json::from_value::<PublishRecord>(json).is_ok());
}
fn server(replies:Vec<(u16,&'static str,&'static str)>)->(String,std::thread::JoinHandle<()>){
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://{}",listener.local_addr().unwrap());
    let handle=std::thread::spawn(move||for(status,extra,body)in replies{
        let(mut stream,_)=listener.accept().unwrap();stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();let mut data=vec![];let mut buf=[0;4096];
        loop{let n=stream.read(&mut buf).unwrap();assert!(n>0);data.extend_from_slice(&buf[..n]);if let Some(end)=data.windows(4).position(|v|v==b"\r\n\r\n"){let h=String::from_utf8_lossy(&data[..end]);let len=h.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v|v.trim().parse::<usize>().ok())).unwrap_or(0);if data.len()>=end+4+len{break;}}}
        write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",body.len()).unwrap();
    });(base,handle)
}
#[test]fn deferred_lumina_jobs_are_rejected_without_network_access(){
    let(tx,rx)=worker::start(eframe::egui::Context::default());
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("image.jpg");std::fs::write(&path,[255,216,255]).unwrap();
    for (id,job)in [(1,Job::Connect("http://127.0.0.1:1".into(),"fixture".into())),(2,Job::Publish(record(path),"title".into(),None,false))]{
        tx.send(Request{id,job,cancel:Arc::new(AtomicBool::new(false))}).unwrap();assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().result.err().unwrap().contains("本地导出"));
    }
}
#[test]fn login_redirects_and_urls_with_hidden_routing_are_rejected(){
    for url in ["http://localhost:3002?x=y","http://localhost:3002/#fragment","http://user@localhost:3002","http://example.org:3002"]{assert!(luminacraft::lumina::normalize_base(url).is_err());}
    let(base,thread)=server(vec![(307,"Location: http://127.0.0.1:1/collect\r\n","{}")]);assert!(luminacraft::lumina::LuminaClient::connect(&base,"fixture-password").is_err());thread.join().unwrap();
}
#[test]fn full_style_recovery_keeps_the_project_openable_and_preserves_extra_receipts(){
    let temp=tempfile::tempdir().unwrap();let mut project=luminacraft::project::Project::default();project.save(temp.path()).unwrap();
    for n in 0..1001{let dir=temp.path().join("ai-style").join(format!("result-{n}"));std::fs::create_dir_all(&dir).unwrap();let record=luminacraft::ai_style::StyleOutput{id:format!("result-{n}"),project_id:project.id.clone(),source_sha256:"0".repeat(64),source_path:temp.path().join("source.png"),catalog_ref:None,edit_sha256:"0".repeat(64),template_name:"fixture".into(),prompt:"fixture".into(),service_name:"fixture".into(),model:"fixture".into(),input:dir.join("input.png"),input_sha256:"0".repeat(64),output:dir.join("output.png"),output_sha256:"0".repeat(64)};std::fs::write(dir.join("result.json"),serde_json::to_vec(&record).unwrap()).unwrap();}
    let results=luminacraft::ai_style::recover_outputs(temp.path(),&project.id).unwrap();assert_eq!(results.len(),1000);assert_eq!(std::fs::read_dir(temp.path().join("ai-style")).unwrap().count(),1001);
    assert_eq!(luminacraft::project::Project::open(temp.path()).unwrap().style_outputs.len(),1000);
}

