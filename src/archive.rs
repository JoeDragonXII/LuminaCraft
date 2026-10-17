//! Add / Copy / Move import adapter. Uses the pinned LightCraft's file probe,
//! catalog and public Organize layout; transfers preserve sidecar bytes and never overwrite.
use anyhow::{Context,Result,ensure};
use lightcraft_catalog::{Op,Photo,Source};
use serde::{Serialize,Deserialize};
use std::{fs,io::{Read,Write},path::{Path,PathBuf},sync::atomic::{AtomicBool,Ordering}};
use crate::library::{self,Library};

#[derive(Clone,Copy,Debug,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="lowercase")]
pub enum ImportMode { Add, #[default] Copy, Move }
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Layout { #[default] Day, Month, Flat }
#[derive(Clone,Debug)]
pub struct ImportOptions {pub mode:ImportMode,pub destination:PathBuf,pub layout:Layout,pub restore:bool,pub source_roots:Vec<PathBuf>}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Receipt {pub source:PathBuf,pub destination:Option<PathBuf>,pub result:String,pub detail:String,pub sidecars:Vec<PathBuf>,pub sidecar_pairs:Vec<(PathBuf,PathBuf)>}
#[derive(Clone,Debug,Default,Serialize,Deserialize)]
pub struct ImportReport {pub items:Vec<Receipt>,pub cancelled:bool,#[serde(skip)]pub report_path:Option<PathBuf>}
impl ImportReport{
    pub fn count(&self,result:&str)->usize{self.items.iter().filter(|r|r.result==result).count()}
    pub fn summary(&self)->String{format!("{}添加 {} · 复制 {} · 移动 {} · 来源保留 {} · 恢复 {} · 跳过 {} · 失败 {}",if self.cancelled{"已停止 · "}else{""},self.count("added"),self.count("copied"),self.count("moved"),self.count("source_kept"),self.count("restored"),self.count("skipped"),self.count("failed"))}
}

#[derive(Clone)]
struct Pair {source:PathBuf,destination:PathBuf,hash:String,sidecar:bool,shared:bool}
struct Staged {pairs:Vec<Pair>,committed:bool}
impl Staged{
    fn rollback(&self){for pair in self.pairs.iter().rev(){if crate::project::hash_file(&pair.destination).is_ok_and(|h|h==pair.hash){let _=fs::remove_file(&pair.destination);}}}
}
impl Drop for Staged{fn drop(&mut self){if !self.committed{self.rollback();}}}

fn reparse(meta:&fs::Metadata)->bool{
    #[cfg(windows)] {use std::os::windows::fs::MetadataExt;meta.file_attributes()&0x400!=0}
    #[cfg(not(windows))] {meta.file_type().is_symlink()}
}
fn check_path(path:&Path)->Result<()> {
    for ancestor in path.ancestors(){if let Ok(meta)=fs::symlink_metadata(ancestor){ensure!(!meta.file_type().is_symlink()&&!reparse(&meta),"不允许符号链接或目录联接：{}",ancestor.display());}}
    Ok(())
}

pub fn validate_options(options:&ImportOptions)->Result<()> {
    if options.mode==ImportMode::Add{return Ok(());}
    ensure!(options.destination.is_absolute(),"请选择完整的归档目标路径。");check_path(&options.destination)?;
    let resolved=if options.destination.exists(){options.destination.canonicalize()?}else{
        let parent=options.destination.parent().context("目标路径无父目录。")?;
        ensure!(parent.exists(),"请先选择已有的父目录。");parent.canonicalize()?.join(options.destination.file_name().context("目标路径无目录名。")?)
    };
    for root in &options.source_roots{
        check_path(root)?;let source=root.canonicalize()?;
        if root.is_dir(){ensure!(!resolved.starts_with(&source)&&!source.starts_with(&resolved),"来源与归档目标不能包含或重合：{}",root.display());}
        else{ensure!(!source.starts_with(&resolved),"所选照片已位于归档目标中，请改用添加原位置。");}
    }
    Ok(())
}

pub(crate) fn sidecars(source:&Path)->Result<Vec<(PathBuf,bool,bool)>> {
    let parent=source.parent().context("来源无父目录。")?;
    let stem=source.file_stem().context("来源无文件名。")?.to_string_lossy();
    let name=source.file_name().context("来源无文件名。")?.to_string_lossy();
    let mut found=vec![];
    for entry in fs::read_dir(parent)?{
        let entry=entry?;let path=entry.path();let filename=entry.file_name().to_string_lossy().into_owned();
        let Some((base,ext))=filename.rsplit_once('.') else{continue;};
        if !ext.eq_ignore_ascii_case("xmp"){continue;}
        let stem_named=base.eq_ignore_ascii_case(&stem);
        if !stem_named&&!base.eq_ignore_ascii_case(&name){continue;}
        check_path(&path)?;ensure!(path.is_file(),"XMP 不是普通文件：{}",path.display());
        ensure!(fs::metadata(&path)?.len()<=2*1024*1024,"XMP 超过 2MB：{}",path.display());
        let shared=stem_named&&fs::read_dir(parent)?.filter_map(std::result::Result::ok).any(|e|{
            let p=e.path();p!=source&&library::supported(&p)&&p.file_stem().is_some_and(|s|s.to_string_lossy().eq_ignore_ascii_case(&stem))
        });
        found.push((path,stem_named,shared));
    }
    found.sort_by(|a,b|a.0.cmp(&b.0));Ok(found)
}

fn copy_verified(source:&Path,destination:&Path,cancel:&AtomicBool)->Result<String>{
    use sha2::{Sha256,Digest};
    check_path(source)?;ensure!(fs::metadata(source)?.len()<=crate::project::MAX_FILE,"来源文件超过 350MB。");
    let mut input=fs::File::open(source)?;
    let mut output=fs::OpenOptions::new().write(true).create_new(true).open(destination)?;
    let result=(||->Result<String>{
        let mut hasher=Sha256::new();let mut buffer=vec![0;256*1024];let mut total=0u64;
        loop{ensure!(!cancel.load(Ordering::Relaxed),"导入已取消。");let count=input.read(&mut buffer)?;if count==0{break;}
            total+=count as u64;ensure!(total<=crate::project::MAX_FILE,"来源文件在复制期间变大。");output.write_all(&buffer[..count])?;hasher.update(&buffer[..count]);}
        output.sync_all()?;
        if let Ok(stamp)=input.metadata().and_then(|m|m.modified()){let _=output.set_modified(stamp);}
        let hash=format!("{:x}",hasher.finalize());
        ensure!(crate::project::hash_file(source)?==hash,"来源文件在复制期间改变。");
        ensure!(crate::project::hash_file(destination)?==hash,"目标副本校验失败。");Ok(hash)
    })();
    drop(output);
    if result.is_err(){let _=fs::remove_file(destination);}
    result
}

fn destination_dir(root:&Path,photo:&Photo,layout:Layout)->Result<PathBuf>{
    let organize=match layout{Layout::Day=>lightcraft_engine::import::Organize::ByDay,Layout::Month=>lightcraft_engine::import::Organize::ByMonth,Layout::Flat=>lightcraft_engine::import::Organize::Flat};
    let mut directory=root.to_path_buf();
    for part in organize.folders(photo){
        ensure!(matches!(Path::new(&part).components().next(),Some(std::path::Component::Normal(_)))&&Path::new(&part).components().count()==1,"日期目录无效。");
        directory.push(part);check_path(&directory)?;fs::create_dir_all(&directory)?;
        ensure!(directory.canonicalize()?.starts_with(root),"目标超出归档目录。");
    }
    Ok(directory)
}

fn place(source:&Path,root:&Path,photo:&Photo,layout:Layout,cancel:&AtomicBool)->Result<Staged>{
    let directory=destination_dir(root,photo,layout)?;let sidecars=sidecars(source)?;
    let stem=source.file_stem().context("来源缺少文件名。")?.to_string_lossy();let ext=source.extension().map(|s|format!(".{}",s.to_string_lossy())).unwrap_or_default();
    for suffix in 0..100_000{
        ensure!(!cancel.load(Ordering::Relaxed),"导入已取消。");
        let name=if suffix==0{format!("{stem}{ext}")}else{format!("{stem}-{suffix}{ext}")};
        let destination=directory.join(name);let mut plans=vec![(source.to_path_buf(),destination.clone(),false,false)];
        for (path,stem_named,shared) in &sidecars{
            let ext=path.extension().context("XMP 后缀无效。")?;
            let target=if *stem_named{destination.with_extension(ext)}else{PathBuf::from(format!("{}.{}",destination.to_string_lossy(),ext.to_string_lossy()))};
            plans.push((path.clone(),target,true,*shared));
        }
        if plans.iter().any(|(_,p,_,_)|p.exists()){continue;}
        let mut staged=Staged{pairs:vec![],committed:false};let mut collision=false;
        for (src,dst,sidecar,shared) in plans{
            match copy_verified(&src,&dst,cancel){
                Ok(hash)=>staged.pairs.push(Pair{source:src,destination:dst,hash,sidecar,shared}),
                Err(e)=>{
                    collision=e.downcast_ref::<std::io::Error>().is_some_and(|e|e.kind()==std::io::ErrorKind::AlreadyExists);
                    if !collision{return Err(e);}break;
                }
            }
        }
        if collision{continue;}return Ok(staged);
    }
    anyhow::bail!("目标同名文件过多，请选择其他归档目录。")
}

fn finish_move(staged:&Staged,cancel:&AtomicBool)->Result<Vec<String>>{
    ensure!(!cancel.load(Ordering::Relaxed),"已停止，来源文件保留。");
    for pair in &staged.pairs{
        check_path(&pair.source)?;check_path(&pair.destination)?;
        ensure!(crate::project::hash_file(&pair.source)?==pair.hash&&crate::project::hash_file(&pair.destination)?==pair.hash,"文件发生变化，来源保留：{}",pair.source.display());
    }
    let mut kept=vec![];
    for pair in &staged.pairs{
        if pair.sidecar&&pair.shared{kept.push(format!("共享 XMP 保留：{}",pair.source.display()));continue;}
        if let Err(e)=fs::remove_file(&pair.source){
            if !pair.sidecar{return Err(e.into());}
            kept.push(format!("XMP 来源保留：{}：{e}",pair.source.display()));
        }
    }
    Ok(kept)
}

pub fn run_import(lib:&mut Library,photos:Vec<Photo>,options:&ImportOptions,cancel:&AtomicBool,mut progress:impl FnMut(usize,usize))->Result<ImportReport>{
    validate_options(options)?;let mut report=ImportReport::default();
    let root=if options.mode==ImportMode::Add{None}else{fs::create_dir_all(&options.destination)?;check_path(&options.destination)?;Some(options.destination.canonicalize()?)};
    let total=photos.len();
    for (index,preview) in photos.into_iter().enumerate(){
        if cancel.load(Ordering::Relaxed){report.cancelled=true;break;}
        let source=PathBuf::from(library::photo_path(&preview).context("导入项无来源路径。")?);
        let mut receipt=Receipt{source:source.clone(),destination:None,result:"failed".into(),detail:String::new(),sidecars:vec![],sidecar_pairs:vec![]};
        let result=(||->Result<()>{
            check_path(&source)?;
            let mut photo=library::index_photo(&source)?;
            ensure!(photo.content_hash==preview.content_hash,"来源在预览后改变，请重新扫描。");
            let existing=lib.catalog.photos().find(|p|library::photo_path(p)==library::photo_path(&photo)||photo.content_hash.as_ref().is_some_and(|h|p.content_hash.as_ref()==Some(h))).cloned();
            if let Some(existing)=existing{
                if existing.deleted&&options.restore{
                    ensure!(library::photo_path(&existing).is_some_and(|p|Path::new(p).is_file()),"原照片库记录离线，请先恢复路径再导入。");
                    lib.apply(vec![Op::SetDeleted{id:existing.id,deleted:false}])?;receipt.result="restored".into();receipt.destination=library::photo_path(&existing).map(PathBuf::from);
                }else{receipt.result="skipped".into();receipt.detail="照片库已有同路径或同内容记录，来源文件保留。".into();}
                return Ok(());
            }
            if options.mode==ImportMode::Add{lib.add(vec![photo])?;receipt.result="added".into();receipt.destination=Some(source.clone());return Ok(());}
            let root=root.as_ref().context("归档目标未准备。")?;
            ensure!(!source.canonicalize()?.starts_with(root),"来源已经位于归档目标内，请使用添加原位置。");
            let mut staged=place(&source,root,&photo,options.layout,cancel)?;
            let destination=staged.pairs.first().context("归档副本为空。")?.destination.canonicalize()?;
            ensure!(library::index_photo(&destination)?.content_hash==photo.content_hash,"来源在归档期间改变，导入未提交。");
            photo.source=Source::File{path:destination.to_str().context("归档路径不是有效 Unicode。")?.into()};
            photo.file_name=destination.file_name().context("归档文件名无效。")?.to_string_lossy().into_owned();
            if cancel.load(Ordering::Relaxed){anyhow::bail!("导入已取消，来源保持原样。");}
            ensure!(lib.add(vec![photo])?==1,"归档路径已有索引，来源保留。");
            staged.committed=true;
            receipt.destination=Some(destination);receipt.sidecars=staged.pairs.iter().filter(|p|p.sidecar).map(|p|p.destination.clone()).collect();
            receipt.sidecar_pairs=staged.pairs.iter().filter(|p|p.sidecar).map(|p|(p.source.clone(),p.destination.clone())).collect();
            receipt.result="copied".into();
            if options.mode==ImportMode::Move{
                match finish_move(&staged,cancel){
                    Ok(kept)=>{receipt.result="moved".into();receipt.detail=kept.join("\n");},
                    Err(e)=>{receipt.result="source_kept".into();receipt.detail=format!("副本与照片库记录已保存；来源保留：{e:#}");}
                }
            }
            Ok(())
        })();
        if let Err(e)=result{receipt.detail=format!("{e:#}");}
        report.items.push(receipt);progress(index+1,total);
    }
    report.cancelled|=cancel.load(Ordering::Relaxed);
    // Receipt storage is separate from image data. A report-write failure never invalidates
    // already durable imports or triggers source deletion.
    use lightcraft_catalog::Store;
    let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let name=format!("import-{}-{stamp}.json",std::process::id());
    let directory=lib.directory.join("import-reports");
    if let Ok(mut store)=lightcraft_catalog::FsStore::open(&directory){if let Ok(bytes)=serde_json::to_vec_pretty(&report){if store.write_atomic(&name,&bytes).is_ok(){report.report_path=Some(directory.join(name));}}}
    Ok(report)
}
