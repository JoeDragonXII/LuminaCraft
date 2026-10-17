use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiRequest {pub input: PathBuf, pub prompt: String, pub parameters: serde_json::Value}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiProgress {pub id:String,pub fraction:f32,pub message:String}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiResult {pub id:String,pub output:PathBuf,pub provider:String,pub is_test:bool}
pub trait AiImageProvider: Send {
    fn capabilities(&self) -> Vec<String>;
    fn submit(&mut self, request: AiRequest) -> Result<String>;
    fn progress(&self, id: &str) -> Result<AiProgress>;
    fn cancel(&mut self, id: &str) -> Result<()>;
    fn result(&self, id: &str) -> Result<AiResult>;
}
pub struct UnconfiguredProvider;
impl AiImageProvider for UnconfiguredProvider {
    fn capabilities(&self)->Vec<String>{vec![]}
    fn submit(&mut self,_:AiRequest)->Result<String>{bail!("AI 服务尚未配置；本阶段不会上传照片。")}
    fn progress(&self,_:&str)->Result<AiProgress>{bail!("AI 服务尚未配置。")}
    fn cancel(&mut self,_:&str)->Result<()>{Ok(())}
    fn result(&self,_:&str)->Result<AiResult>{bail!("AI 服务尚未配置。")}
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Mock {cancelled:bool}
    impl AiImageProvider for Mock {
        fn capabilities(&self)->Vec<String>{vec!["test-only".into()]}
        fn submit(&mut self,r:AiRequest)->Result<String>{if !r.input.is_file(){bail!("input missing");}Ok("job-1".into())}
        fn progress(&self,id:&str)->Result<AiProgress>{Ok(AiProgress{id:id.into(),fraction:if self.cancelled{0.0}else{1.0},message:"test".into()})}
        fn cancel(&mut self,_:&str)->Result<()>{self.cancelled=true;Ok(())}
        fn result(&self,id:&str)->Result<AiResult>{if self.cancelled{bail!("cancelled");}Ok(AiResult{id:id.into(),output:"test-output.png".into(),provider:"mock".into(),is_test:true})}
    }
    #[test] fn provider_contract_cancel_and_unconfigured_errors(){let d=tempfile::tempdir().unwrap();let p=d.path().join("input.png");std::fs::write(&p,b"test").unwrap();let mut m=Mock{cancelled:false};let id=m.submit(AiRequest{input:p,prompt:"test".into(),parameters:serde_json::json!({})}).unwrap();assert_eq!(m.progress(&id).unwrap().fraction,1.0);assert!(m.result(&id).unwrap().is_test);m.cancel(&id).unwrap();assert!(m.result(&id).is_err());assert!(UnconfiguredProvider.submit(AiRequest{input:PathBuf::new(),prompt:String::new(),parameters:serde_json::Value::Null}).is_err());}
}
