//! Source-preserving folder management plans. All writes are catalog ops; disk scans are read-only.
use super::*;
use std::collections::HashMap;
use lightcraft_catalog::{Filter,Sort};

#[derive(Clone)]
pub struct FolderRelink {pub id:PhotoId,pub from:PathBuf,pub to:PathBuf}
pub struct FolderPlan {
    pub revision:u64,pub folder:String,pub destination:Option<String>,pub new_photos:Vec<Photo>,
    pub missing:Vec<PhotoId>,pub relinks:Vec<FolderRelink>,pub errors:Vec<String>,
}
#[derive(Clone,Copy)]
pub struct FolderChoices {pub import_new:bool,pub relink_moved:bool,pub remove_missing:bool}
impl Default for FolderChoices{fn default()->Self{Self{import_new:true,relink_moved:true,remove_missing:false}}}
pub fn folder_ids(catalog:&Catalog,path:&str)->Vec<PhotoId>{catalog.query(&Filter{library_folder:Some(path.into()),..Default::default()},&Sort::default())}
pub fn folder_remove_ops(catalog:&Catalog,path:&str)->Result<Vec<Op>>{
    ensure!(!lightcraft_catalog::folders::is_disk_root(path),"请选择具体文件夹，不移除整块磁盘或网络共享");
    ensure!(!lightcraft_catalog::query::folder_key(path).is_empty(),"文件夹路径无效");
    let ids=folder_ids(catalog,path);ensure!(!ids.is_empty(),"此文件夹没有已入库照片");
    Ok(ids.into_iter().map(|id|Op::SetDeleted{id,deleted:true}).collect())
}
fn relink(catalog:&Catalog,id:PhotoId,target:&Path)->Result<FolderRelink>{
    let record=catalog.photo(id).context("照片记录已不存在")?;
    let from=PathBuf::from(photo_path(record).context("照片没有文件来源")?);
    ensure!(!from.exists(),"旧原片仍存在，请使用导入或归档，而不是重新定位");
    let target=target.canonicalize()?;
    let probe=index_photo(&target)?;
    ensure!(record.content_hash.as_ref().is_some_and(|h|probe.content_hash.as_ref()==Some(h)),"目标文件内容与照片库记录不同，未修改关联：{}",target.display());
    Ok(FolderRelink{id,from,to:target})
}
pub fn locate_folder(catalog:&Catalog,folder:&str,destination:&Path,cancel:&AtomicBool)->Result<FolderPlan>{
    ensure!(destination.is_dir(),"请选择目标文件夹");let destination=destination.canonicalize()?;
    let mut plan=FolderPlan{revision:catalog.revision,folder:folder.into(),destination:Some(destination.to_string_lossy().into_owned()),new_photos:vec![],missing:vec![],relinks:vec![],errors:vec![]};
    for id in folder_ids(catalog,folder){
        ensure!(!cancel.load(Ordering::Relaxed),"操作已取消");
        let photo=catalog.photo(id).context("照片记录不存在")?;let path=photo_path(photo).context("照片没有来源")?;
        if Path::new(path).exists(){continue;}
        plan.missing.push(id);
        let rest=lightcraft_catalog::query::folder_rest(path,folder).context("文件夹路径不匹配")?;
        ensure!(rest.iter().all(|part|!part.is_empty() && part!=".." && part!="." && !part.contains(['/', '\\', ':'])),"相对路径不安全");
        let target=rest.into_iter().fold(destination.clone(),|path,part|path.join(part));
        if target.is_file(){match relink(catalog,id,&target){Ok(r)=>plan.relinks.push(r),Err(e)=>plan.errors.push(e.to_string())}}
    }
    ensure!(!plan.missing.is_empty(),"此文件夹没有缺失的原片");Ok(plan)
}
pub fn scan_folder(catalog:&Catalog,folder:&str,skip:&Path,cancel:Arc<AtomicBool>)->Result<FolderPlan>{
    ensure!(Path::new(folder).is_dir(),"文件夹离线，请先定位缺失的文件");
    let mut found=vec![];let mut errors=vec![];
    scan(vec![PathBuf::from(folder)],skip.to_path_buf(),cancel.clone(),|event|{match event{ScanEvent::Photo(p)=>found.push(*p),ScanEvent::Error(e)=>errors.push(e),ScanEvent::Done{..}=>{}}true});
    ensure!(!cancel.load(Ordering::Relaxed),"操作已取消");
    let ids=folder_ids(catalog,folder);
    let missing=ids.into_iter().filter(|id|catalog.photo(*id).and_then(|p|photo_path(p)).is_some_and(|p|!Path::new(p).is_file())).collect::<Vec<_>>();
    let known=catalog.photos().filter_map(|p|photo_path(p).map(lightcraft_catalog::query::folder_key)).collect::<HashSet<_>>();
    let originals=catalog.photos().filter_map(|p|photo_path(p).map(|path|(lightcraft_catalog::query::folder_key(path),p.content_hash.clone()))).collect::<HashMap<_,_>>();
    for p in &found{if let Some(path)=photo_path(p){if let Some(Some(expected))=originals.get(&lightcraft_catalog::query::folder_key(path)){
        if p.content_hash.as_ref()!=Some(expected){errors.push(format!("已入库原片内容改变，未覆盖原记录：{path}"));}
    }}}
    let new_photos=found.into_iter().filter(|p|photo_path(p).is_some_and(|p|!known.contains(&lightcraft_catalog::query::folder_key(p)))).collect::<Vec<_>>();
    let mut by_hash:HashMap<String,Vec<PathBuf>>=HashMap::new();
    for p in &new_photos{if let (Some(hash),Some(path))=(&p.content_hash,photo_path(p)){by_hash.entry(hash.clone()).or_default().push(PathBuf::from(path));}}
    let mut relinks=vec![];
    for id in &missing{
        let Some(hash)=catalog.photo(*id).and_then(|p|p.content_hash.as_ref())else{continue};
        if let Some(paths)=by_hash.get(hash).filter(|paths|paths.len()==1){
            ensure!(!cancel.load(Ordering::Relaxed),"操作已取消");
            match relink(catalog,*id,&paths[0]){Ok(r)=>relinks.push(r),Err(e)=>errors.push(e.to_string())}
        }
    }
    Ok(FolderPlan{revision:catalog.revision,folder:folder.into(),destination:None,new_photos,missing,relinks,errors})
}
impl FolderPlan {
    pub fn ops(&self,catalog:&Catalog,choices:FolderChoices)->Result<Vec<Op>>{
        ensure!(catalog.revision==self.revision,"照片库在预览后已改变，请重新扫描");
        let mut ops=vec![];let mut relinked=HashSet::new();
        if choices.relink_moved{
            for item in &self.relinks{
                let current=relink(catalog,item.id,&item.to)?;
                ensure!(current.from==item.from,"照片来源在预览后已改变");
                ops.push(Op::Relink{id:item.id,file_name:item.to.file_name().context("文件名无效")?.to_string_lossy().into_owned(),source:Source::File{path:item.to.to_string_lossy().into_owned()},format:None});relinked.insert(item.id);
            }
            if let Some(to)=&self.destination{if self.missing.iter().all(|id|relinked.contains(id)){
                ops.extend(catalog.folder_records_follow(&self.folder,to));
            }}
        }
        if choices.remove_missing{
            ensure!(self.errors.is_empty(),"扫描存在错误，不能移除缺失记录；请先处理扫描错误");
            for id in &self.missing{if !relinked.contains(id){let p=catalog.photo(*id).context("照片记录已改变")?;ensure!(photo_path(p).is_some_and(|p|!Path::new(p).exists()),"原片已重新出现，请重新扫描");ops.push(Op::SetDeleted{id:*id,deleted:true});}}
        }
        if choices.import_new{
            let mut allocation=catalog.clone();
            let associated=self.relinks.iter().filter(|r|relinked.contains(&r.id)).map(|r|lightcraft_catalog::query::folder_key(&r.to.to_string_lossy())).collect::<HashSet<_>>();
            for preview in &self.new_photos{
                if photo_path(preview).is_some_and(|p|associated.contains(&lightcraft_catalog::query::folder_key(p))){continue;}
                let mut current=index_photo(Path::new(photo_path(preview).context("文件来源无效")?))?;
                ensure!(current.content_hash==preview.content_hash,"新照片在预览后已改变，请重新扫描");
                current.id=allocation.alloc_photo_id();ops.push(Op::AddPhoto{photo:Box::new(current)});
            }
        }Ok(ops)
    }
}
/// Update a loaded project's file references after a validated catalog relink.
/// Preserve hashes, edits, versions and the stable catalog identity.
pub fn follow_catalog_paths(project:&mut crate::project::Project,library:&Library)->Result<usize>{
    let mut next=project.clone();let mut changed=0;
    for photo in &mut next.photos{
        let Some(reference)=photo.catalog_ref.as_ref().filter(|r|crate::bridge::same_path(&r.library,&library.directory))else{continue};
        let Some(path)=library.catalog.photo(PhotoId(reference.id)).and_then(|p|photo_path(p)).map(PathBuf::from)else{continue};
        if crate::bridge::same_path(&path,&photo.source.path){continue;}
        ensure!(crate::project::hash_file(&path)?==photo.source.sha256,"重新定位的原片与工程校验值不符，工程未改写");
        if let Some(old)=&photo.source.xmp_path{if !old.exists(){
            let candidates=[path.with_extension("xmp"),PathBuf::from(format!("{}.xmp",path.display()))];
            let matched=candidates.into_iter().find(|p|photo.source.xmp_sha256.as_ref().is_some_and(|hash|crate::project::hash_file(p).is_ok_and(|h|&h==hash))).context("目标缺少内容一致的原 XMP，工程路径未改写")?;
            photo.source.xmp_path=Some(matched);
        }}
        photo.source.path=path;changed+=1;
    }
    next.validate()?;if changed>0{*project=next;}Ok(changed)
}
pub fn create_child_folder(parent:&Path,name:&str)->Result<PathBuf>{
    ensure!(parent.is_dir(),"父文件夹离线");
    ensure!(!name.is_empty() && name.len()<=180 && name==name.trim() && !name.ends_with('.') && !name.chars().any(|c|c.is_control() || "<>:\"/\\|?*".contains(c)) && name!="." && name!="..","请输入有效的子文件夹名称");
    let stem=name.split('.').next().unwrap_or("").to_ascii_uppercase();
    ensure!(!["CON","PRN","AUX","NUL","COM1","COM2","COM3","COM4","COM5","COM6","COM7","COM8","COM9","LPT1","LPT2","LPT3","LPT4","LPT5","LPT6","LPT7","LPT8","LPT9"].contains(&stem.as_str()),"文件夹名称为系统保留名");
    let parent=parent.canonicalize()?;let child=parent.join(name);std::fs::create_dir(&child)?;Ok(child)
}
