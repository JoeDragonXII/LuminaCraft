use crate::{engine::{self,RenderOutput,Source},project::{Photo,Project,FaceRegion,PublishRecord,hash_file}};
use std::{path::PathBuf,sync::{Arc,mpsc,atomic::{AtomicBool,Ordering}}};
use serde_json::Value;
use anyhow::{Result,ensure};

pub enum Job {
    Managed(Box<Job>,Arc<crate::task_gate::CommitGate>),
    LocalPreview(PublishRecord),
    LocalCopy(PublishRecord,PathBuf),
    Import(Vec<PathBuf>),
    Open(PathBuf),
    Save(Project,PathBuf),
    CatalogExport(crate::catalog_io::Export,PathBuf,bool),
    CatalogOpen(PathBuf),
    LightroomExport(crate::lightroom_export::Export,PathBuf),
    Preview(Photo,bool,usize),
    Faces(Photo),
    Xmp(Photo,PathBuf),
    Export(Photo,PathBuf,bool,bool),
    Connect(String,String),
    Publish(PublishRecord,String,Option<String>,bool),
}
pub enum Outcome {
    Imported(Vec<Photo>,Vec<String>),
    Opened(Project),
    Saved(PathBuf,String),
    CatalogExported(crate::catalog_io::Report,bool),
    CatalogOpened(crate::library::Library,Option<Project>,Option<String>),
    LightroomExported(crate::lightroom_export::Report),
    Preview(RenderOutput),
    LocalPreview(PathBuf,String,image::RgbaImage),
    LocalCopied(PublishRecord),
    Faces(Vec<FaceRegion>),
    Xmp(Photo),
    Exported(PublishRecord,Value),
    Connected(Vec<Value>),
    Published(PublishRecord),
}
pub struct Request {pub id:u64,pub job:Job,pub cancel:Arc<AtomicBool>}
pub struct Response {pub id:u64,pub result:std::result::Result<Outcome,String>}

pub fn start(ctx:eframe::egui::Context)->(mpsc::Sender<Request>,mpsc::Receiver<Response>){
    let (tx,rx)=mpsc::channel::<Request>();let (out,result)=mpsc::channel();
    std::thread::spawn(move || {
        let mut cached:Option<(String,usize,Source)>=None;
        for request in rx {
            let(job,gate)=match request.job{Job::Managed(job,gate)=>(*job,gate),job=>(job,Arc::new(crate::task_gate::CommitGate::default()))};
            let processed=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Result<Outcome>{
                ensure!(!request.cancel.load(Ordering::Relaxed)&&!gate.cancelled(),"操作已取消");
                match job {
                    Job::Managed(..)=>anyhow::bail!("不支持嵌套任务"),
                    Job::LocalPreview(record)=>Ok(Outcome::LocalPreview(record.output.clone(),record.output_sha256.clone(),record.preview()?)),
                    Job::LocalCopy(mut record,path)=>{record.copy_with_commit(&path,&||gate.begin_commit())?;record.output=path;record.asset_id=None;record.collection_id=None;record.lumina_base=None;Ok(Outcome::LocalCopied(record))},
                    Job::Import(paths)=>{
                        let mut photos=vec![];let mut errors=vec![];
                        for path in paths {ensure!(!request.cancel.load(Ordering::Relaxed),"操作已取消。");
                            match std::panic::catch_unwind(||engine::import_photo(&path)) {
                                Ok(Ok((photo,source)))=>{cached=Some((Source::preview_key(&photo,&photo.edit.develop),1200,source));photos.push(photo);},
                                Ok(Err(e))=>errors.push(format!("{}：{e:#}",path.display())),
                                Err(_)=>errors.push(format!("{}：原片解析异常，已跳过。",path.display())),
                            }
                        }
                        Ok(Outcome::Imported(photos,errors))
                    }
                    Job::Open(path)=>Ok(Outcome::Opened(Project::open(&path)?)),
                    Job::LightroomExport(export,path)=>{gate.begin_commit()?;Ok(Outcome::LightroomExported(export.run(&path,&request.cancel)?))},
                    Job::CatalogExport(export,path,open)=>{gate.begin_commit()?;Ok(Outcome::CatalogExported(if crate::catalog_file::is_file(&path){crate::catalog_file::export(export,&path,&request.cancel)?}else{export.run(&path,&request.cancel)?},open))},
                    Job::CatalogOpen(path)=>{
                        gate.begin_commit()?;
                        let dir=if crate::catalog_file::is_file(&path){crate::catalog_file::prepare(&path,&request.cancel)?}else{crate::catalog_io::resolve(&path)?};
                        ensure!(!crate::bridge::live_handoff(&dir)?,"完整版正在使用该目录，请返回主界面后再打开");
                        let lib=crate::library::Library::open(&dir)?;
                        let (project,warning)=if dir.join("project.json").is_file(){match crate::catalog_io::load_project(&dir,&lib){Ok(p)=>(Some(p),None),Err(e)=>(None,Some(format!("照片库已打开；编辑工程未载入：{e:#}")))}}else{(None,None)};
                        Ok(Outcome::CatalogOpened(lib,project,warning))
                    },
                    Job::Save(mut project,path)=>{project.save_with_commit(&path,&||gate.begin_commit())?;Ok(Outcome::Saved(path,project.digest()?))}
                    Job::Preview(photo,before,edge)=>{
                        let mut edit=photo.edit.clone();if before{edit.develop=photo.baseline.clone();edit.retouch=Default::default();}
                        let key=Source::preview_key(&photo,&edit.develop);
                        if cached.as_ref().is_none_or(|(hash,n,_)|hash!=&key||*n!=edge){
                            photo.source.verify()?;let source=Source::load_photo(&photo,&edit.develop,edge)?;cached=Some((key,edge,source));
                        }
                        let source=&cached.as_ref().ok_or_else(||anyhow::anyhow!("原片尚未读取。"))?.2;
                        if engine::prepare_profile(&photo,&edit)?{source.cache.clear();}
                        Ok(Outcome::Preview(engine::render_photo(source,&edit,edge,&request.cancel)?))
                    }
                    Job::Faces(photo)=>{
                        engine::prepare_profile(&photo,&photo.edit)?;
                        photo.source.verify()?;let source=Source::load_photo(&photo,&photo.edit.develop,1200)?;
                        let render=engine::render_photo(&source,&photo.edit,1200,&request.cancel)?;
                        let model=std::env::current_exe()?.parent().map(|p|p.join("assets/yunet.onnx")).filter(|p|p.is_file()).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/yunet.onnx"));
                        let faces=crate::retouch::detect_faces(&render.image,&render.frame,&model)?;
                        cached=Some((Source::preview_key(&photo,&photo.edit.develop),1200,source));Ok(Outcome::Faces(faces))
                    }
                    Job::Xmp(mut photo,path)=>{
                        photo.source.verify()?;ensure!(std::fs::metadata(&path)?.len()<=2*1024*1024,"XMP 文件过大。");
                        let source=Source::load(&photo.source.path,1200)?;let text=std::fs::read_to_string(&path)?;
                        let (settings,report)=engine::import_xmp(&text,&source)?;
                        photo.edit.develop=settings;photo.report=report;photo.source.original_xmp=Some(text);photo.source.xmp_sha256=Some(hash_file(&path)?);photo.source.xmp_path=Some(path.canonicalize()?);
                        Ok(Outcome::Xmp(photo))
                    }
                    Job::Export(photo,path,tiff,keep_gps)=>{
                        // Drop the proxy while rendering full resolution to bound peak memory.
                        cached=None;
                        let report=engine::export_photo_with_commit(&photo,&path,tiff,keep_gps,&request.cancel,&||gate.begin_commit())?;
                        let edit_sha256=report["edit_sha256"].as_str().ok_or_else(||anyhow::anyhow!("缺少导出指纹"))?.to_owned();
                        let output_sha256=hash_file(&path)?;
                        let record=PublishRecord{lumina_base:None,source_sha256:photo.source.sha256,edit_sha256,output:path,output_sha256,asset_id:None,collection_id:None,origin:None};
                        Ok(Outcome::Exported(record,report))
                    }
                    Job::Connect(..)|Job::Publish(..)=>anyhow::bail!("当前版本仅支持本地导出，Lumina 对接已停用"),
                }
            }));
            let result=match processed {Ok(r)=>r.map_err(|e|format!("{e:#}")),Err(_)=>Err(if gate.committing(){"任务在提交阶段异常，请检查目标文件；原片保持原样。"}else{"引擎异常，未提交目标文件；请重试。"}.into())};
            let result=if (gate.cancelled()||request.cancel.load(Ordering::Relaxed))&&!gate.committing(){Err("操作已取消；已完成的步骤会保留。".into())}else{result};
            let _=out.send(Response{id:request.id,result});ctx.request_repaint();
        }
    });
    (tx,result)
}

