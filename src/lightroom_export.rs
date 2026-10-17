//! Experimental management-only Lightroom catalog writer. The selected template supplies
//! schema, never photo rows or account data. Originals and the template remain read-only.
use anyhow::{Context, Result, ensure};
use lightcraft_catalog::{Catalog, PhotoId, Source, Flag};
use std::{collections::{HashMap, HashSet, BTreeMap}, fs, path::{Path,PathBuf}, sync::atomic::{AtomicBool,Ordering}};
use crate::{sqlite_windows::{Db,Value as V}, project::hash_file};

pub const VERSION: &str = "1400000";
#[derive(Clone)]
pub struct Export {pub catalog:Catalog,pub selected:Option<HashSet<PhotoId>>,pub template:PathBuf}
#[derive(Debug,serde::Serialize)]
pub struct Report {pub output:PathBuf,pub photos:usize,pub files:usize,pub collections:usize,pub keywords:usize,pub virtual_copies:usize,pub smart_materialized:usize,pub template_version:String,pub readback_passed:bool,pub lightroom_verified:bool,pub warnings:Vec<String>}
fn check(cancel:&AtomicBool)->Result<()>{ensure!(!cancel.load(Ordering::Relaxed),"导出已取消");Ok(())}
fn inactive(path:&Path)->Result<()>{
    for suffix in ["-wal","-journal",".lock"] {let p=PathBuf::from(format!("{}{suffix}",path.display()));
        ensure!(!p.exists() || (suffix!=".lock" && fs::metadata(&p)?.len()==0),"请关闭正在使用模板目录的程序：{}",p.display());}
    Ok(())
}
fn ident(s:&str)->Result<String>{ensure!(!s.is_empty()&&s.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_'),"模板包含不支持的标识符");Ok(format!("\"{s}\""))}
struct Writer {db:Db,next:i64,seed:String}
impl Writer {
    fn id(&mut self)->i64{let n=self.next;self.next+=1;n}
    fn guid(&self,id:i64)->String {use sha2::{Digest,Sha256};let mut b=Sha256::digest(format!("{}:{id}",self.seed));b[6]=(b[6]&15)|64;b[8]=(b[8]&63)|128;let h:String=b[..16].iter().map(|n|format!("{n:02x}")).collect();format!("{}-{}-{}-{}-{}",&h[..8],&h[8..12],&h[12..16],&h[16..20],&h[20..])}
    fn insert(&mut self,table:&str,fields:&[(&str,V)])->Result<i64>{let id=self.id();let mut names=vec!["id_local".to_string()];let mut values=vec![V::Int(id)];for (name,v) in fields{names.push(ident(name)?);values.push(v.clone());}
        let sql=format!("INSERT INTO {} ({}) VALUES ({})",ident(table)?,names.join(","),vec!["?";values.len()].join(","));self.db.query(&sql,&values).with_context(||format!("写入 {table}"))?;Ok(id)}
    fn entity(&mut self,table:&str,fields:&[(&str,V)])->Result<i64>{let mut f=fields.to_vec();f.push(("id_global",self.guid(self.next).into()));self.insert(table,&f)}
    fn variable(&mut self,name:&str,kind:&str,value:V)->Result<()> {self.entity("Adobe_variablesTable",&[("name",name.into()),("type",kind.into()),("value",value)])?;Ok(())}
}
fn normalized(path:&str)->String {let p=if let Some(s)=path.strip_prefix("\\\\?\\UNC\\"){format!("//{s}")}else{path.strip_prefix("\\\\?\\").unwrap_or(path).to_string()};p.replace('\\',"/")}
fn path_key(path:&str)->String{normalized(path).to_lowercase()}
fn pick(p:&lightcraft_catalog::Photo)->i64{match p.flag{Flag::Pick=>1,Flag::Reject=>-1,Flag::None=>0}}
fn label(p:&lightcraft_catalog::Photo)->String{p.label.map(|l|format!("{l:?}")).unwrap_or_default()}
fn words(words:&[String])->Vec<String>{let mut out:Vec<_>=words.iter().map(|w|w.split('|').map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>().join("|")).filter(|s|!s.is_empty()).collect();out.sort();out.dedup();out}

impl Export {
    pub fn run(&self,output:&Path,cancel:&AtomicBool)->Result<Report>{
        check(cancel)?;ensure!(output.extension().is_some_and(|s|s.eq_ignore_ascii_case("lrcat")),"请选择 .lrcat 文件名");ensure!(!output.exists(),"禁止覆盖已有文件");
        let template=fs::canonicalize(&self.template).context("无法读取结构模板")?;inactive(&template)?;
        ensure!(fs::metadata(&template)?.len()<=1<<30,"模板超过 1 GiB");let original_hash=hash_file(&template)?;
        let source=Db::read_only(&template)?;
        let version=source.query("SELECT value FROM Adobe_variablesTable WHERE name='Adobe_DBVersion'",&[])?;
        ensure!(version.len()==1&&version[0][0].text()?==VERSION,"目前仅支持旧目录版本 {VERSION}");
        let crs=source.query("SELECT value FROM Adobe_variablesTable WHERE name='Adobe_CrsVersion'",&[])?;
        ensure!(crs.len()==1,"模板缺少显影格式版本");let crs=crs[0][0].clone();
        let schema=source.query("SELECT type,name,sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type,name",&[])?;
        ensure!(schema.len()<=2048,"模板结构过大");
        let mut keep:HashSet<_>=self.catalog.photos().filter(|p|!p.deleted&&self.selected.as_ref().is_none_or(|s|s.contains(&p.id))).map(|p|p.id).collect();
        for id in keep.clone(){if let Some(master)=self.catalog.photo(id).and_then(|p|p.copy_of){let p=self.catalog.photo(master).context("虚拟副本缺少原始记录")?;ensure!(p.copy_of.is_none(),"不支持嵌套虚拟副本");keep.insert(master);}}
        ensure!(self.selected.is_none()||!keep.is_empty(),"没有可导出的选片");
        let mut photos:Vec<_>=keep.iter().map(|id|self.catalog.photo(*id).unwrap()).collect();photos.sort_by_key(|p|(p.copy_of.is_some(),p.id));
        let parent=output.parent().context("保存目录不存在")?;let staging=tempfile::Builder::new().prefix(".luminacraft-lrcat-").tempdir_in(parent)?;let staged=staging.path().join("catalog.lrcat");
        let mut w=Writer{db:Db::new(&staged)?,next:1,seed:format!("{}-{:?}",std::process::id(),std::time::SystemTime::now())};
        w.db.query("PRAGMA trusted_schema=OFF",&[])?;w.db.query("BEGIN IMMEDIATE",&[])?;
        for r in &schema{check(cancel)?;let kind=r[0].text()?;ident(r[1].text()?)?;ensure!(matches!(kind,"table"|"index"|"trigger"),"不支持的模板结构类型");if kind=="table"{let sql=r[2].text()?;ensure!(sql.trim_start().to_uppercase().starts_with("CREATE TABLE "),"不支持虚拟表模板");w.db.query(sql,&[])?;}}
        let mut ids=HashMap::new();let mut files=HashMap::new();let mut folders=HashMap::new();
        let keyword_root=w.entity("AgLibraryKeyword",&[("name",V::Null),("genealogy",V::from(""))])?;
        w.db.query("UPDATE AgLibraryKeyword SET genealogy=? WHERE id_local=?",&[format!("/6{keyword_root}").into(),keyword_root.into()])?;
        let mut keywords:BTreeMap<String,(i64,String)>=BTreeMap::new();
        let mut photo_keywords=HashMap::new();
        for photo in &photos {
            check(cancel)?;let Source::File{path}=&photo.source else{anyhow::bail!("演示照片没有可引用的原片路径")};
            let path=normalized(path);ensure!(Path::new(&path).is_absolute(),"原片路径必须为绝对路径");
            let file=if let Some(id)=files.get(&path_key(&path)){*id}else{
                // Each distinct directory is a root. This keeps offline/UNC paths intact without
                // inventing parent folder records or relying on the receiver's drive layout.
                let p=Path::new(&path);let dir=normalized(p.parent().context("照片缺少父目录")?.to_str().context("非 Unicode 路径")?);let dir=format!("{}/",dir.trim_end_matches('/'));
                let folder=if let Some(id)=folders.get(&path_key(&dir)){*id}else{let root=w.entity("AgLibraryRootFolder",&[("absolutePath",dir.clone().into()),("name",dir.trim_end_matches('/').into())])?;let id=w.entity("AgLibraryFolder",&[("rootFolder",root.into()),("pathFromRoot","".into())])?;folders.insert(path_key(&dir),id);id};
                let filename=p.file_name().context("无效原片文件名")?.to_string_lossy().to_string();let ext=p.extension().unwrap_or_default().to_string_lossy().to_string();
                let id=w.entity("AgLibraryFile",&[("folder",folder.into()),("baseName",p.file_stem().unwrap_or_default().to_string_lossy().to_string().into()),("extension",ext.clone().into()),("idx_filename",filename.clone().into()),("lc_idx_filename",filename.to_lowercase().into()),("lc_idx_filenameExtension",ext.to_lowercase().into()),("originalFilename",filename.into())])?;files.insert(path_key(&path),id);id};
            let master=photo.copy_of.map(|id|ids.get(&id).copied().context("虚拟副本原始记录尚未生成")).transpose()?;
            let raw=matches!(photo.format.to_uppercase().as_str(),"ARW"|"CR2"|"CR3"|"NEF"|"NRW"|"RAF"|"ORF"|"RW2"|"PEF"|"DNG"|"RAW");
            let id=w.entity("Adobe_images",&[("rootFile",file.into()),("fileFormat",if raw{"RAW".into()}else{photo.format.to_uppercase().into()}),("fileWidth",(photo.width as i64).into()),("fileHeight",(photo.height as i64).into()),("aspectRatioCache",V::Real(photo.width as f64/photo.height.max(1) as f64)),("rating",(photo.rating as i64).into()),("pick",pick(photo).into()),("colorLabels",label(photo).into()),("captureTime",photo.captured.clone().map(V::from).unwrap_or(V::Null)),("masterImage",master.map(V::Int).unwrap_or(V::Null)),("copyName",photo.copy_name.clone().map(V::from).unwrap_or(V::Null)),("orientation","AB".into()),("pyramidIDCache","none".into())])?;
            ids.insert(photo.id,id);
            w.entity("Adobe_AdditionalMetadata",&[("image",id.into()),("isRawFile",(raw as i64).into())])?;
            w.insert("AgLibraryIPTC",&[("image",id.into()),("caption",photo.meta.caption.clone().into()),("copyright",photo.meta.copyright.clone().into()),("altTextAccessibility",photo.meta.alt_text.clone().into()),("extDescrAccessibility",photo.meta.extended_description.clone().into())])?;
            w.insert("AgLibraryImageAttributes",&[("image",id.into())])?;
            let canonical=words(&photo.meta.keywords);photo_keywords.insert(photo.id,canonical.clone());
            for word in &canonical {let tag=add_keyword(&mut w,&mut keywords,keyword_root,word)?;w.insert("AgLibraryKeywordImage",&[("image",id.into()),("tag",tag.into())])?;}
        }
        if self.selected.is_none(){for word in self.catalog.listed_keywords(){add_keyword(&mut w,&mut keywords,keyword_root,&word.path)?;}}
        let mut albums:Vec<_>=self.catalog.albums().collect();albums.sort_by_key(|a|a.id);let mut album_ids=HashMap::new();let mut genealogy=HashMap::<_,String>::new();let mut pending=albums.clone();let mut smart=0;
        while !pending.is_empty(){let before=pending.len();let mut rest=Vec::new();for a in pending{check(cancel)?;if a.parent.is_some_and(|p|!album_ids.contains_key(&p)){rest.push(a);continue;}
            let id=w.next;let parent_gene=a.parent.and_then(|p|genealogy.get(&p)).cloned().unwrap_or_default();let gene=format!("{parent_gene}/{}{id}",if a.folder{2}else{1});
            let id=w.insert("AgLibraryCollection",&[("name",a.name.clone().into()),("creationId",if a.folder{"com.adobe.ag.library.group".into()}else{"com.adobe.ag.library.collection".into()}),("genealogy",gene.clone().into()),("parent",a.parent.and_then(|p|album_ids.get(&p)).copied().map(V::Int).unwrap_or(V::Null)),("systemOnly",0.into())])?;
            album_ids.insert(a.id,id);genealogy.insert(a.id,gene);if a.smart.is_some(){smart+=1;}
            for (pos,photo) in self.catalog.album_photos(a.id).iter().filter(|id|keep.contains(id)).enumerate(){w.insert("AgLibraryCollectionImage",&[("collection",id.into()),("image",ids[photo].into()),("positionInCollection",format!("{:012}",pos).into())])?;}
        }ensure!(rest.len()<before,"集合层级存在循环或缺失父级");pending=rest;}
        w.variable("Adobe_DBVersion","string",VERSION.into())?;w.variable("Adobe_CrsVersion","string",crs)?;
        w.variable("Adobe_storeProviderID","string",w.guid(0).into())?;
        w.variable("AgLibraryKeyword_rootTagID","integer",keyword_root.into())?;
        for name in ["AgMetadata_autoWriteXmp","isOKToInitializeSync","ozAutoSyncEnabled"]{w.variable(name,"boolean",0.into())?;}
        w.variable("LR4ToneCurvesOkOrWereRepaired","boolean",1.into())?;
        w.variable("libraryDefaultSmartCollectionsHaveBeenAdded","boolean",1.into())?;
        w.variable("Adobe_entityIDCounter","integer",(w.next+100).into())?;
        w.db.query("INSERT INTO MigrationSchemaVersion(version) VALUES('1')",&[])?;
        // Adobe's triggers use extended functions (e.g. three-argument instr). Install only
        // after inserts; no trigger executes in the Windows SQLite writer.
        for r in &schema{check(cancel)?;if r[0].text()?!="table"{w.db.query(r[2].text()?,&[])?;}}
        w.db.query("COMMIT",&[])?;
        ensure!(w.db.query("PRAGMA integrity_check",&[])?==vec![vec![V::from("ok")]],"目录完整性校验失败");drop(w);
        check(cancel)?;let data=lightcraft_engine::lightroom_catalog::read(&staged).map_err(anyhow::Error::msg)?;
        ensure!(data.warnings.is_empty(),"LightCraft 回读出现警告：{:?}",data.warnings);ensure!(data.photos.len()==photos.len(),"LightCraft 回读照片数量不一致");
        for p in &photos{let row=data.photos.iter().find(|r|r.source_id==ids[&p.id]).context("回读缺失照片")?;
            ensure!(path_key(&row.path)==path_key(match &p.source{Source::File{path}=>path,_=>unreachable!()})&&row.image["rating"].as_i64()==Some(p.rating as i64)&&row.image["pick"].as_i64()==Some(pick(p))&&row.image["colorLabels"].as_str()==Some(label(p).as_str())&&words(&row.keywords)==photo_keywords[&p.id],"LightCraft 回读照片元数据不一致");
            ensure!(row.image.get("masterImage").and_then(|v|v.as_i64())==p.copy_of.map(|id|ids[&id]),"虚拟副本关系不一致");
        }
        ensure!(data.collections.len()==albums.len(),"集合数量不一致");
        for a in &albums{let id=album_ids[&a.id];let row=data.collections.iter().find(|r|r["id_local"].as_i64()==Some(id)).context("回读缺少集合")?;ensure!(row["name"].as_str()==Some(a.name.as_str())&&row.get("parent").and_then(|v|v.as_i64())==a.parent.map(|p|album_ids[&p]),"集合层级不一致");let actual:HashSet<_>=data.members.iter().filter(|r|r["collection"].as_i64()==Some(id)).filter_map(|r|r["image"].as_i64()).collect();let expected:HashSet<_>=self.catalog.album_photos(a.id).iter().filter_map(|id|ids.get(id).copied()).collect();ensure!(actual==expected,"集合成员不一致");}
        drop(source);inactive(&template)?;ensure!(hash_file(&template)?==original_hash,"模板在导出期间发生变化，未提交输出");
        check(cancel)?;fs::OpenOptions::new().write(true).open(&staged)?.sync_all().context("刷新新目录文件")?;
        // Same-volume hard link commits atomically and fails if the destination exists.
        fs::hard_link(&staged,output).context("无法提交文件（目标已存在或磁盘不支持硬链接）")?;
        let mut warnings=vec!["实验版：LightCraft 回读已通过；尚未由 Lightroom Classic 实际打开验收。".into(),"仅目录管理数据；不包含显影、蒙版、修图、历史和 AI 结果，请同时保留 .lumcat。原片继续引用原位置。".into()];
        if smart>0{warnings.push(format!("{smart} 个智能集合已保存为当前成员快照。"));}
        Ok(Report{output:output.into(),photos:photos.len(),files:files.len(),collections:albums.len(),keywords:keywords.len(),virtual_copies:photos.iter().filter(|p|p.copy_of.is_some()).count(),smart_materialized:smart,template_version:VERSION.into(),readback_passed:true,lightroom_verified:false,warnings})
    }
}
fn add_keyword(w:&mut Writer,keywords:&mut BTreeMap<String,(i64,String)>,root:i64,path:&str)->Result<i64>{let parts:Vec<_>=path.split('|').map(str::trim).filter(|s|!s.is_empty()).collect();ensure!(!parts.is_empty()&&parts.len()<63,"无效关键词层级");let mut parent=root;let mut gene=format!("/6{root}");let mut key=String::new();for part in parts{if !key.is_empty(){key.push('|');}key.push_str(&part.to_lowercase());if let Some((id,g))=keywords.get(&key){parent=*id;gene=g.clone();continue;}let id=w.next;gene=format!("{gene}/6{id}");parent=w.entity("AgLibraryKeyword",&[("name",part.into()),("lc_name",part.to_lowercase().into()),("parent",parent.into()),("genealogy",gene.clone().into())])?;keywords.insert(key.clone(),(parent,gene.clone()));}Ok(parent)}

#[cfg(all(test,windows))]
mod tests {
    use super::*;
    use lightcraft_catalog::{Photo,Op,Album,AlbumId,ColorLabel};
    fn template(path:&Path){
        let db=Db::new(path).unwrap();
        // Synthetic schema only. No proprietary template or personal catalog data in tests.
        for (table,cols) in [
            ("Adobe_variablesTable","id_global UNIQUE,name,type,value"),
            ("Adobe_images","id_global UNIQUE,rootFile,fileFormat,fileWidth,fileHeight,aspectRatioCache,rating,pick,colorLabels,captureTime,masterImage,copyName,orientation,pyramidIDCache"),
            ("AgLibraryRootFolder","id_global UNIQUE,absolutePath,name,relativePathFromCatalog"),
            ("AgLibraryFolder","id_global UNIQUE,rootFolder,pathFromRoot"),
            ("AgLibraryFile","id_global UNIQUE,folder,baseName,extension,idx_filename,lc_idx_filename,lc_idx_filenameExtension,originalFilename"),
            ("AgLibraryKeyword","id_global UNIQUE,name,lc_name,parent,genealogy"),
            ("AgLibraryKeywordImage","image,tag"),
            ("Adobe_AdditionalMetadata","id_global UNIQUE,image,isRawFile,xmp TEXT DEFAULT ''"),
            ("AgLibraryIPTC","image,caption,copyright,altTextAccessibility,extDescrAccessibility"),
            ("AgLibraryImageAttributes","image"),
            ("AgLibraryCollection","name,creationId,genealogy,parent,systemOnly"),
            ("AgLibraryCollectionImage","collection,image,positionInCollection"),
            ("privateSecret","value"),
        ] {db.query(&format!("CREATE TABLE {table}(id_local INTEGER PRIMARY KEY,{cols})"),&[]).unwrap();}
        db.query("CREATE TABLE MigrationSchemaVersion(version TEXT PRIMARY KEY)",&[]).unwrap();
        db.query("INSERT INTO Adobe_variablesTable(name,value) VALUES('Adobe_DBVersion','1400000')",&[]).unwrap();
        db.query("INSERT INTO Adobe_variablesTable(name,value) VALUES('Adobe_CrsVersion','17.0.0')",&[]).unwrap();
        db.query("INSERT INTO Adobe_variablesTable(name,value) VALUES('Adobe_lrMobileOzUserId','PRIVATE-ACCOUNT-SENTINEL')",&[]).unwrap();
        db.query("INSERT INTO privateSecret(value) VALUES('PRIVATE-PHOTO-SENTINEL')",&[]).unwrap();
        db.query("CREATE INDEX filenameIndex ON AgLibraryFile(lc_idx_filename)",&[]).unwrap();
        db.query("CREATE TRIGGER metadataTrigger AFTER INSERT ON Adobe_images BEGIN UPDATE privateSecret SET value='TRIGGER'; END",&[]).unwrap();
    }
    fn fixture(base:&Path)->Export {
        let template_path=base.join("template.lrcat");template(&template_path);let mut catalog=Catalog::default();
        let file=base.join("offline-中文.png");let mut p=Photo::new(PhotoId(10),Source::File{path:file.to_string_lossy().into_owned()},"offline-中文.png","PNG",24,16,"2026-10-15T12:00:00");p.rating=4;p.flag=Flag::Pick;p.label=Some(ColorLabel::Green);p.meta.keywords=vec!["旅程|北京".into()];p.captured=Some("2025-01-02T03:04:05".into());
        catalog.apply(Op::AddPhoto{photo:Box::new(p.clone())}).unwrap();p.id=PhotoId(11);p.copy_of=Some(PhotoId(10));p.copy_name=Some("黑白版本".into());p.rating=5;catalog.apply(Op::AddPhoto{photo:Box::new(p)}).unwrap();
        let mut folder=Album::new(AlbumId(1),"旅程");folder.folder=true;let mut album=Album::new(AlbumId(2),"北京");album.parent=Some(folder.id);album.photos=vec![PhotoId(11)];let mut smart=Album::new(AlbumId(3),"智能快照");smart.smart=Some(Box::default());
        for album in [folder,album,smart]{catalog.apply(Op::AddAlbum{album}).unwrap();}
        Export{catalog,selected:None,template:template_path}
    }
    #[test]fn selected_virtual_copy_offline_paths_and_collection_hierarchy_roundtrip_without_private_rows(){
        let tmp=tempfile::tempdir().unwrap();let mut e=fixture(tmp.path());e.selected=Some(HashSet::from([PhotoId(11)]));let sha=hash_file(&e.template).unwrap();let modified=fs::metadata(&e.template).unwrap().modified().unwrap();let output=tmp.path().join("out.lrcat");let r=e.run(&output,&AtomicBool::new(false)).unwrap();
        assert_eq!((r.photos,r.files,r.virtual_copies,r.collections,r.smart_materialized),(2,1,1,3,1));assert!(r.readback_passed&&!r.lightroom_verified);let db=Db::read_only(&output).unwrap();assert!(db.query("SELECT * FROM privateSecret",&[]).unwrap().is_empty());assert!(db.query("SELECT value FROM Adobe_variablesTable WHERE name='Adobe_lrMobileOzUserId'",&[]).unwrap().is_empty());
        assert_eq!(hash_file(&e.template).unwrap(),sha);assert_eq!(fs::metadata(&e.template).unwrap().modified().unwrap(),modified);
        let mut full=lightcraft_engine::Session::new().with_fs();full.open_library(tmp.path().join("readback"),false).unwrap();full.execute("library.importLightroom",&serde_json::json!({"path":output})).unwrap();assert_eq!(full.catalog.len(),2);let copy=full.catalog.photos().find(|p|p.copy_of.is_some()).unwrap();assert_eq!(copy.copy_name.as_deref(),Some("黑白版本"));assert_eq!(copy.rating,5);assert_eq!(copy.label,Some(ColorLabel::Green));assert_eq!(copy.meta.keywords,vec!["旅程|北京"]);assert_eq!(copy.captured.as_deref(),Some("2025-01-02T03:04:05"));assert_eq!(full.catalog.albums().find(|a|a.name=="智能快照").unwrap().photos.len(),2);full.close_library().unwrap();
        assert!(!fs::read_dir(tmp.path()).unwrap().any(|f|f.unwrap().file_name().to_string_lossy().starts_with(".luminacraft-lrcat-")));
    }
    #[test]fn cancellation_overwrite_wrong_version_active_template_and_empty_selection_fail_without_outputs(){
        let tmp=tempfile::tempdir().unwrap();let mut e=fixture(tmp.path());let output=tmp.path().join("out.lrcat");assert!(e.run(&output,&AtomicBool::new(true)).is_err());assert!(!output.exists());fs::write(&output,"existing").unwrap();assert!(e.run(&output,&AtomicBool::new(false)).is_err());assert_eq!(fs::read(&output).unwrap(),b"existing");fs::remove_file(&output).unwrap();
        let wal=PathBuf::from(format!("{}-wal",e.template.display()));fs::write(&wal,"active").unwrap();assert!(e.run(&output,&AtomicBool::new(false)).is_err());fs::remove_file(wal).unwrap();e.selected=Some(HashSet::new());assert!(e.run(&output,&AtomicBool::new(false)).is_err());e.selected=None;
        let db=Db::open(e.template.to_str().unwrap(),2).unwrap();db.query("UPDATE Adobe_variablesTable SET value='999' WHERE name='Adobe_DBVersion'",&[]).unwrap();drop(db);assert!(e.run(&output,&AtomicBool::new(false)).is_err());assert!(!output.exists());
    }
    #[test]fn schema_failure_and_orphan_copy_clean_staging_and_preserve_source(){
        let tmp=tempfile::tempdir().unwrap();let mut e=fixture(tmp.path());let mut p=(**e.catalog.photo(PhotoId(11)).unwrap()).clone();p.id=PhotoId(12);p.copy_of=Some(PhotoId(999));e.catalog.apply(Op::AddPhoto{photo:Box::new(p)}).unwrap();let output=tmp.path().join("out.lrcat");assert!(e.run(&output,&AtomicBool::new(false)).is_err());assert!(!output.exists());
        e.selected=Some(HashSet::from([PhotoId(10)]));let db=Db::open(e.template.to_str().unwrap(),2).unwrap();db.query("DROP TABLE AgLibraryFile",&[]).unwrap();drop(db);let sha=hash_file(&e.template).unwrap();assert!(e.run(&output,&AtomicBool::new(false)).is_err());assert_eq!(hash_file(&e.template).unwrap(),sha);assert!(!output.exists());assert!(!fs::read_dir(tmp.path()).unwrap().any(|f|f.unwrap().file_name().to_string_lossy().starts_with(".luminacraft-lrcat-")));
    }
}
