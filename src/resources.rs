//! Durable copies of advanced decoder resources, independent of disposable model caches.
use anyhow::{Context,Result,ensure};
use serde::{Serialize,Deserialize};
use std::{fs,io::Write,path::{Path,PathBuf}};
use crate::project::hash_file;
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
pub struct CachedDenoise {pub path:PathBuf,pub key:String,pub sha256:String,pub source_sha256:String}
impl CachedDenoise {
    pub fn spec(&self,source:PathBuf)->lightcraft_engine::media::DenoiseSpec {
        let record=self.clone();
        lightcraft_engine::media::DenoiseSpec{product:self.path.clone(),key:self.key.clone(),make:Some(std::sync::Arc::new(move||{
            record.verify(&record.source_sha256).map_err(|e|e.to_string())?;
            if hash_file(&source).map_err(|e|e.to_string())?!=record.source_sha256{return Err("原片已改变，不能使用旧降噪资源".into());}Ok(true)
        }))}
    }
    pub fn validate(&self,source:&str)->Result<()> {
        ensure!(self.path.is_absolute() && !self.key.is_empty() && self.key.len()<=512,"降噪资源描述无效");
        ensure!(self.source_sha256==source && self.sha256.len()==64 && self.sha256.bytes().all(|b|b.is_ascii_hexdigit()),"降噪资源与原片身份不符");Ok(())
    }
    pub fn verify(&self,source:&str)->Result<()> {
        self.validate(source)?;
        ensure!(fs::metadata(&self.path).context("降噪缓存缺失，请在完整版重新生成")?.len()<=2*1024*1024*1024,"降噪缓存过大");
        ensure!(hash_file(&self.path)?==self.sha256,"降噪缓存已改变，请在完整版重新生成");Ok(())
    }
}
pub fn capture(session:&lightcraft_engine::Session,id:lightcraft_catalog::PhotoId,source:&str)->Result<Option<CachedDenoise>> {
    let Some(spec)=session.media.denoise.spec(id) else{return Ok(None)};
    let library=&session.library.as_ref().context("降噪资源没有所属照片库")?.dir;
    ensure!(fs::metadata(&spec.product)?.len()<=2*1024*1024*1024,"降噪缓存过大");
    let sha=hash_file(&spec.product)?;let directory=library.join("BridgeResources/Denoise");fs::create_dir_all(&directory)?;
    let path=directory.join(format!("{sha}.lcdn"));
    if !path.exists(){
        let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let temporary=directory.join(format!("{}.{}.{nonce}.tmp",sha,std::process::id()));
        let mut output=fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        let result=(||->Result<()>{std::io::copy(&mut fs::File::open(&spec.product)?,&mut output)?;output.flush()?;output.sync_all()?;
            ensure!(hash_file(&temporary)?==sha,"复制期间降噪缓存发生变化");
            // A hard link publishes atomically without replacing another writer's file.
            match fs::hard_link(&temporary,&path){Ok(())=>{},Err(e) if path.exists()=>{ensure!(hash_file(&path)?==sha,"已有降噪资源损坏：{e}");},Err(e)=>return Err(e.into())};Ok(())})();
        drop(output);let _=fs::remove_file(&temporary);result?;
    }
    let record=CachedDenoise{path,key:spec.key.clone(),sha256:sha,source_sha256:source.into()};record.verify(source)?;
    let index=library.join("BridgeResources/DenoiseIndex").join(id.0.to_string());fs::create_dir_all(&index)?;
    if latest(library,id.0,source)?.as_ref()!=Some(&record){
        let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        crate::bridge::write_new(&index.join(format!("{nonce:039}.json")),&record)?;
    }
    Ok(Some(record))
}
pub fn latest(library:&Path,id:u64,source:&str)->Result<Option<CachedDenoise>> {
    let dir=library.join("BridgeResources/DenoiseIndex").join(id.to_string());if !dir.exists(){return Ok(None)};
    let mut paths=fs::read_dir(dir)?.take(10000).map(|e|e.map(|e|e.path())).collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|p|p.extension().is_some_and(|e|e=="json"));paths.sort();
    for path in paths.into_iter().rev(){let record:CachedDenoise=crate::bridge::read(&path)?;if record.source_sha256==source{record.validate(source)?;return Ok(Some(record));}}
    Ok(None)
}
