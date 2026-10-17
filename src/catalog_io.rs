//! Native LightCraft directory export. Never writes source photos or the source library.
use anyhow::{Context, Result, ensure};
use lightcraft_catalog::{Catalog, FsStore, Journal, PhotoId};
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, HashSet}, fs, io::{Read, Write}, path::{Component, Path, PathBuf}, sync::atomic::{AtomicBool, Ordering}};
use crate::project::{Project, hash_file};

pub const MARKER: &str = "catalog.lccatalog";
#[derive(Clone)]
pub struct Export {
    pub catalog: Catalog,
    pub library: PathBuf,
    pub project: Option<Project>,
    pub selected: Option<HashSet<PhotoId>>,
    pub copy_originals: bool,
}
#[derive(Serialize, Deserialize)]
struct Manifest { format: String, version: u32, root: PathBuf, assets: Vec<Asset> }
#[derive(Serialize, Deserialize)]
struct Asset { relative: PathBuf, sha256: String }
#[derive(Debug)]
pub struct Report { pub directory: PathBuf, pub photos: usize, pub files: usize }

fn check(cancel: &AtomicBool) -> Result<()> { ensure!(!cancel.load(Ordering::Relaxed), "目录导出已取消"); Ok(()) }
fn regular(path: &Path) -> Result<()> {
    let mut at=Some(path);
    while let Some(p)=at {
        let m=fs::symlink_metadata(p)?;
        ensure!(!m.file_type().is_symlink(), "不跟随符号链接：{}",p.display());
        #[cfg(windows)] {use std::os::windows::fs::MetadataExt;ensure!(m.file_attributes()&0x400==0,"不跟随链接或联接：{}",p.display());}
        at=p.parent().filter(|p|!p.as_os_str().is_empty());
    }
    ensure!(path.is_file(),"资源不是普通文件：{}",path.display());Ok(())
}
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut file,value)?;file.sync_all()?;Ok(())
}
fn replace_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temp=path.with_extension(format!("{}.tmp",nonce()));write_json(&temp,value)?;
    let result=fs::rename(&temp,path);if result.is_err(){let _=fs::remove_file(&temp);}result?;Ok(())
}
fn nonce()->String {format!("{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos())}
fn bounded_json(path:&Path)->Result<serde_json::Value>{
    ensure!(fs::metadata(path)?.len()<=crate::project::MAX_PROJECT_BYTES as u64,"目录附属数据过大");Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn path_map(value:&mut serde_json::Value,paths:&HashMap<String,String>){
    match value {
        serde_json::Value::String(s)=>{if let Some(next)=paths.get(s){*s=next.clone();}},
        serde_json::Value::Array(a)=>for v in a {path_map(v,paths);},
        serde_json::Value::Object(o)=>{let old=std::mem::take(o);for (k,mut v) in old {path_map(&mut v,paths);o.insert(paths.get(&k).cloned().unwrap_or(k),v);}},_=>{}
    }
}
fn insert_path(map:&mut HashMap<String,String>,old:&Path,new:&Path){
    map.insert(old.to_string_lossy().into_owned(),new.to_string_lossy().into_owned());
    map.insert(old.to_string_lossy().replace('\\',"/"),new.to_string_lossy().replace('\\',"/"));
}
struct Builder<'a> { staging:&'a Path, target:&'a Path, paths:HashMap<String,String>, assets:Vec<Asset>, cancel:&'a AtomicBool }
impl Builder<'_> {
    fn copy(&mut self,source:&Path,relative:PathBuf)->Result<PathBuf>{
        check(self.cancel)?;
        if let Some(path)=self.paths.get(&source.to_string_lossy().into_owned()){return Ok(PathBuf::from(path));}
        regular(source)?;let target=self.staging.join(&relative);fs::create_dir_all(target.parent().context("资源无父目录")?)?;
        let before=fs::metadata(source)?;let mut input=fs::File::open(source)?;
        let mut output=fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
        let mut buf=[0u8;262144];loop {check(self.cancel)?;let n=input.read(&mut buf)?;if n==0{break;}output.write_all(&buf[..n])?;}
        output.sync_all()?;drop(output);
        let hash=hash_file(&target)?;
        ensure!(before.len()==fs::metadata(source)?.len() && before.modified()?==fs::metadata(source)?.modified()? && hash_file(source)?==hash,"复制期间资源变化：{}",source.display());
        let final_path=self.target.join(&relative);insert_path(&mut self.paths,source,&final_path);
        self.assets.push(Asset{relative,sha256:hash});Ok(final_path)
    }
    fn resource(&mut self,path:&Path)->Result<PathBuf>{
        regular(path)?;let hash=hash_file(path)?;
        let rel=PathBuf::from("Resources").join(hash).join(path.file_name().context("资源无文件名")?);
        // Distinct paths may contain the same resource bytes.
        if self.staging.join(&rel).exists(){let final_path=self.target.join(&rel);insert_path(&mut self.paths,path,&final_path);return Ok(final_path);}
        self.copy(path,rel)
    }
}
impl Export {
    pub fn run(mut self,target:&Path,cancel:&AtomicBool)->Result<Report>{
        ensure!(target.is_absolute()&&!target.exists(),"请选择尚不存在的新目录，不覆盖已有目录");
        let parent=target.parent().context("导出目录不能是磁盘根目录")?;
        ensure!(parent.is_dir(),"导出目录的父文件夹不存在");
        let staging=parent.join(format!(".luminacraft-export-{}",nonce()));fs::create_dir(&staging)?;
        let result=self.build(&staging,target,cancel);
        match result {
            Ok(report)=>{if let Err(e)=fs::rename(&staging,target){let _=fs::remove_dir_all(&staging);return Err(e.into());}Ok(report)},
            Err(e)=>{let _=fs::remove_dir_all(&staging);Err(e)}
        }
    }
    fn build(&mut self,staging:&Path,target:&Path,cancel:&AtomicBool)->Result<Report>{
        check(cancel)?;
        if let Some(project)=&mut self.project {
            for photo in &mut project.photos {
                if photo.catalog_ref.is_none() && self.selected.is_none() {
                    let existing=self.catalog.photos().find(|p|p.copy_of.is_none()&&crate::library::photo_path(p).is_some_and(|s|crate::bridge::same_path(Path::new(s),&photo.source.path))).map(|p|p.id);
                    let id=match existing {Some(id)=>id,None=>{let mut p=crate::library::index_photo(&photo.source.path)?;p.id=self.catalog.alloc_photo_id();let id=p.id;self.catalog.apply(lightcraft_catalog::Op::AddPhoto{photo:Box::new(p)})?;id}};
                    photo.catalog_ref=Some(crate::project::CatalogRef{library:self.library.clone(),id:id.0,copy_name:None});
                }
                if let Some(r)=&photo.catalog_ref {if crate::bridge::same_path(&r.library,&self.library) && self.catalog.photo(PhotoId(r.id)).is_some(){
                    self.catalog.apply(lightcraft_catalog::Op::SetDevelop{id:PhotoId(r.id),settings:std::sync::Arc::new(photo.edit.develop.clone()),label:"主界面编辑".into(),edited:None})?;
                }}
            }
        }
        if let Some(selected)=&mut self.selected {
            ensure!(!selected.is_empty(),"请先选择要导出的照片");
            ensure!(selected.iter().all(|id|self.catalog.photo(*id).is_some()),"选片已不在照片库中");
            // A virtual copy keeps its master and full version identity.
            loop {let parents:Vec<_>=selected.iter().filter_map(|id|self.catalog.photo(*id).and_then(|p|p.copy_of)).filter(|p|!selected.contains(p)).collect();if parents.is_empty(){break;}selected.extend(parents);}
            let removed:Vec<_>=self.catalog.photos().filter(|p|!selected.contains(&p.id)).map(|p|p.id).collect();
            self.catalog.apply(self.catalog.delete_photos_permanently_ops(&removed))?;
        }
        let mut b=Builder{staging,target,paths:HashMap::new(),assets:vec![],cancel};
        insert_path(&mut b.paths,&self.library,target);
        let ids:HashSet<_>=self.catalog.photos().map(|p|p.id.0).collect();
        if let Some(project)=&mut self.project {
            let mut indices=HashMap::new();let mut photos=Vec::new();
            for (old,p) in project.photos.iter().enumerate(){
                let belongs=p.catalog_ref.as_ref().is_some_and(|r|crate::bridge::same_path(&r.library,&self.library)&&ids.contains(&r.id));
                if belongs {indices.insert(old,photos.len());photos.push(p.clone());}
            }
            project.photos=photos;
            project.versions.retain_mut(|v|{if let Some(index)=indices.get(&v.photo){v.photo=*index;true}else{false}});
            project.style_outputs.retain(|r|project.photos.iter().any(|p|r.matches(p)));
            let hashes:HashSet<_>=project.photos.iter().map(|p|p.source.sha256.clone()).collect();
            project.publishes.retain(|r|hashes.contains(&r.source_sha256));
            if self.selected.is_some(){project.advanced_outputs.retain(|r|!r.source_sha256.is_empty()&&hashes.contains(&r.source_sha256));}
            for p in &project.photos {
                if self.copy_originals{p.source.verify()?;}else{p.source.verify_available()?;}
                if let Some(cached)=&p.cached_denoise{cached.verify(&p.source.sha256)?;let mut cached=cached.clone();cached.path=b.resource(&cached.path)?;
                    if let Some(r)=&p.catalog_ref{let dir=staging.join("BridgeResources/DenoiseIndex").join(r.id.to_string());fs::create_dir_all(&dir)?;write_json(&dir.join("100000000000000000000000000000000000000.json"),&cached)?;}
                }
            }
            for r in &project.style_outputs {for (path,sha) in [(&r.input,&r.input_sha256),(&r.output,&r.output_sha256)]{ensure!(hash_file(path)?==*sha,"风格资源已改变");b.resource(path)?;}}
            for r in project.publishes.iter().chain(&project.advanced_outputs){ensure!(hash_file(&r.output)?==r.output_sha256,"导出成片已改变");b.resource(&r.output)?;}
        }
        // Copy originals in groups so both sidecar naming styles stay adjacent.
        if self.copy_originals {
            for p in self.catalog.photos(){
                check(cancel)?;let Some(path)=crate::library::photo_path(p).map(PathBuf::from) else{continue};
                use sha2::{Digest,Sha256};
                let parent=path.parent().context("原片无父目录")?;
                let group=format!("{:x}",Sha256::digest(parent.to_string_lossy().as_bytes()));
                let rel_parent=PathBuf::from("Originals").join(group);
                b.copy(&path,rel_parent.join(path.file_name().context("原片无文件名")?))?;
                for (sidecar,_,_) in crate::archive::sidecars(&path)?{b.copy(&sidecar,rel_parent.join(sidecar.file_name().context("XMP无文件名")?))?;}
                insert_path(&mut b.paths,parent,&target.join(rel_parent));
            }
        }
        // Native preference files contain user presets and LUT registration, never login keys.
        for name in ["prefs.json","presets.json"] {
            let path=self.library.join(name);if !path.exists(){continue;}
            let mut value=bounded_json(&path)?;
            if name=="prefs.json" {if let Some(profiles)=value.get("lutProfiles").and_then(|v|v.as_array()){
                for profile in profiles {if let Some(path)=profile.get("file").and_then(|v|v.as_str()){b.resource(Path::new(path))?;}}
            }}
            path_map(&mut value,&b.paths);write_json(&staging.join(name),&value)?;
        }
        // Whole-library exports retain Lightroom recovery archives and their import identity index.
        let interop=self.library.join("Interop");
        if self.selected.is_none()&&interop.is_dir(){for entry in fs::read_dir(interop)?{let entry=entry?;if entry.file_type()?.is_file()&&entry.path().extension().is_some_and(|e|e=="json"||e=="lca"){
            let immutable=b.assets.len();b.copy(&entry.path(),PathBuf::from("Interop").join(entry.file_name()))?;
            // The native importer may rotate or update these archives after export.
            b.assets.truncate(immutable);
        }}}
        // Carry already generated advanced denoise products, including photos outside the project.
        for p in self.catalog.photos(){
            if let Some(path)=crate::library::photo_path(p){
                let index=self.library.join("BridgeResources/DenoiseIndex").join(p.id.0.to_string());
                if !index.is_dir(){continue;}
                // Native content_hash is a preview fingerprint, not the project's SHA-256.
                let hash=hash_file(Path::new(path))?;
                if let Some(mut cached)=crate::resources::latest(&self.library,p.id.0,&hash)? {
                    cached.verify(&hash)?;cached.path=b.resource(&cached.path)?;
                    let dir=staging.join("BridgeResources/DenoiseIndex").join(p.id.0.to_string());fs::create_dir_all(&dir)?;write_json(&dir.join("000000000000000000000000000000000000000.json"),&cached)?;
                }
            }
        }
        let mut value=serde_json::to_value(&self.catalog)?;path_map(&mut value,&b.paths);
        let catalog:Catalog=serde_json::from_value(value)?;
        let (mut journal,_,_)=Journal::open(Box::new(FsStore::open(staging)?))?;journal.snapshot(&catalog)?;drop(journal);
        if let Some(project)=&mut self.project {
            let mut value=serde_json::to_value(&project)?;path_map(&mut value,&b.paths);
            let project:Project=serde_json::from_value(value)?;project.validate()?;
            // Verify the actual staged bytes against project identities, not only against
            // the source observed by the copier (which could have changed after preflight).
            let mut checked=HashMap::<PathBuf,String>::new();
            let mut verify=|path:&Path,expected:&str,allow_missing:bool|->Result<()> {
                let local=path.strip_prefix(target).map(|rel|staging.join(rel)).unwrap_or_else(|_|path.to_path_buf());
                if allow_missing&&!local.exists(){return Ok(());}
                let hash=match checked.get(&local){Some(hash)=>hash.clone(),None=>{let hash=hash_file(&local)?;checked.insert(local.clone(),hash.clone());hash}};
                ensure!(hash==expected,"资源与工程身份不一致：{}",path.display());Ok(())
            };
            for photo in &project.photos{verify(&photo.source.path,&photo.source.sha256,!self.copy_originals)?;if let (Some(path),Some(hash))=(&photo.source.xmp_path,&photo.source.xmp_sha256){verify(path,hash,!self.copy_originals)?;}if let Some(c)=&photo.cached_denoise{verify(&c.path,&c.sha256,false)?;}}
            for r in &project.style_outputs{verify(&r.input,&r.input_sha256,false)?;verify(&r.output,&r.output_sha256,false)?;}
            for r in project.publishes.iter().chain(&project.advanced_outputs){verify(&r.output,&r.output_sha256,false)?;}
            write_json(&staging.join("project.json"),&project)?;
        }
        check(cancel)?;
        let files=b.assets.len();
        write_json(&staging.join(MARKER),&Manifest{format:"LuminaCraft/LightCraft catalog".into(),version:1,root:target.to_path_buf(),assets:b.assets})?;
        Ok(Report{directory:target.to_path_buf(),photos:catalog.len(),files})
    }
}

/// Resolve a descriptor or a native directory, without creating anything.
pub fn resolve(path:&Path)->Result<PathBuf>{
    if crate::catalog_file::is_file(path){return crate::catalog_file::prepare(path,&AtomicBool::new(false));}
    let dir=if path.is_file(){ensure!(path.file_name().is_some_and(|n|n==MARKER),"请选择 catalog.lccatalog");path.parent().context("目录文件无父目录")?}else{path};
    ensure!(dir.is_dir(),"目录不存在");let dir=dir.canonicalize()?;
    ensure!(dir.join("catalog.snap").is_file()||dir.join("catalog.log").is_file(),"该位置不是 LightCraft 照片库目录");Ok(dir)
}

/// Main saves native develop state before saving its additional retouch/project data.
pub fn save_project_develop(project:&Project,library:&mut crate::library::Library)->Result<()> {
    let mut ops=Vec::new();
    for photo in &project.photos {if let Some(r)=&photo.catalog_ref {if crate::bridge::same_path(&r.library,&library.directory){
        let native=library.catalog.photo(PhotoId(r.id)).context("目录中的工程照片已不存在")?;
        if *native.develop!=photo.edit.develop {ops.push(lightcraft_catalog::Op::SetDevelop{id:native.id,settings:std::sync::Arc::new(photo.edit.develop.clone()),label:"主界面编辑".into(),edited:None});}
    }}}
    library.apply(ops)
}
pub fn load_project(directory:&Path,library:&crate::library::Library)->Result<Project>{
    let mut project=Project::open_catalog(directory)?;
    for photo in &mut project.photos {if let Some(r)=&photo.catalog_ref {if crate::bridge::same_path(&r.library,&library.directory){
        if let Some(native)=library.catalog.photo(PhotoId(r.id)){photo.edit.develop=(*native.develop).clone();}
    }}}
    project.validate()?;Ok(project)
}

fn internal_paths(value:&serde_json::Value,old:&Path,new:&Path,map:&mut HashMap<String,String>){
    match value {
        serde_json::Value::String(s)=>{let path=Path::new(s);if let Ok(rel)=path.strip_prefix(old){if rel.components().all(|c|matches!(c,Component::Normal(_))){insert_path(map,path,&new.join(rel));}}},
        serde_json::Value::Array(a)=>for v in a{internal_paths(v,old,new,map);},
        serde_json::Value::Object(o)=>for (k,v) in o{internal_paths(&serde_json::Value::String(k.clone()),old,new,map);internal_paths(v,old,new,map);},_=>{}
    }
}

/// Called with the native library's exclusive lock held. Idempotent after an interrupted rebase.
pub(crate) fn relocate(dir:&Path,journal:&mut Journal,catalog:&mut Catalog)->Result<()> {
    let marker=dir.join(MARKER);if !marker.exists(){return Ok(());}
    ensure!(fs::metadata(&marker)?.len()<=32*1024*1024,"目录描述文件过大");
    let mut m:Manifest=serde_json::from_slice(&fs::read(&marker)?)?;
    ensure!(m.version==1 && m.format=="LuminaCraft/LightCraft catalog" && m.root.is_absolute(),"不支持的目录格式");
    for asset in &m.assets {ensure!(!asset.relative.as_os_str().is_empty()&&asset.relative.components().all(|c|matches!(c,Component::Normal(_))),"目录资源路径无效");}
    if crate::bridge::same_path(dir,&m.root){return Ok(());}
    let mut paths=HashMap::new();insert_path(&mut paths,&m.root,dir);
    for asset in &m.assets {
        let new=dir.join(&asset.relative);regular(&new)?;
        ensure!(hash_file(&new)?==asset.sha256,"目录资源缺失或损坏：{}",new.display());
        let mut rel=Some(asset.relative.as_path());while let Some(p)=rel {if p.as_os_str().is_empty(){break;}insert_path(&mut paths,&m.root.join(p),&dir.join(p));rel=p.parent();}
    }
    let mut value=serde_json::to_value(&catalog)?;internal_paths(&value,&m.root,dir,&mut paths);path_map(&mut value,&paths);let next:Catalog=serde_json::from_value(value)?;
    let mut updates=Vec::new();
    for name in ["project.json","prefs.json","presets.json"] {let path=dir.join(name);if path.exists(){regular(&path)?;let mut value=bounded_json(&path)?;internal_paths(&value,&m.root,dir,&mut paths);path_map(&mut value,&paths);updates.push((path,value));}}
    let indices=dir.join("BridgeResources/DenoiseIndex");
    if indices.is_dir(){for entry in fs::read_dir(indices)? {let entry=entry?;if !entry.file_type()?.is_dir(){continue;}for file in fs::read_dir(entry.path())? {let path=file?.path();if path.extension().is_some_and(|e|e=="json"){regular(&path)?;let mut value=bounded_json(&path)?;path_map(&mut value,&paths);updates.push((path,value));}}}}
    let styles=dir.join("ai-style");
    if styles.is_dir(){for entry in fs::read_dir(styles)?{let entry=entry?;if !entry.file_type()?.is_dir(){continue;}for file in fs::read_dir(entry.path())?{let path=file?.path();if path.extension().is_some_and(|e|e=="json"){regular(&path)?;let mut value=bounded_json(&path)?;internal_paths(&value,&m.root,dir,&mut paths);path_map(&mut value,&paths);updates.push((path,value));}}}}
    for (path,value) in updates {replace_json(&path,&value)?;}
    journal.snapshot(&next)?;*catalog=next;m.root=dir.to_path_buf();replace_json(&marker,&m)?;Ok(())
}

