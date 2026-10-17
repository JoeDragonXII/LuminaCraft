//! Saved prompt templates and the OpenAI Images edit adapter. No keys in serializable state.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{Cursor, Write}, path::{Path, PathBuf}, sync::{Arc, mpsc, atomic::{AtomicBool, Ordering}}, time::Duration};
use crate::{engine, project::{Photo, CatalogRef, hash_file, photo_edit_hash}};

pub fn new_id() -> String {
    format!("ai-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos())
}
fn valid_id(id:&str)->bool { !id.is_empty() && id.len()<=100 && id.bytes().all(|c|c.is_ascii_alphanumeric() || c==b'-') }
pub fn config_dir()->PathBuf {
    std::env::var_os("LUMINACRAFT_AI_DIR").map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("LuminaCraft").join("ai")
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Service {pub id:String,pub name:String,pub base_url:String,pub model:String}
impl Default for Service {
    fn default()->Self{Self{id:new_id(),name:"OpenAI".into(),base_url:"https://api.openai.com/v1".into(),model:"gpt-image-2.5-sunburst".into()}}
}
impl Service {
    pub fn credential_id(&self)->String{format!("{}-{}",self.id,digest(self.base_url.trim().trim_end_matches('/').as_bytes()))}
    pub fn validate(&self)->Result<()> {
        ensure!(valid_id(&self.id) && !self.name.trim().is_empty() && self.name.len()<=128,"服务名称无效");
        ensure!(!self.model.trim().is_empty() && self.model.len()<=128 && !self.model.chars().any(char::is_control),"请填写图片模型 ID");
        endpoint(&self.base_url,"images/edits")?;Ok(())
    }
}
fn endpoint(base:&str,route:&str)->Result<String>{
    let url=url::Url::parse(base.trim()).context("API 地址无效")?;
    let local=matches!(url.host_str(),Some("127.0.0.1"|"localhost"|"[::1]"));
    ensure!(url.scheme()=="https" || (url.scheme()=="http" && local),"API 地址需要 HTTPS（本机调试可用 HTTP）");
    ensure!(url.host_str().is_some() && url.username().is_empty() && url.password().is_none() && url.query().is_none() && url.fragment().is_none(),"地址不能包含密钥、用户名或查询参数");
    Ok(format!("{}/{route}",url.as_str().trim_end_matches('/')))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Template {pub id:String,pub name:String,pub prompt:String,pub service_id:Option<String>}
impl Default for Template {fn default()->Self{Self{id:new_id(),name:"新风格".into(),prompt:String::new(),service_id:None}}}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {pub services:Vec<Service>,pub templates:Vec<Template>,pub default_service:Option<String>}
impl Settings {
    pub fn validate(&self)->Result<()> {
        ensure!(self.services.len()<=40 && self.templates.len()<=200,"配置数量超过限制");
        let mut ids=std::collections::HashSet::new();
        for s in &self.services{s.validate()?;ensure!(ids.insert(&s.id),"服务 ID 重复");}
        ensure!(self.default_service.as_ref().is_none_or(|id|self.services.iter().any(|s|&s.id==id)),"默认服务不存在");
        ids.clear();
        for t in &self.templates {
            ensure!(t.name!=crate::map_style::NAME && t.id!=crate::map_style::ID && valid_id(&t.id) && ids.insert(&t.id) && !t.name.trim().is_empty() && t.name.len()<=128,"风格名称或 ID 无效");
            validate_prompt(&t.prompt)?;
            ensure!(t.service_id.as_ref().is_none_or(|id|self.services.iter().any(|s|&s.id==id)),"风格绑定的服务不存在");
        }Ok(())
    }
    pub fn load(dir:&Path)->Result<Self>{
        let file=dir.join("settings.json");if !file.exists(){return Ok(Self::default());}
        ensure!(fs::metadata(&file)?.len()<=8*1024*1024,"AI 配置文件过大");
        let settings:Self=serde_json::from_slice(&fs::read(file)?)?;settings.validate()?;Ok(settings)
    }
    pub fn save(&self,dir:&Path)->Result<()> {
        self.validate()?;fs::create_dir_all(dir)?;
        let temp=dir.join(format!("{}.tmp",new_id()));write_new(&temp,&serde_json::to_vec_pretty(self)?)?;
        let result=fs::rename(&temp,dir.join("settings.json"));if result.is_err(){let _=fs::remove_file(temp);}result?;Ok(())
    }
    pub fn service_for(&self,t:&Template)->Option<&Service>{
        t.service_id.as_ref().or(self.default_service.as_ref()).and_then(|id|self.services.iter().find(|s|&s.id==id))
    }
    /// Roll back credential writes if a later write or settings commit fails.
    pub fn save_with_secrets(&self,dir:&Path,old:&Settings,keys:&std::collections::HashMap<String,String>,secrets:&impl Secrets)->Result<Vec<String>>{
        self.validate()?;
        let mut changes=vec![];
        for service in &self.services{
            let key=keys.get(&service.id).filter(|k|!k.is_empty());
            let changed=old.services.iter().find(|s|s.id==service.id).is_some_and(|s|s.base_url!=service.base_url);
            ensure!(!changed||key.is_some(),"修改服务地址后请重新输入 API 密钥");
            if let Some(key)=key{
                ensure!(!key.trim().is_empty()&&key.len()<=4096&&!key.chars().any(char::is_control),"API 密钥无效");
                let id=service.credential_id();let previous=secrets.get_optional(&id)?;
                changes.push((id,key,previous));
            }
        }
        let mut attempted=0;
        let committed=(||->Result<()>{for(id,key,_)in &changes{attempted+=1;secrets.set(id,key)?;}self.save(dir)})();
        if let Err(error)=committed{
            let mut rollback_failed=false;
            for(id,_,previous)in changes[..attempted].iter().rev(){
                let result=match previous{Some(key)=>secrets.set(id,key),None=>secrets.delete(id)};
                rollback_failed|=result.is_err();
            }
            if rollback_failed{bail!("配置保存失败；部分凭据回滚失败，请重新保存密钥。原配置文件保留");}
            return Err(error);
        }
        let mut warnings=vec![];
        for service in &old.services{
            if !self.services.iter().any(|s|s.credential_id()==service.credential_id()){
                if secrets.delete(&service.credential_id()).is_err(){warnings.push("配置已保存，旧凭据清理失败".into());}
            }
        }
        Ok(warnings)
    }
}
pub fn validate_prompt(prompt:&str)->Result<()>{ensure!(!prompt.trim().is_empty() && prompt.len()<=32000,"提示词不能为空，且不能超过 32000 字节");Ok(())}

pub trait Secrets {fn get(&self,id:&str)->Result<String>;fn get_optional(&self,id:&str)->Result<Option<String>>{self.get(id).map(Some)}fn set(&self,id:&str,key:&str)->Result<()>;fn delete(&self,id:&str)->Result<()>;}
pub struct WindowsSecrets;
impl Secrets for WindowsSecrets {
    fn get_optional(&self,id:&str)->Result<Option<String>>{
        match keyring::Entry::new("LuminaCraft.OpenAI",id)?.get_password(){Ok(key)=>Ok(Some(key)),Err(keyring::Error::NoEntry)=>Ok(None),Err(_)=>bail!("Windows 凭据读取失败，配置未修改")}
    }
    fn get(&self,id:&str)->Result<String>{
        keyring::Entry::new("LuminaCraft.OpenAI",id).and_then(|e|e.get_password()).map_err(|_|anyhow::anyhow!("未找到此服务的 API 密钥，请在 AI 设置中保存密钥"))
    }
    fn set(&self,id:&str,key:&str)->Result<()>{
        ensure!(!key.trim().is_empty() && key.len()<=4096 && !key.chars().any(char::is_control),"API 密钥无效");
        keyring::Entry::new("LuminaCraft.OpenAI",id).and_then(|e|e.set_password(key.trim())).map_err(|_|anyhow::anyhow!("Windows 凭据存储写入失败，密钥未保存"))
    }
    fn delete(&self,id:&str)->Result<()>{
        match keyring::Entry::new("LuminaCraft.OpenAI",id)?.delete_credential(){Ok(())|Err(keyring::Error::NoEntry)=>Ok(()),Err(_)=>bail!("旧密钥未能从 Windows 凭据存储移除")}
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StyleOutput {
    pub id:String,pub project_id:String,pub source_sha256:String,pub source_path:PathBuf,pub catalog_ref:Option<CatalogRef>,
    pub edit_sha256:String,pub template_name:String,pub prompt:String,pub service_name:String,pub model:String,
    pub input:PathBuf,pub input_sha256:String,pub output:PathBuf,pub output_sha256:String,
}
impl StyleOutput {
    pub fn matches(&self,photo:&Photo)->bool {
        self.source_sha256==photo.source.sha256 && match (&self.catalog_ref,&photo.catalog_ref) {
            (Some(a),Some(b))=>a.id==b.id && crate::bridge::same_path(&a.library,&b.library),(None,None)=>crate::bridge::same_path(&self.source_path,&photo.source.path),_=>false
        }
    }
    pub fn validate(&self)->Result<()> {
        ensure!(valid_id(&self.id) && valid_id(&self.project_id) && [&self.source_sha256,&self.edit_sha256,&self.input_sha256,&self.output_sha256].iter().all(|s|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit())),"风格版本记录无效");
        ensure!(self.input.is_absolute() && self.output.is_absolute() && self.template_name.len()<=128 && self.service_name.len()<=128 && self.model.len()<=128,"风格版本路径或名称无效");
        validate_prompt(&self.prompt)
    }
    pub fn preview(&self,input:bool)->Result<image::RgbaImage>{
        let (path,hash)=if input{(&self.input,&self.input_sha256)}else{(&self.output,&self.output_sha256)};
        ensure!(fs::metadata(path)?.len()<=48*1024*1024,"风格图片超过大小限制");
        ensure!(hash_file(path)?==*hash,"风格图片已改变，不能继续使用");
        let image=decode_png(&fs::read(path)?)?;
        Ok(if image.width()>1800||image.height()>1800{image.thumbnail(1800,1800)}else{image}.into_rgba8())
    }
    pub fn export(&self,path:&Path,jpeg:bool)->Result<()> {
        ensure!(fs::metadata(&self.output)?.len()<=48*1024*1024,"风格图片超过大小限制");
        ensure!(hash_file(&self.output)?==self.output_sha256,"风格结果已改变，不能导出");
        let image=decode_png(&fs::read(&self.output)?)?;
        let mut bytes=Cursor::new(Vec::new());
        if jpeg {image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes,94).encode_image(&image.to_rgb8())?;}
        else {image.write_to(&mut bytes,image::ImageFormat::Png)?;}
        write_new(path,&bytes.into_inner())
    }
}
fn write_new(path:&Path,bytes:&[u8])->Result<()> {
    let mut f=fs::OpenOptions::new().write(true).create_new(true).open(path).context("文件已存在或目录不可写，请选择新文件名")?;
    if let Err(e)=f.write_all(bytes).and_then(|_|f.sync_all()){drop(f);let _=fs::remove_file(path);return Err(e.into());}Ok(())
}
fn decode_png(bytes:&[u8])->Result<image::DynamicImage>{
    ensure!(bytes.len()<=48*1024*1024 && bytes.starts_with(b"\x89PNG\r\n\x1a\n"),"服务未返回有效 PNG 图片");
    let mut reader=image::ImageReader::with_format(Cursor::new(bytes),image::ImageFormat::Png);
    let mut limits=image::Limits::default();limits.max_image_width=Some(8192);limits.max_image_height=Some(8192);limits.max_alloc=Some(256*1024*1024);reader.limits(limits);
    Ok(reader.decode().context("返回图片无法解码")?)
}
fn digest(bytes:&[u8])->String{format!("{:x}",Sha256::digest(bytes))}
pub fn recover_outputs(directory:&Path,project_id:&str)->Result<Vec<StyleOutput>>{
    let dir=directory.join("ai-style");if !dir.exists(){return Ok(vec![]);}
    let mut outputs=vec![];
    for entry in fs::read_dir(dir)?.take(2001){
        if outputs.len()>=1000{log::warn!("风格恢复已达1000项，额外回执保留在原目录");break;}
        let entry=entry?;if !entry.file_type()?.is_dir(){continue;}
        let path=entry.path().join("result.json");if !path.is_file(){continue;}
        let result=(||->Result<StyleOutput>{
            ensure!(fs::metadata(&path)?.len()<=128*1024,"风格回执过大");
            let result:StyleOutput=serde_json::from_slice(&fs::read(&path)?)?;result.validate()?;
            ensure!(entry.file_name().to_str()==Some(result.id.as_str()) && crate::bridge::same_path(&result.input,&entry.path().join("input.png")) && crate::bridge::same_path(&result.output,&entry.path().join("output.png")),"风格回执路径不匹配");
            Ok(result)
        })();
        match result{Ok(result) if result.project_id==project_id=>outputs.push(result),Ok(_)=>{},Err(_)=>log::warn!("忽略损坏的 AI 风格恢复回执，原文件保留：{}",path.display())}
    }
    outputs.sort_by(|a,b|a.id.cmp(&b.id));Ok(outputs)
}

pub struct Generate {pub project_id:String,pub directory:PathBuf,pub photo:Photo,pub service:Service,pub template_name:String,pub prompt:String}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum GenerationStage {Preparing,Generating,Saving}
impl GenerationStage {
    pub fn label(self)->&'static str {match self{Self::Preparing=>"准备当前调色与修饰后的照片…",Self::Generating=>"正在生成，等待服务返回…",Self::Saving=>"校验并保存风格版本…"}}
}
pub enum Event {Stage(GenerationStage),Finished(Result<StyleOutput,String>)}
pub struct Task {pub rx:mpsc::Receiver<Event>,pub cancel:Arc<AtomicBool>}
impl Drop for Task {fn drop(&mut self){self.cancel.store(true,Ordering::Relaxed);}}
pub fn start(request:Generate,ctx:egui::Context)->Task {
    let (tx,rx)=mpsc::channel();let cancel=Arc::new(AtomicBool::new(false));let flag=cancel.clone();
    std::thread::spawn(move||{
        let stage=|s|{let _=tx.send(Event::Stage(s));ctx.request_repaint();};
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||generate(request,&WindowsSecrets,&flag,&stage)))
            .unwrap_or_else(|_|Err(anyhow::anyhow!("AI 后台任务异常；工程和原片已保留"))).map_err(|e|e.to_string());
        let _=tx.send(Event::Finished(result));ctx.request_repaint();
    });Task{rx,cancel}
}
fn check_cancel(cancel:&AtomicBool)->Result<()>{ensure!(!cancel.load(Ordering::Relaxed),"已取消本次任务；服务端可能仍在处理，未自动重试");Ok(())}
fn authorization(key:&str)->Result<ureq::http::HeaderValue>{
    ensure!(!key.trim().is_empty() && key.len()<=4096 && !key.chars().any(char::is_control),"API 密钥无效");
    let mut value=ureq::http::HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_|anyhow::anyhow!("API 密钥无效"))?;
    value.set_sensitive(true);Ok(value)
}
pub fn generate(request:Generate,secrets:&impl Secrets,cancel:&AtomicBool,stage:&impl Fn(GenerationStage))->Result<StyleOutput>{
    let Generate{project_id,directory,photo,service,template_name,mut prompt}=request;
    service.validate()?;if template_name!=crate::map_style::NAME{validate_prompt(&prompt)?;}ensure!(valid_id(&project_id) && template_name.len()<=128,"工程或风格名称无效");
    ensure!(directory.join("project.json").is_file(),"请先保存工程，再生成风格版本");
    let saved:crate::project::Project=serde_json::from_slice(&fs::read(directory.join("project.json"))?)?;
    ensure!(saved.id==project_id,"工程目录已更换，请先重新保存工程");
    check_cancel(cancel)?;let key=secrets.get(&service.credential_id())?;
    stage(GenerationStage::Preparing);photo.source.verify()?;
    let edit_fingerprint=photo_edit_hash(&photo)?;
    let source=engine::Source::load_photo(&photo,&photo.edit.develop,2048)?;engine::prepare_profile(&photo,&photo.edit)?;
    if template_name==crate::map_style::NAME{prompt=crate::map_style::prompt(source.metadata.gps.as_ref(),&prompt)?;}
    let rendered=engine::render_photo(&source,&photo.edit,2048,cancel)?;
    let pixels:Vec<u8>=rendered.image.data.iter().flatten().copied().collect();
    let image=image::RgbaImage::from_raw(rendered.image.width as u32,rendered.image.height as u32,pixels).context("渲染结果无效")?;
    let mut png=Cursor::new(Vec::new());image::DynamicImage::ImageRgba8(image).write_to(&mut png,image::ImageFormat::Png)?;
    let png=png.into_inner();check_cancel(cancel)?;
    ensure!(photo_edit_hash(&photo)?==edit_fingerprint,"准备输入期间 LUT 或元数据发生变化，请重试");
    let id=new_id();let root=directory.canonicalize()?.join("ai-style");fs::create_dir_all(&root)?;
    let folder=root.join(&id);fs::create_dir(&folder)?;let input=folder.join("input.png");write_new(&input,&png)?;
    stage(GenerationStage::Generating);
    let output_bytes=edit_image(&service,&key,&prompt,&png,cancel,Duration::from_secs(300))?;
    check_cancel(cancel)?;stage(GenerationStage::Saving);decode_png(&output_bytes)?;
    let output=folder.join("output.png");write_new(&output,&output_bytes)?;
    let record=StyleOutput{id,project_id,source_sha256:photo.source.sha256.clone(),source_path:photo.source.path.clone(),catalog_ref:photo.catalog_ref.clone(),edit_sha256:edit_fingerprint,template_name,prompt,service_name:service.name,model:service.model,input,input_sha256:digest(&png),output,output_sha256:digest(&output_bytes)};
    record.validate()?;write_new(&folder.join("result.json"),&serde_json::to_vec_pretty(&record)?)?;Ok(record)
}
// One HTTP submission, no automatic retries or redirects (including custom API hosts).
pub fn edit_image(service:&Service,key:&str,prompt:&str,png:&[u8],cancel:&AtomicBool,timeout:Duration)->Result<Vec<u8>>{
    service.validate()?;validate_prompt(prompt)?;check_cancel(cancel)?;
    ensure!(!key.trim().is_empty() && key.len()<=4096 && !key.chars().any(char::is_control),"API 密钥无效");
    let mut boundary=new_id();while prompt.contains(&boundary) || png.windows(boundary.len()).any(|w|w==boundary.as_bytes()){boundary=new_id();}
    let mut body=Vec::new();
    for (name,value) in [("model",service.model.as_str()),("prompt",prompt),("n","1"),("output_format","png"),("size","auto")] {
        write!(&mut body,"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")?;
    }
    write!(&mut body,"--{boundary}\r\nContent-Disposition: form-data; name=\"image[]\"; filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\n")?;
    body.extend_from_slice(png);write!(&mut body,"\r\n--{boundary}--\r\n")?;
    let agent:ureq::Agent=ureq::Agent::config_builder().timeout_global(Some(timeout)).max_redirects(0).http_status_as_error(false).build().into();
    let mut response=agent.post(endpoint(&service.base_url,"images/edits")?).header("Authorization",authorization(key)?).header("Content-Type",format!("multipart/form-data; boundary={boundary}")).send(&body)
        .map_err(|_|anyhow::anyhow!("网络中断或请求超时；服务端可能已收到任务，未自动重试"))?;
    check_cancel(cancel)?;
    match response.status().as_u16(){200=>{},401|403=>bail!("认证失败或没有图片模型权限，请检查 API 密钥和模型"),429=>bail!("服务限流或额度不足，请稍后手动重试"),400=>bail!("图片请求被拒绝，请检查模型、提示词和接口地址"),n=>bail!("图片服务返回 HTTP {n}，未自动重试")}
    let bytes=response.body_mut().with_config().limit(70*1024*1024).read_to_vec().map_err(|_|anyhow::anyhow!("图片响应读取失败或超过限制；未自动重试"))?;
    check_cancel(cancel)?;
    let value:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|anyhow::anyhow!("图片服务返回了无效响应"))?;
    let data=value["data"].as_array().filter(|a|a.len()==1).context("图片服务未返回单张图片")?;
    let encoded=data[0]["b64_json"].as_str().context("服务未返回图片数据，请使用支持图片编辑的 GPT Image 模型")?;
    let decoded=STANDARD.decode(encoded).map_err(|_|anyhow::anyhow!("返回的图片编码无效"))?;decode_png(&decoded)?;Ok(decoded)
}

pub fn test_connection(service:&Service,secrets:&impl Secrets)->Result<()> {
    service.validate()?;let key=secrets.get(&service.credential_id())?;
    let agent:ureq::Agent=ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).max_redirects(0).http_status_as_error(false).build().into();
    let mut response=agent.get(endpoint(&service.base_url,"models")?).header("Authorization",authorization(&key)?).call().map_err(|_|anyhow::anyhow!("连接失败，请检查网络与 API 地址"))?;
    ensure!(response.status().is_success(),"连接失败（HTTP {}），请检查密钥与访问权限",response.status().as_u16());
    let bytes=response.body_mut().with_config().limit(4*1024*1024).read_to_vec().map_err(|_|anyhow::anyhow!("模型列表读取失败"))?;
    let value:serde_json::Value=serde_json::from_slice(&bytes).context("模型列表格式无效")?;
    ensure!(value["data"].as_array().is_some_and(|a|a.iter().any(|v|v["id"].as_str()==Some(&service.model))),"连接成功，但列表中未找到此模型；请确认模型 ID 和权限");Ok(())
}
