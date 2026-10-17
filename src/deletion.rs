//! Explicit disk deletion. Only the Windows recycle operation touches originals; no unlink fallback.
use anyhow::{Result,Context,ensure};
use lightcraft_catalog::{Catalog,PhotoId,Source};
use serde::{Serialize,Deserialize};
use std::{collections::{BTreeSet,BTreeMap},fs,io::Write,path::{Path,PathBuf}};
use crate::project::hash_file;
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub struct File {pub path:PathBuf,pub sha256:String,pub bytes:u64,pub records:Vec<PhotoId>}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub struct Plan {pub revision:u64,pub selected:Vec<PhotoId>,pub files:Vec<File>,pub records:usize}
#[derive(Debug,Default,Serialize,Deserialize)]
pub struct Report {pub recycled:usize,pub removed:usize,pub errors:Vec<String>,pub recovery:PathBuf}
fn key(p:&Path)->String{p.to_string_lossy().replace('\\',"/").trim_start_matches("//?/").to_lowercase()}
fn regular(path:&Path)->Result<()> {
    ensure!(path.is_absolute(),"原片路径必须是绝对路径");
    for p in path.ancestors(){let m=fs::symlink_metadata(p)?;ensure!(!m.file_type().is_symlink(),"不删除符号链接");
        #[cfg(windows)]{use std::os::windows::fs::MetadataExt;ensure!(m.file_attributes()&0x400==0,"不删除链接或联接目录中的文件");}}
    ensure!(path.is_file(),"原片缺失或不是文件，请改用从照片库移除");Ok(())
}
pub fn prepare(catalog:&Catalog,ids:&[PhotoId])->Result<Plan>{
    let selected:BTreeSet<_>=ids.iter().copied().collect();ensure!(!selected.is_empty(),"请先选择照片");let mut paths=BTreeMap::new();
    for id in &selected{let p=catalog.photo(*id).context("选片已不在照片库")?;if let Some(master)=p.copy_of{ensure!(selected.contains(&master),"虚拟副本没有独立原片，请使用从照片库移除；需要删除原片时请选择原始记录");continue;}let Source::File{path}=&p.source else{anyhow::bail!("演示照片没有磁盘原片")};let path=PathBuf::from(path);regular(&path)?;let canonical=fs::canonicalize(path)?;paths.insert(key(&canonical),canonical);}
    let mut references:BTreeMap<String,(PathBuf,Vec<PhotoId>)>=BTreeMap::new();
    for p in catalog.photos(){if let Source::File{path}=&p.source{references.entry(key(Path::new(path))).or_insert_with(||(PathBuf::from(path),Vec::new())).1.push(p.id);}}
    let mut groups:BTreeMap<String,Vec<PhotoId>>=BTreeMap::new();
    for (name,(path,ids)) in references{let resolved=if paths.contains_key(&name){name}else{fs::canonicalize(path).map(|p|key(&p)).unwrap_or(name)};if paths.contains_key(&resolved){groups.entry(resolved).or_default().extend(ids);}}
    let mut files=Vec::new();
    for (name,path) in paths {let mut records=groups.remove(&name).unwrap_or_default();records.sort();let bytes=fs::metadata(&path)?.len();files.push(File{sha256:hash_file(&path)?,path,bytes,records});}
    Ok(Plan{revision:catalog.revision,selected:selected.into_iter().collect(),records:files.iter().map(|f|f.records.len()).sum(),files})
}
fn write_new(path:&Path,value:&impl Serialize)->Result<()> {let mut f=fs::OpenOptions::new().write(true).create_new(true).open(path)?;f.write_all(&serde_json::to_vec_pretty(value)?)?;f.sync_all()?;Ok(())}
fn check_packaged(plan:&Plan,directory:&Path)->Result<()> {
    let marker=directory.join(crate::catalog_io::MARKER);if !marker.exists(){return Ok(())}
    ensure!(fs::metadata(&marker)?.len()<=64*1024*1024,"目录清单过大，未进行磁盘删除");
    let manifest:serde_json::Value=serde_json::from_slice(&fs::read(marker)?)?;
    if let Some(assets)=manifest.get("assets").and_then(|v|v.as_array()){for asset in assets{if let Some(relative)=asset.get("relative").and_then(|v|v.as_str()){let asset=directory.join(relative);ensure!(!plan.files.iter().any(|f|key(&f.path)==key(&asset)),"此原片是打包目录的内嵌资源，暂不支持从磁盘删除；请使用从照片库移除");}}}
    Ok(())
}
fn execute_with(plan:&Plan,catalog:&Catalog,directory:&Path,mut remove:impl FnMut(&[PhotoId])->Result<()>,mut recycle:impl FnMut(&Path)->Result<()>)->Result<Report>{
    ensure!(prepare(catalog,&plan.selected)?==*plan,"照片库或原片已变化，请重新打开删除确认");
    check_packaged(plan,directory)?;
    let recovery=directory.join("DiskDelete").join(format!("{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));fs::create_dir_all(&recovery)?;
    write_new(&recovery.join("plan.json"),plan)?;
    let records:Vec<_>=plan.files.iter().flat_map(|f|&f.records).filter_map(|id|catalog.photo(*id)).collect();write_new(&recovery.join("records.json"),&records)?;
    let mut report=Report{recovery:recovery.clone(),..Default::default()};
    for (n,file) in plan.files.iter().enumerate(){
        let result=(||->Result<()>{regular(&file.path)?;ensure!(hash_file(&file.path)?==file.sha256,"原片已改变");
            // A durable per-file intent precedes the OS action; a crash never loses the path/id map.
            write_new(&recovery.join(format!("{n}-intent.json")),file)?;
            recycle(&file.path)?;ensure!(!file.path.exists(),"系统未移走原片，照片库记录保留");report.recycled+=1;
            remove(&file.records).context("原片已移入回收站，但目录保存失败；请保留恢复档案")?;report.removed+=file.records.len();
            write_new(&recovery.join(format!("{n}-done.json")),file)?;Ok(())})();
        if let Err(e)=result{report.errors.push(format!("{}：{e:#}",file.path.display()));break;}
    }
    write_new(&recovery.join("report.json"),&report).with_context(||format!("删除回执保存失败，请检查恢复档案 {}",recovery.display()))?;Ok(report)
}
pub fn from_library(lib:&mut crate::library::Library,plan:&Plan)->Result<Report>{
    lib.save_file()?;let catalog=lib.catalog.clone();let dir=lib.directory.clone();
    execute_with(plan,&catalog,&dir,|ids|{let op=lib.catalog.delete_photos_permanently_ops(ids);lib.apply(vec![op])?;lib.save_file()?;Ok(())},recycle_file)
}
/// The advanced UI asks for a frozen plan first; only its second confirmation submits that plan.
pub fn advanced(session:&mut lightcraft_engine::Session,params:&serde_json::Value)->Result<serde_json::Value,String>{
    (||->Result<serde_json::Value>{
        if let Some(value)=params.get("plan"){
            ensure!(params.get("confirmed").and_then(|v|v.as_bool())==Some(true),"需要单独确认从磁盘删除");let plan:Plan=serde_json::from_value(value.clone())?;
            session.persist()?;let dir=session.library.as_ref().filter(|l|l.on_disk).context("请先保存照片库")?.dir.clone();crate::catalog_file::save(&dir)?;let catalog=session.catalog.clone();
            let report=execute_with(&plan,&catalog,&dir,|ids|{session.undo.clear();session.redo.clear();let result=session.execute("photo.deletePermanently",&serde_json::json!({"ids":ids}));session.undo.clear();session.redo.clear();result?;session.persist()?;crate::catalog_file::save(&dir)?;Ok(())},recycle_file)?;
            Ok(serde_json::to_value(report)?)
        }else{let ids:Vec<PhotoId>=serde_json::from_value(params.get("ids").cloned().context("ids required")?)?;Ok(serde_json::to_value(prepare(&session.catalog,&ids)?)?)}
    })().map_err(|e|format!("{e:#}"))
}
pub fn recycle_file(path:&Path)->Result<()> {
    regular(path)?;
    #[cfg(not(windows))]{anyhow::bail!("当前仅支持 Windows 回收站");}
    #[cfg(windows)]{
        let path=path.to_path_buf();std::thread::spawn(move||recycle_windows(&path)).join().map_err(|_|anyhow::anyhow!("Windows 回收站操作异常"))?
    }
}
#[cfg(windows)]
fn recycle_windows(path:&Path)->Result<()> {
    use windows::{core::PCWSTR,Win32::{System::Com::*,UI::Shell::*,Storage::FileSystem::GetDriveTypeW}};
    let s=path.to_string_lossy();let s=s.strip_prefix("\\\\?\\").unwrap_or(&s);ensure!(s.as_bytes().get(1)==Some(&b':'),"网络路径暂不支持回收站删除");
    let root:Vec<u16>=format!("{}\\",s.get(..2).context("无效盘符")?).encode_utf16().chain([0]).collect();
    let wide:Vec<u16>=s.encode_utf16().chain([0]).collect();
    // SAFETY: all strings are NUL terminated and owned until the synchronous COM operation
    // completes. This dedicated STA thread owns and releases every COM interface before uninit.
    unsafe {
        ensure!(GetDriveTypeW(PCWSTR(root.as_ptr()))==3,"仅支持本机固定磁盘的回收站删除");
        CoInitializeEx(None,COINIT_APARTMENTTHREADED).ok()?;
        struct Apartment;impl Drop for Apartment{fn drop(&mut self){/* SAFETY: exactly balances this thread's successful initialization. */unsafe{CoUninitialize();}}}let _apartment=Apartment;
        let operation:IFileOperation=CoCreateInstance(&FileOperation,None,CLSCTX_INPROC_SERVER)?;
        operation.SetOperationFlags(FOFX_RECYCLEONDELETE|FOFX_EARLYFAILURE|FOF_ALLOWUNDO|FOF_NOCONFIRMATION|FOF_NOERRORUI|FOF_SILENT|FOF_NO_CONNECTED_ELEMENTS)?;
        let item:IShellItem=SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()),None)?;
        operation.DeleteItem(&item,None)?;operation.PerformOperations()?;ensure!(!operation.GetAnyOperationsAborted()?.as_bool(),"回收站操作已取消，未移除目录记录");
    }Ok(())
}

#[cfg(test)] mod tests {
    use super::*;use lightcraft_catalog::{Photo,Op};
    fn fixture(base:&Path)->Catalog {let mut c=Catalog::default();for n in 1..=2{let path=base.join(format!("{n}.png"));fs::write(&path,format!("synthetic-photo-{n}")).unwrap();let p=Photo::new(PhotoId(n),Source::File{path:path.to_string_lossy().into_owned()},&format!("{n}.png"),"PNG",20,10,"2026-10-15");c.apply(Op::AddPhoto{photo:Box::new(p)}).unwrap();}let mut p=(**c.photo(PhotoId(1)).unwrap()).clone();p.id=PhotoId(3);p.copy_of=Some(PhotoId(1));c.apply(Op::AddPhoto{photo:Box::new(p)}).unwrap();c}
    #[test]fn plan_includes_shared_copies_and_never_treats_virtual_copy_as_a_separate_file(){let t=tempfile::tempdir().unwrap();let c=fixture(t.path());assert!(prepare(&c,&[PhotoId(3)]).is_err());let p=prepare(&c,&[PhotoId(1),PhotoId(1)]).unwrap();assert_eq!(p.files.len(),1);assert_eq!(prepare(&c,&[PhotoId(1),PhotoId(3)]).unwrap().files.len(),1);assert_eq!(p.files[0].records,vec![PhotoId(1),PhotoId(3)]);assert!(t.path().join("1.png").exists());}
    #[test]fn changed_file_revision_and_missing_source_reject_before_recycling(){let t=tempfile::tempdir().unwrap();let mut c=fixture(t.path());let plan=prepare(&c,&[PhotoId(1)]).unwrap();fs::write(t.path().join("1.png"),"changed").unwrap();assert!(execute_with(&plan,&c,t.path(),|_|panic!("must not commit"),|_|panic!("must not recycle")).is_err());let plan=prepare(&c,&[PhotoId(1)]).unwrap();c.apply(Op::SetRating{id:PhotoId(1),rating:3}).unwrap();assert!(execute_with(&plan,&c,t.path(),|_|panic!("must not commit"),|_|panic!("must not recycle")).is_err());fs::remove_file(t.path().join("1.png")).unwrap();assert!(prepare(&c,&[PhotoId(1)]).is_err());}
    #[test]fn recycle_failure_keeps_catalog_and_partial_success_has_durable_recovery(){let t=tempfile::tempdir().unwrap();let c=fixture(t.path());let plan=prepare(&c,&[PhotoId(1),PhotoId(2)]).unwrap();let mut removed=vec![];let r=execute_with(&plan,&c,t.path(),|ids|{removed.extend_from_slice(ids);Ok(())},|p|{if p.file_name().unwrap()=="2.png"{anyhow::bail!("synthetic recycle failure")};fs::rename(p,p.with_extension("test-recycled"))?;Ok(())}).unwrap();assert_eq!(r.recycled,1);assert_eq!(r.removed,2);assert_eq!(removed,vec![PhotoId(1),PhotoId(3)]);assert_eq!(r.errors.len(),1);assert!(t.path().join("2.png").exists());assert!(r.recovery.join("plan.json").exists()&&r.recovery.join("records.json").exists()&&r.recovery.join("report.json").exists());}
    #[test]fn catalog_failure_after_recycle_preserves_recovery_intent_and_stops_batch(){let t=tempfile::tempdir().unwrap();let c=fixture(t.path());let plan=prepare(&c,&[PhotoId(1),PhotoId(2)]).unwrap();let r=execute_with(&plan,&c,t.path(),|_|anyhow::bail!("synthetic full disk"),|p|{fs::rename(p,p.with_extension("test-recycled"))?;Ok(())}).unwrap();assert_eq!((r.recycled,r.removed),(1,0));assert_eq!(r.errors.len(),1);assert!(r.recovery.join("0-intent.json").exists());assert!(t.path().join("2.png").exists());}
    #[cfg(windows)]#[test]fn link_sources_are_rejected_without_touching_target(){use std::os::windows::fs::symlink_file;let t=tempfile::tempdir().unwrap();let mut c=fixture(t.path());let path=t.path().join("link.png");if symlink_file(t.path().join("1.png"),&path).is_err(){return;}let p=Photo::new(PhotoId(4),Source::File{path:path.to_string_lossy().into_owned()},"link.png","PNG",20,10,"2026-10-15");c.apply(Op::AddPhoto{photo:Box::new(p)}).unwrap();assert!(prepare(&c,&[PhotoId(4)]).is_err());assert!(t.path().join("1.png").exists());}
    #[test]fn embedded_originals_are_not_deleted_out_from_under_the_package_manifest(){let t=tempfile::tempdir().unwrap();let c=fixture(t.path());let plan=prepare(&c,&[PhotoId(1)]).unwrap();fs::write(t.path().join(crate::catalog_io::MARKER),r#"{"assets":[{"relative":"1.png"}]}"#).unwrap();assert!(execute_with(&plan,&c,t.path(),|_|panic!("must not commit"),|_|panic!("must not recycle")).is_err());assert!(t.path().join("1.png").exists());}
    #[cfg(windows)]#[test]#[ignore="explicit native recycle fixture only"]fn main_library_recycle_keeps_xmp_and_persists_removal(){let t=tempfile::tempdir().unwrap();let p=t.path().join("synthetic.png");image::RgbImage::from_pixel(12,8,image::Rgb([20,30,50])).save(&p).unwrap();let xmp=p.with_extension("xmp");fs::write(&xmp,"synthetic sidecar").unwrap();let dir=t.path().join("library");let mut lib=crate::library::Library::open(&dir).unwrap();lib.add(vec![crate::library::index_photo(&p).unwrap()]).unwrap();let id=lib.catalog.photos().next().unwrap().id;let plan=prepare(&lib.catalog,&[id]).unwrap();let report=from_library(&mut lib,&plan).unwrap();assert!(report.errors.is_empty(),"{:?}",report.errors);assert_eq!((report.recycled,report.removed),(1,1));assert!(!p.exists());assert_eq!(fs::read_to_string(&xmp).unwrap(),"synthetic sidecar");drop(lib);assert_eq!(crate::library::Library::open(&dir).unwrap().catalog.len(),0);}
}


