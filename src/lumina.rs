use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value,json};
use std::{path::Path,time::Duration};

pub struct LuminaClient {pub base:String, cookie:String, agent:ureq::Agent}
pub fn normalize_base(base:&str)->Result<String>{
    let url=url::Url::parse(base.trim()).context("Lumina 地址无效")?;
    ensure!(url.scheme()=="http"&&matches!(url.host_str(),Some("127.0.0.1"|"localhost")),"仅连接本机 Lumina");
    ensure!(url.username().is_empty()&&url.password().is_none()&&url.path()=="/"&&url.query().is_none()&&url.fragment().is_none(),"Lumina 地址不能包含路径、身份信息或查询参数");
    Ok(url.origin().ascii_serialization())
}
impl LuminaClient {
    pub fn connect(base:&str,password:&str)->Result<Self>{
        let base=normalize_base(base)?;
        let agent:ureq::Agent=ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).max_redirects(0).build().into();
        let response=agent.post(format!("{base}/api/studio/auth/login")).header("Origin",&base).send_json(json!({"password":password})).context("Lumina 登录失败")?;
        ensure!(response.status().is_success(),"Lumina 登录未成功，未跟随重定向");
        let cookie=response.headers().get("set-cookie").and_then(|v|v.to_str().ok()).and_then(|s|s.split(';').next()).unwrap_or("").to_owned();
        ensure!(!cookie.is_empty(),"Lumina 登录未返回会话，连接未建立");
        Ok(Self{base,cookie,agent})
    }
    fn json(&self,path:&str,body:Value)->Result<Value>{
        Ok(self.agent.post(format!("{}{path}",self.base)).header("Origin",&self.base).header("Cookie",&self.cookie).send_json(body)?.body_mut().read_json()?)
    }
    pub fn upload(&self,path:&Path)->Result<String>{
        ensure!(std::fs::metadata(path)?.len()<=200*1024*1024,"成片超过 200MB。");
        let data=std::fs::read(path)?;ensure!(data.len()<=200*1024*1024,"成片超过 200MB。");
        ensure!(data.starts_with(&[0xff,0xd8,0xff]),"发送的成片必须是有效 JPEG。");
        let boundary="LuminaCraftGeneratedPhotoBoundary";
        let mut body=format!("--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"luminacraft.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n").into_bytes();
        body.extend(data);body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
        let payload:Value=self.agent.post(format!("{}/api/studio/assets",self.base)).header("Origin",&self.base).header("Cookie",&self.cookie).header("Content-Type",format!("multipart/form-data; boundary={boundary}")).send(&body)?.body_mut().read_json()?;
        payload["assets"][0]["asset"]["id"].as_str().map(str::to_owned).ok_or_else(||anyhow::anyhow!("上传结果不明确，请重试；Lumina 会按内容去重。"))
    }
    pub fn collections(&self)->Result<Vec<Value>>{
        let payload:Value=self.agent.get(format!("{}/api/studio/collections/summaries",self.base)).header("Cookie",&self.cookie).call()?.body_mut().read_json()?;
        payload["items"].as_array().cloned().ok_or_else(||anyhow::anyhow!("图集列表格式无效。"))
    }
    pub fn add_to_collection(&self,id:&str,asset:&str)->Result<()> {
        ensure!(id.chars().all(|c|c.is_ascii_alphanumeric()||c=='-'),"图集 ID 无效。");
        self.json(&format!("/api/studio/collections/{id}/photos"),json!({"assetIds":[asset]}))?;Ok(())
    }
    pub fn create_draft(&self,title:&str,asset:&str)->Result<String>{
        let response=self.json("/api/studio/collections",json!({"title":title,"category":"摄影作品","visibility":"draft","assetIds":[asset]}))?;
        if let Some(id)=response["collection"]["id"].as_str(){Ok(id.into())}else{bail!("图集创建结果不明确，请检查 Lumina 后重试。")}
    }
}
