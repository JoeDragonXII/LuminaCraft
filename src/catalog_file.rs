//! Single-file catalogs; private native workspaces keep the existing LightCraft engine unchanged.
use anyhow::{Context,Result,ensure};
use serde::{Serialize,Deserialize};
use std::{collections::HashSet,fs,io::{Read,Write},path::{Component,Path,PathBuf},sync::atomic::{AtomicBool,Ordering}};
use sha2::{Digest,Sha256};
use crate::{catalog_io,project::hash_file};
pub const EXT:&str="lumcat";
const BINDING:&str="single-file.json";
const FILE_INFO:&str="lumina-file.json";
#[derive(Serialize,Deserialize)]
pub struct Backing {pub source:PathBuf,sha256:String,signature:String}
#[derive(Serialize,Deserialize)]
struct Info {format:String,version:u32}
pub fn is_file(path:&Path)->bool{path.extension().is_some_and(|s|s.eq_ignore_ascii_case(EXT))}
fn nonce()->String{format!("{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos())}
fn check(cancel:&AtomicBool)->Result<()>{ensure!(!cancel.load(Ordering::Relaxed),"目录文件操作已取消");Ok(())}
fn write_json<T:Serialize>(path:&Path,value:&T)->Result<()>{let temp=path.with_extension(format!("{}.tmp",nonce()));let result=(||->Result<()>{let mut f=fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;serde_json::to_writer(&mut f,value)?;f.sync_all()?;drop(f);fs::rename(&temp,path)?;Ok(())})();if result.is_err(){let _=fs::remove_file(temp);}result}
fn read_json<T:serde::de::DeserializeOwned>(path:&Path)->Result<T>{ensure!(fs::metadata(path)?.len()<=1024*1024,"目录文件状态过大");Ok(serde_json::from_slice(&fs::read(path)?)?)}
fn reject_link(path:&Path)->Result<()>{let mut at=Some(path);while let Some(p)=at{let m=fs::symlink_metadata(p)?;ensure!(!m.file_type().is_symlink(),"目录文件不跟随符号链接");#[cfg(windows)]{use std::os::windows::fs::MetadataExt;ensure!(m.file_attributes()&0x400==0,"目录文件不跟随联接");}at=p.parent().filter(|p|!p.as_os_str().is_empty());}Ok(())}
fn files(dir:&Path)->Result<Vec<PathBuf>>{
    let mut result=Vec::new();let mut pending=Vec::new();
    for name in [catalog_io::MARKER,"catalog.snap","catalog.log","project.json","prefs.json","presets.json"]{let path=dir.join(name);if path.exists(){reject_link(&path)?;ensure!(path.is_file(),"目录元数据不是普通文件");result.push(PathBuf::from(name));}}
    for name in ["Originals","Resources","Profiles","BridgeResources","Interop","ai-style"]{let path=dir.join(name);if path.exists(){pending.push((PathBuf::from(name),0));}}
    while let Some((rel,depth))=pending.pop(){ensure!(depth<32,"资源目录层级过深");let path=dir.join(&rel);reject_link(&path)?;for entry in fs::read_dir(path)?{let entry=entry?;let next=rel.join(entry.file_name());reject_link(&entry.path())?;if entry.file_type()?.is_dir(){pending.push((next,depth+1));}else if entry.file_type()?.is_file(){result.push(next);}ensure!(result.len()+pending.len()<=200_000,"目录文件资源数量过多");}}
    result.sort();Ok(result)
}
fn signature(dir:&Path)->Result<String>{let mut h=Sha256::new();for rel in files(dir)?{h.update(rel.to_string_lossy().as_bytes());h.update([0]);h.update(hash_file(&dir.join(rel))?.as_bytes());}Ok(format!("{:x}",h.finalize()))}
pub fn backing(dir:&Path)->Result<Option<Backing>>{let p=dir.join(BINDING);if !p.exists(){return Ok(None)};let b:Backing=read_json(&p)?;ensure!(b.source.is_absolute()&&is_file(&b.source)&&b.sha256.len()==64&&b.sha256.bytes().all(|b|b.is_ascii_hexdigit()),"目录文件绑定无效");Ok(Some(b))}
fn pack(dir:&Path,target:&Path,cancel:&AtomicBool)->Result<()> {
    let out=fs::OpenOptions::new().write(true).create_new(true).open(target)?;
    let mut zip=zip::ZipWriter::new(out);let options=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
    zip.start_file(FILE_INFO,options)?;zip.write_all(&serde_json::to_vec(&Info{format:"LuminaCraft catalog file".into(),version:1})?)?;
    let mut buf=[0u8;262144];
    for rel in files(dir)?{check(cancel)?;let path=dir.join(&rel);let before=fs::metadata(&path)?;zip.start_file(rel.to_string_lossy().replace('\\',"/"),options)?;let mut f=fs::File::open(&path)?;let mut hash=Sha256::new();loop{check(cancel)?;let n=f.read(&mut buf)?;if n==0{break}zip.write_all(&buf[..n])?;hash.update(&buf[..n]);}
        ensure!(before.len()==fs::metadata(&path)?.len()&&before.modified()?==fs::metadata(&path)?.modified()?&&format!("{:x}",hash.finalize())==hash_file(&path)?,"打包期间目录数据变化，请重试");
    }
    let out=zip.finish()?;out.sync_all()?;Ok(())
}
pub fn export(export:catalog_io::Export,target:&Path,cancel:&AtomicBool)->Result<catalog_io::Report>{
    ensure!(is_file(target)&&target.is_absolute()&&!target.exists(),"请选择新的 .lumcat 文件，不覆盖已有文件");
    let parent=target.parent().context("目录文件没有父文件夹")?.canonicalize()?;
    let stage=parent.join(format!(".lumcat-{}",nonce()));fs::create_dir(&stage)?;
    let result=(||->Result<catalog_io::Report>{let mut report=export.run(&stage.join("native"),cancel)?;pack(&report.directory,&stage.join("catalog.tmp"),cancel)?;check(cancel)?;ensure!(!target.exists(),"目标文件已存在");fs::rename(stage.join("catalog.tmp"),target)?;report.directory=target.to_path_buf();Ok(report)})();
    // This freshly created staging directory is a direct child of the verified parent.
    if stage.parent()==Some(parent.as_path()){let _=fs::remove_dir_all(&stage);}result
}
fn unpack(source:&Path,dir:&Path,cancel:&AtomicBool)->Result<()> {
    let mut zip=zip::ZipArchive::new(fs::File::open(source)?)?;ensure!(zip.len()<=200_000,"目录文件资源数量过多");
    let info:Info={let mut f=zip.by_name(FILE_INFO)?;ensure!(f.size()<=1024,"目录文件标识过大");let mut bytes=Vec::new();f.read_to_end(&mut bytes)?;serde_json::from_slice(&bytes)?};
    ensure!(info.version==1&&info.format=="LuminaCraft catalog file","不支持的目录文件格式");
    let mut names=HashSet::new();let mut buf=[0u8;262144];
    for n in 0..zip.len(){check(cancel)?;let mut entry=zip.by_index(n)?;let name=entry.name().to_string();if name==FILE_INFO{continue;}
        ensure!(!name.contains('\\')&&!name.contains(':')&&!name.chars().any(char::is_control),"目录文件路径无效");
        let rel=Path::new(&name);ensure!(!rel.as_os_str().is_empty()&&rel.components().all(|p|matches!(p,Component::Normal(_))),"目录文件含越界路径");
        ensure!(names.insert(name.to_lowercase()),"目录文件存在重复路径");
        ensure!(entry.compression()==zip::CompressionMethod::Stored&&!entry.is_dir()&&!entry.is_symlink(),"目录文件含不支持的条目");
        ensure!(entry.size()<=2*1024*1024*1024,"目录文件单个资源超过 2 GiB");
        let top=rel.components().next().context("资源路径为空")?.as_os_str().to_string_lossy();
        ensure!([catalog_io::MARKER,"catalog.snap","catalog.log","project.json","prefs.json","presets.json","Resources","Originals","Profiles","BridgeResources","Interop","ai-style"].contains(&top.as_ref()),"目录文件含未知资源");
        let path=dir.join(rel);fs::create_dir_all(path.parent().context("资源没有父目录")?)?;let mut out=fs::OpenOptions::new().write(true).create_new(true).open(&path)?;let mut total=0u64;loop{check(cancel)?;let n=entry.read(&mut buf)?;if n==0{break}total+=n as u64;ensure!(total<=2*1024*1024*1024,"资源超出大小限制");out.write_all(&buf[..n])?;}ensure!(total==entry.size(),"目录文件资源不完整");out.sync_all()?;
    }
    ensure!(dir.join(catalog_io::MARKER).is_file()&&dir.join("catalog.snap").is_file(),"目录文件缺少原生照片库");Ok(())
}
fn cache_root()->Result<PathBuf>{if let Some(path)=std::env::var_os("LUMINACRAFT_CATALOG_CACHE"){return Ok(PathBuf::from(path))}Ok(std::env::var_os("LOCALAPPDATA").map(PathBuf::from).context("找不到本机目录文件工作区")?.join("LuminaCraft/CatalogFiles"))}
pub fn prepare(source:&Path,cancel:&AtomicBool)->Result<PathBuf>{prepare_in(source,&cache_root()?,cancel)}
pub fn prepare_in(source:&Path,cache:&Path,cancel:&AtomicBool)->Result<PathBuf>{
    reject_link(source)?;let source=source.canonicalize()?;let sha=hash_file(&source)?;
    let key=format!("{:x}",Sha256::digest(source.to_string_lossy().to_lowercase().as_bytes()));let home=cache.join(key);fs::create_dir_all(&home)?;let home=home.canonicalize()?;
    let _guard=lightcraft_catalog::LibraryLock::acquire(&home,"LuminaCraft single-file preparation").map_err(|e|anyhow::anyhow!(e.to_string()))?;
    let pointer=home.join("active.json");
    if pointer.exists(){let name:String=read_json(&pointer)?;ensure!(name.len()<100&&Path::new(&name).components().all(|c|matches!(c,Component::Normal(_))),"目录工作区状态无效");let dir=home.join(name);
        if let Some(b)=backing(&dir)?{if crate::bridge::same_path(&b.source,&source){if b.sha256==sha{return Ok(dir)}
            let _lock=lightcraft_catalog::LibraryLock::acquire(&dir,"LuminaCraft file refresh").map_err(|e|anyhow::anyhow!(e.to_string()))?;
            ensure!(signature(&dir)?==b.signature,"目录文件在外部发生变化，本机仍有未保存数据；请先从原工作区另存为，避免覆盖");
        }}
    }
    let name=format!("work-{}",nonce());let dir=home.join(&name);fs::create_dir(&dir)?;
    let result=(||->Result<()>{unpack(&source,&dir,cancel)?;ensure!(hash_file(&source)?==sha,"读取期间目录文件发生变化");let b=Backing{source:source.clone(),sha256:sha,signature:signature(&dir)?};write_json(&dir.join(BINDING),&b)?;write_json(&pointer,&name)?;Ok(())})();
    if result.is_err(){if dir.parent()==Some(home.as_path()){let _=fs::remove_dir_all(&dir);}result?;}Ok(dir)
}
/// Caller holds the native library lock and has flushed its journal.
pub fn save(dir:&Path)->Result<bool>{
    let Some(mut b)=backing(dir)? else{return Ok(false)};
    ensure!(b.source.is_file()&&hash_file(&b.source)?==b.sha256,"目录文件在外部改变或被移走，工作区数据已保留；请另存为新目录文件");
    {let mut zip=zip::ZipArchive::new(fs::File::open(&b.source)?)?;let mut info=zip.by_name(FILE_INFO)?;ensure!(info.size()<=1024,"目录文件标识过大");let mut bytes=Vec::new();info.read_to_end(&mut bytes)?;let info:Info=serde_json::from_slice(&bytes)?;ensure!(info.version==1&&info.format=="LuminaCraft catalog file","不能覆盖其他格式的文件");}
    let signature=signature(dir)?;if signature==b.signature{return Ok(false)};
    let temp=b.source.with_extension(format!("{}.tmp",nonce()));
    let result=(||->Result<()>{pack(dir,&temp,&AtomicBool::new(false))?;let next=hash_file(&temp)?;ensure!(hash_file(&b.source)?==b.sha256,"保存期间目录文件在外部变化，未覆盖");fs::rename(&temp,&b.source)?;b.sha256=next;b.signature=signature;write_json(&dir.join(BINDING),&b)?;Ok(())})();
    if result.is_err(){let _=fs::remove_file(temp);}result?;Ok(true)
}
