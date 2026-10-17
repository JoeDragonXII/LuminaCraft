//! Durable handoff between the main workflow window and the full developer.
use anyhow::{Context, Result, ensure};
use lightcraft_catalog::{PhotoId, Op};
use lightcraft_develop::DevelopSettings;
use serde::{Serialize, Deserialize};
use std::{fs, io::Write, path::{Path,PathBuf}, sync::Arc};
use crate::project::{Project, Photo, hash_file};
#[path="bridge_recovery.rs"] mod recovery;
pub use recovery::*;
#[path="bridge_exports.rs"] mod export_bridge;
pub use export_bridge::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all="snake_case")]
pub enum Purpose { Import, Develop, Library }
#[derive(Clone, Serialize, Deserialize)]
pub struct Entry { pub id: Option<u64>, pub path: PathBuf, pub sha256: String, pub develop: DevelopSettings }
#[derive(Clone, Serialize, Deserialize)]
pub struct Request {
    pub schema: u32, pub token: String, pub library: PathBuf, pub purpose: Purpose,
    pub project: Project, pub project_directory: Option<PathBuf>, pub saved_digest: Option<String>,
    pub entries: Vec<Entry>, pub selected: Vec<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ReturnedPhoto { pub id: u64, pub path: PathBuf, pub sha256: String, pub develop: DevelopSettings, #[serde(default)] pub origin: Option<PathBuf>, #[serde(default,skip_serializing_if="Option::is_none")] pub copy_name:Option<String>, #[serde(default,skip_serializing_if="Option::is_none")] pub cached_denoise:Option<crate::resources::CachedDenoise> }
#[derive(Clone, Serialize, Deserialize)]
pub struct Response {
    pub schema: u32, pub token: String, pub project_id: String, pub library: PathBuf,
    pub photos: Vec<ReturnedPhoto>, pub selected: Vec<u64>, pub error: Option<String>,
    pub exports: Option<serde_json::Value>,
    #[serde(default,skip_serializing_if="Option::is_none")] pub browse:Option<BrowseReturn>,
}
#[derive(Clone,Serialize,Deserialize)]
pub struct BrowseReturn {pub library:PathBuf,pub selected:Vec<u64>}
pub fn write_new<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    fs::create_dir_all(path.parent().context("交接路径无父目录")?)?;
    let bytes=serde_json::to_vec_pretty(value)?;
    ensure!(bytes.len()<=512*1024*1024,"交接数据超过 512 MiB");
    let temp=path.with_extension(format!("{}.tmp",std::process::id()));
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
    let result=(|| {file.write_all(&bytes)?;file.sync_all()?;drop(file);ensure!(!path.exists(),"交接文件已存在");fs::rename(&temp,path)?;Ok(())})();
    if result.is_err(){let _=fs::remove_file(temp);}result
}
pub fn read<T: for<'a> Deserialize<'a>>(path:&Path)->Result<T> {
    ensure!(fs::metadata(path)?.len()<=512*1024*1024,"交接数据过大");
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
impl Request {
    fn entry_resource(&self,entry:&Entry)->Option<&crate::resources::CachedDenoise>{
        self.project.photos.iter().find(|photo|photo.source.sha256==entry.sha256 && same_path(&photo.source.path,&entry.path)
            && photo.catalog_ref.as_ref().is_none_or(|r|same_path(&r.library,&self.library) && entry.id==Some(r.id)))
            .and_then(|photo|photo.cached_denoise.as_ref())
    }
    pub fn validate(&self)->Result<()> {
        ensure!(self.schema==1 && !self.token.is_empty() && self.token.len()<=128,"不支持的交接格式");
        self.project.validate()?;
        ensure!(self.library.is_absolute(),"交接照片库必须使用绝对路径");Ok(())
    }
    pub fn apply_to(&self, session:&mut lightcraft_engine::Session)->Result<Vec<u64>> {
        self.validate()?;
        ensure!(session.library.as_ref().is_some_and(|l|l.dir==self.library),"完整版打开了错误的照片库");
        // Validate every explicit identity before any import or settings mutation.
        // A missing virtual copy must never fall back to its master by source path.
        for entry in &self.entries {
            ensure!(hash_file(&entry.path)?==entry.sha256,"原片已改变，保留主界面工程");
            if let Some(resource)=self.entry_resource(entry){resource.verify(&entry.sha256)?;}
            if let Some(id)=entry.id {
                let photo=session.catalog.photo(PhotoId(id)).context("工程对应的照片版本已不存在，请重新关联版本")?;
                ensure!(!photo.deleted,"工程对应的照片版本已移出照片库，请先恢复该版本");
                ensure!(crate::library::photo_path(photo).is_some_and(|s|same_path(Path::new(s),&entry.path)),"工程照片版本的来源已改变，请重新关联版本");
            }
        }
        let mut ids=if self.purpose==Purpose::Develop{Vec::new()}else{self.selected.iter().copied().map(PhotoId).filter(|id|session.catalog.photo(*id).is_some()).collect::<Vec<_>>()};
        let mut ops=Vec::new();
        let mut associated=Vec::new();
        for entry in &self.entries {
            ensure!(hash_file(&entry.path)?==entry.sha256,"原片已改变，保留主界面工程");
            let found=entry.id.map(PhotoId).filter(|id|session.catalog.photo(*id).is_some_and(|p|crate::library::photo_path(p).is_some_and(|s|same_path(Path::new(s),&entry.path))))
                .or_else(||session.catalog.photos().find(|p|p.copy_of.is_none() && crate::library::photo_path(p).is_some_and(|s|same_path(Path::new(s),&entry.path))).map(|p|p.id));
            let id=match found {Some(id)=>id,None=> {
                let report=session.execute("library.import",&serde_json::json!({"paths":[entry.path],"mode":"add"}))?;
                let id=report["imported"].as_array().and_then(|v|v.first()).and_then(|v|v.as_u64()).context("无法把工程原片关联到照片库")?;PhotoId(id)
            }};
            ops.push(Op::SetDevelop{id,settings:Arc::new(entry.develop.clone()),label:"来自 LuminaCraft 主界面".into(),edited:None});
            associated.push(id.0);
            if self.purpose==Purpose::Develop && associated.len()==1{ids.push(id);}
        }
        if !ops.is_empty(){session.commit("主界面参数交接",Op::Batch{ops})?;session.persist()?;}
        for (entry,id) in self.entries.iter().zip(&associated){
            if let Some(resource)=self.entry_resource(entry){
                let id=PhotoId(*id);session.media.denoise.set_handoff(id,resource.spec(entry.path.clone()));session.media.forget(id);
            }
        }
        session.execute("library.select",&serde_json::json!({"ids":ids.iter().map(|id|id.0).collect::<Vec<_>>(),"active":ids.first().map(|id|id.0)}))?;
        Ok(associated)
    }
    pub fn response(&self, session:&mut lightcraft_engine::Session, exports:Option<serde_json::Value>)->Result<Response> {
        session.persist()?;
        let current=session.library.as_ref().context("完整版没有打开照片库")?.dir.clone();
        if !same_path(&current,&self.library) {
            let browse=BrowseReturn{library:current,selected:session.selection.ids.iter().map(|id|id.0).collect()};
            // Commit the browsing library, then read submitted edits only from their
            // original catalog. IDs from another library must never enter this project.
            session.open_library(&self.library,false)?;
            session.execute("library.select",&serde_json::json!({"ids":[]}))?;
            let mut response=self.response(session,exports)?;
            response.selected.clear();response.browse=Some(browse);
            return Ok(response);
        }
        // Import/library return refreshes the unbounded catalog, not the editing project.
        let mut ids=if self.purpose==Purpose::Develop{session.selection.ids.clone()}else{Vec::new()};
        if self.purpose==Purpose::Develop {if let Some(id)=session.active(){if !ids.contains(&id){ids.push(id);}}}
        // Also return the submitted project entries even when the selection has changed.
        for entry in &self.entries {
            let record=match entry.id{Some(id)=>session.catalog.photo(PhotoId(id)),None=>session.catalog.photos().find(|p|p.copy_of.is_none() && crate::library::photo_path(p).is_some_and(|s|same_path(Path::new(s),&entry.path)))};
            if let Some(p)=record {
                if !ids.contains(&p.id){ids.push(p.id);}
            }
        }
        let mut photos=Vec::new();
        for id in &ids {
            let Some(p)=session.catalog.photo(*id).filter(|p|!p.deleted) else{continue};
            let Some(path)=crate::library::photo_path(p).map(PathBuf::from) else{continue};
            if !path.is_file(){continue;}
            let sha256=hash_file(&path)?;
            let cached_denoise=crate::resources::capture(session,*id,&sha256)?.or(crate::resources::latest(&self.library,id.0,&sha256)?);
            photos.push(ReturnedPhoto{id:id.0,sha256,path,develop:(*p.develop).clone(),origin:self.entries.iter().find(|e|e.id==Some(id.0)).map(|e|e.path.clone()),copy_name:p.copy_name.clone(),cached_denoise});
        }
        Ok(Response{schema:1,token:self.token.clone(),project_id:self.project.id.clone(),library:self.library.clone(),photos,
            selected:session.selection.ids.iter().map(|id|id.0).collect(),error:None,exports,browse:None})
    }
}
impl Response {
    pub fn validate_for(&self, req:&Request)->Result<()> {
        ensure!(self.schema==1 && self.token==req.token && self.project_id==req.project.id && self.library==req.library,"返回数据不属于本次工程交接");
        if let Some(browse)=&self.browse{ensure!(browse.library.is_absolute(),"切换照片库返回数据无效");}
        if let Some(error)=&self.error{anyhow::bail!("完整版未完成交接：{error}");}Ok(())
    }
    /// Update a clone first: failed identities or parameters never partly change the project.
    pub fn merge_existing(&self, req:&Request, project:&Project)->Result<Project> {
        self.validate_for(req)?;ensure!(project.id==self.project_id,"主界面工程已经切换");
        let mut next=project.clone();
        for photo in &mut next.photos {
            if photo.catalog_ref.as_ref().is_some_and(|r|!same_path(&r.library,&self.library)){continue;}
            let entry=req.entries.iter().find(|e|e.sha256==photo.source.sha256 && same_path(&e.path,&photo.source.path)
                && (photo.catalog_ref.as_ref().is_some_and(|c|e.id==Some(c.id)) || (photo.catalog_ref.is_none() && !req.project.photos.iter().any(|p|p.catalog_ref.as_ref().is_some_and(|c|e.id==Some(c.id))))));
            let exact=photo.catalog_ref.as_ref().filter(|r|same_path(&r.library,&self.library)).map(|r|r.id).or_else(||entry.and_then(|e|e.id));
            let returned=if let Some(id)=exact{self.photos.iter().find(|r|r.id==id)}else{self.photos.iter().find(|r|r.copy_name.is_none() && r.sha256==photo.source.sha256 && (same_path(&r.path,&photo.source.path) || r.origin.as_ref().is_some_and(|p|same_path(p,&photo.source.path))))};
            if let Some(r)=returned {
                ensure!(r.sha256==photo.source.sha256 && hash_file(&r.path)?==r.sha256,"返回原片身份不符");
                photo.source.path=r.path.clone();
                if let Some(old)=&photo.source.xmp_path {if !old.exists() {
                    let relocated=lightcraft_engine::sidecar::find_sidecar(&r.path.to_string_lossy(),lightcraft_engine::sidecar::SidecarNaming::Stem);
                    let moved=relocated.context("移动后没有找到原 XMP，保留恢复文件")?;
                    ensure!(photo.source.xmp_sha256.as_ref()==Some(&hash_file(&moved)?),"移动后的原 XMP 内容不符");photo.source.xmp_path=Some(moved);
                }}
                photo.edit.develop=r.develop.clone();
                if let Some(cached)=&r.cached_denoise{cached.validate(&photo.source.sha256)?;photo.cached_denoise=Some(cached.clone());}
                photo.catalog_ref=Some(crate::project::CatalogRef{library:self.library.clone(),id:r.id,copy_name:r.copy_name.clone()});
            }
        }
        if let Some(exports)=&self.exports{merge_export_receipts(&mut next,exports)?;}
        next.validate()?;Ok(next)
    }
    pub fn apply_imported(&self, photo:&mut Photo) {
        if let Some(r)=self.photos.iter().find(|r|r.sha256==photo.source.sha256 && same_path(&r.path,&photo.source.path)){
            photo.baseline=r.develop.clone();photo.edit.develop=r.develop.clone();
            photo.cached_denoise=r.cached_denoise.clone();
            photo.catalog_ref=Some(crate::project::CatalogRef{library:self.library.clone(),id:r.id,copy_name:r.copy_name.clone()});
        }
    }
    pub fn import_queue(&self,project:&Project)->Result<Vec<ReturnedPhoto>> {
        let queue:Vec<_>=self.photos.iter().filter(|r|self.selected.contains(&r.id) && !project.photos.iter().any(|p|p.catalog_ref.as_ref().is_some_and(|c|c.id==r.id && same_path(&c.library,&self.library)) || (p.catalog_ref.is_none() && r.copy_name.is_none() && p.source.sha256==r.sha256 && same_path(&p.source.path,&r.path)))).cloned().collect();
        Ok(queue)
    }
}
pub fn bind_imported(photo:&mut Photo,returned:&ReturnedPhoto,library:&Path)->Result<()> {
    ensure!(same_path(&photo.source.path,&returned.path) && (returned.sha256.is_empty() || photo.source.sha256==returned.sha256),"载入的原片与所选版本不一致");
    photo.baseline=returned.develop.clone();photo.edit.develop=returned.develop.clone();
    photo.cached_denoise=returned.cached_denoise.clone().or(crate::resources::latest(library,returned.id,&photo.source.sha256)?);
    if let Some(cached)=&photo.cached_denoise{cached.validate(&photo.source.sha256)?;}
    photo.catalog_ref=Some(crate::project::CatalogRef{library:library.to_path_buf(),id:returned.id,copy_name:returned.copy_name.clone()});
    crate::project::validate_edit(&photo.edit)?;Ok(())
}
pub fn same_path(a:&Path,b:&Path)->bool {
    a.canonicalize().unwrap_or_else(|_|a.to_path_buf()).to_string_lossy().replace('\\',"/").to_lowercase()==b.canonicalize().unwrap_or_else(|_|b.to_path_buf()).to_string_lossy().replace('\\',"/").to_lowercase()
}
