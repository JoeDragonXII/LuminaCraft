use super::*;
use lightcraft_catalog::Catalog;

/// Held shared by participating windows; exclusive acquisition protects recovery.
pub struct HandoffLease { file:fs::File }
impl HandoffLease {
    pub fn participate(request:&Path)->Result<Self> {
        let file=lease_file(request)?;file.try_lock_shared().context("交接正在恢复，请稍后重试")?;Ok(Self{file})
    }
    pub fn recover(request:&Path)->Result<Self> {
        let file=lease_file(request)?;file.try_lock().context("仍有窗口参与交接，不能恢复或覆盖其状态")?;Ok(Self{file})
    }
    pub fn upgrade(&self)->Result<()> {
        self.file.unlock()?;
        match self.file.try_lock(){Ok(())=>Ok(()),Err(e)=>{self.file.lock_shared()?;Err(anyhow::anyhow!("等待完整版完成退出或重启：{e}"))}}
    }
}
impl Drop for HandoffLease {fn drop(&mut self){let _=self.file.unlock();}}
fn lease_file(request:&Path)->Result<fs::File> {
    Ok(fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(request.with_file_name("handoff.lock"))?)
}

#[derive(Serialize,Deserialize)]
pub struct Checkpoint {pub token:String,pub project:Project,pub saved_digest:Option<String>,pub directory:Option<PathBuf>}

/// No deletion: successful states are immutable recovery material.
pub fn checkpoint(request_path:&Path,request:&Request,project:&Project,directory:Option<PathBuf>)->Result<()> {
    project.validate()?;
    let target=request_path.with_file_name("project-return.json");
    let saved=Checkpoint{token:request.token.clone(),project:project.clone(),saved_digest:project.saved_digest.clone(),directory};
    if target.exists(){
        let old:Checkpoint=read(&target)?;
        ensure!(old.token==saved.token && old.project.digest()?==saved.project.digest()?,"已存在不同的返回工程检查点");
    } else {write_new(&target,&saved)?;}
    let receipt=request_path.with_file_name("applied.json");
    if !receipt.exists(){write_new(&receipt,&serde_json::json!({"token":request.token,"projectDigest":project.digest()?}))?;}
    Ok(())
}

fn pending_local(library:&Path)->Result<Vec<PathBuf>> {
    let root=library.join("Bridge");if !root.exists(){return Ok(vec![]);}
    let mut found=Vec::new();
    for entry in fs::read_dir(root)?.take(10_000) {
        let entry=entry?;if !entry.file_type()?.is_dir(){continue;}
        let request=entry.path().join("request.json");
        if !request.is_file() || entry.path().join("dismissed.json").exists() || entry.path().join("project-saved.json").exists(){continue;}
        if !entry.path().join("applied.json").exists(){found.push(request);continue;}
        let checkpoint=entry.path().join("project-return.json");
        if checkpoint.exists(){
            let saved:Checkpoint=read(&checkpoint)?;
            if saved.saved_digest.as_ref()!=Some(&saved.project.digest()?){found.push(request);}
        }
    }
    found.sort_by_key(|p|fs::metadata(p).and_then(|m|m.modified()).ok());found.reverse();Ok(found)
}
#[derive(Serialize,Deserialize)]
pub struct BrowseCheckpoint {pub token:String,pub request:PathBuf,pub browse:BrowseReturn}
pub fn record_browse(path:&Path,request:&Request,browse:&BrowseReturn)->Result<()> {
    ensure!(request.token.chars().all(|c|c.is_ascii_alphanumeric() || c=='-' || c=='_'),"浏览记录 token 无效");
    ensure!(browse.library.is_absolute(),"浏览库状态无效");
    let state=BrowseCheckpoint{token:request.token.clone(),request:path.to_path_buf(),browse:browse.clone()};
    let directory=path.with_file_name("Browse");fs::create_dir_all(&directory)?;
    let target=(0..10000).map(|n|directory.join(format!("{n:05}.json"))).find(|p|!p.exists()).context("浏览库恢复记录已满")?;
    write_new(&target,&state)?;
    if !same_path(&browse.library,&request.library){
        let link=browse.library.join("BridgeLinks").join(format!("{}.json",request.token));
        if !link.exists(){write_new(&link,&state)?;}
    }
    Ok(())
}
pub fn latest_browse(path:&Path,request:&Request)->Result<Option<BrowseReturn>> {
    let dir=path.with_file_name("Browse");if !dir.exists(){return Ok(None);}
    let mut files=fs::read_dir(dir)?.map(|e|e.map(|e|e.path())).collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|p|p.extension().is_some_and(|e|e=="json"));files.sort();
    let Some(last)=files.last() else{return Ok(None)};let state:BrowseCheckpoint=read(last)?;
    ensure!(state.token==request.token && same_path(&state.request,path) && state.browse.library.is_absolute(),"浏览恢复记录不属于该交接");
    Ok((!same_path(&state.browse.library,&request.library)).then_some(state.browse))
}
pub fn pending(library:&Path)->Result<Vec<PathBuf>> {
    let mut found=pending_local(library)?;let links=library.join("BridgeLinks");
    if links.exists(){for entry in fs::read_dir(links)?.take(10000){
        let path=entry?.path();if path.extension().is_none_or(|e|e!="json"){continue;}
        let state:BrowseCheckpoint=read(&path)?;
        ensure!(same_path(&state.browse.library,library),"恢复链接属于其他照片库");
        if !state.request.exists(){continue;}
        let request:Request=read(&state.request)?;request.validate()?;ensure!(request.token==state.token,"恢复链接身份不符");
        if pending_local(&request.library)?.contains(&state.request) && !found.contains(&state.request){found.push(state.request);}
    }}
    Ok(found)
}
pub fn live_handoff(library:&Path)->Result<bool> {
    for request in pending(library)? {
        if HandoffLease::recover(&request).is_err(){return Ok(true);}
    }
    Ok(false)
}

pub fn resolved_request(path:&Path)->Result<Request> {
    let original:Request=read(path)?;original.validate()?;
    let resolved=path.with_file_name("resolved-request.json");
    if !resolved.exists(){return Ok(original);}
    let bound:Request=read(&resolved)?;bound.validate()?;
    ensure!(bound.token==original.token && bound.library==original.library && bound.project.digest()?==original.project.digest()? && bound.entries.len()==original.entries.len(),"交接绑定不匹配");
    for (a,b) in original.entries.iter().zip(&bound.entries){ensure!(a.path==b.path && a.sha256==b.sha256 && a.develop==b.develop,"交接绑定改变了源参数");}
    Ok(bound)
}

/// Called only while the receiving window owns the catalog and the recovery lease.
pub fn reconstruct(request:&Request,catalog:&Catalog)->Result<Response> {
    let mut photos=Vec::new();
    for entry in &request.entries {
        let found=match entry.id{Some(id)=>catalog.photo(PhotoId(id)),None=>catalog.photos().find(|p|p.copy_of.is_none() && crate::library::photo_path(p).is_some_and(|s|same_path(Path::new(s),&entry.path)))}.filter(|p|!p.deleted);
        let Some(photo)=found else {continue};
        let Some(path)=crate::library::photo_path(photo).map(PathBuf::from) else{continue};
        ensure!(hash_file(&path)?==entry.sha256,"恢复原片已改变，保留恢复材料");
        photos.push(ReturnedPhoto{id:photo.id.0,path,sha256:entry.sha256.clone(),develop:(*photo.develop).clone(),origin:Some(entry.path.clone()),copy_name:photo.copy_name.clone(),cached_denoise:crate::resources::latest(&request.library,photo.id.0,&entry.sha256)?});
    }
    Ok(Response{schema:1,token:request.token.clone(),project_id:request.project.id.clone(),library:request.library.clone(),photos,selected:request.selected.clone(),error:None,exports:None,browse:None})
}

pub fn recover_project(path:&Path,library:&Path,catalog:&Catalog)->Result<(Request,Response,Project,Option<PathBuf>)> {
    let request=resolved_request(path)?;ensure!(same_path(&request.library,library),"恢复数据属于另一个照片库");
    let result=path.with_file_name("result.json");
    let mut response=if result.exists(){read::<Response>(&result)?}else if path.with_file_name("resolved-request.json").exists(){reconstruct(&request,catalog)?}else{
        // Never replace unsaved main-window edits with older catalog values when startup failed.
        Response{schema:1,token:request.token.clone(),project_id:request.project.id.clone(),library:request.library.clone(),photos:vec![],selected:request.selected.clone(),error:None,exports:None,browse:None}
    };
    if response.exports.is_none(){response.exports=Some(export_batches(path)?);}
    if !result.exists(){response.browse=latest_browse(path,&request)?;}
    response.validate_for(&request)?;
    let returned=path.with_file_name("project-return.json");
    let (mut project,directory)=if returned.exists(){
        let saved:Checkpoint=read(&returned)?;ensure!(saved.token==request.token && saved.project.id==request.project.id,"工程检查点不匹配");
        let mut project=saved.project;project.saved_digest=saved.saved_digest;(project,saved.directory)
    }else {
        let mut source=request.project.clone();source.saved_digest=request.saved_digest.clone();
        (response.merge_existing(&request,&source)?,request.project_directory.clone())
    };
    project.saved_digest=project.saved_digest.or_else(||request.saved_digest.clone());project.validate()?;
    for photo in &project.photos{photo.source.verify()?;}
    Ok((request,response,project,directory))
}
