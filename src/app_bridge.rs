use super::*;
use luminacraft::bridge::{self, Purpose, Request, Entry, Response};
pub(super) struct AdvancedTask { pub child:std::process::Child, pub request:Request, pub path:PathBuf, pub lease:bridge::HandoffLease }
impl StudioApp {
    pub(super) fn open_advanced(&mut self,purpose:Purpose) {
        if self.busy || self.library.scanning || self.ai.task.is_some() || self.ai.export_busy() {self.message="请等当前任务完成后打开完整版。".into();return;}
        let prepared=(||->anyhow::Result<AdvancedTask>{
            let lib=self.library.library.as_ref().ok_or_else(||anyhow::anyhow!("照片库未打开"))?;
            let token=format!("{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos());
            let library=if purpose==Purpose::Develop {
                self.project.photos.get(self.active).and_then(|p|p.catalog_ref.as_ref()).map(|c|c.library.clone()).unwrap_or_else(||lib.directory.clone()).canonicalize()?
            }else{lib.directory.canonicalize()?};
            let path=library.join("Bridge").join(&token).join("request.json");
            let mut entries=vec![];
            for offset in 0..self.project.photos.len() {
                    let photo=&self.project.photos[(self.active+offset)%self.project.photos.len()];
                    if purpose!=Purpose::Develop&&!photo.source.path.is_file(){continue;}
                    if photo.catalog_ref.as_ref().is_some_and(|c|!bridge::same_path(&c.library,&library)){continue;}
                    let id=photo.catalog_ref.as_ref().filter(|c|bridge::same_path(&c.library,&library)).map(|c|c.id)
                        .or_else(||if bridge::same_path(&lib.directory,&library){lib.catalog.photos().find(|p|p.copy_of.is_none() && luminacraft::library::photo_path(p).is_some_and(|p|bridge::same_path(std::path::Path::new(p),&photo.source.path))).map(|p|p.id.0)}else{None});
                    entries.push(Entry{id,path:photo.source.path.clone(),sha256:photo.source.sha256.clone(),develop:photo.edit.develop.clone()});
            }
            let selected=if bridge::same_path(&lib.directory,&library){self.library.selected_ids()}else{Vec::new()};
            let request=Request{schema:1,token,library:library.clone(),purpose,project:self.project.clone(),project_directory:self.directory.clone(),saved_digest:self.project.saved_digest.clone(),entries,selected};
            request.validate()?;bridge::write_new(&path,&request)?;
            let lease=bridge::HandoffLease::participate(&path)?;
            self.library.suspend()?;
            let mut command=std::process::Command::new(std::env::current_exe()?);
            command.args(["--advanced","--handoff"]).arg(&path).env("LUMINACRAFT_LIBRARY",&library).env_remove("LUMINACRAFT_SCREENSHOT");
            match command.spawn(){Ok(child)=>Ok(AdvancedTask{child,request,path,lease}),Err(e)=>{drop(lease);let _=self.library.resume(&library,&request.selected);Err(e.into())}}
        })();
        match prepared {Ok(task)=>{self.pending_preview=None;self.cancel.store(true,Ordering::Relaxed);self.id+=1;self.advanced=Some(task);},Err(e)=>{self.message=format!("无法打开完整版：{e:#}");self.library.message=self.message.clone();}}
    }
    pub(super) fn poll_advanced(&mut self,ctx:&egui::Context) {
        let finished=match &mut self.advanced {Some(t)=>match t.child.try_wait(){Ok(Some(_))=>true,Ok(None)=>false,Err(e)=>{self.message=format!("等待完整版：{e}");false}},None=>return};
        if !finished {ctx.request_repaint_after(std::time::Duration::from_millis(250));return;}
        let Some(task)=self.advanced.take() else{return};
        if let Err(e)=task.lease.upgrade(){self.message=e.to_string();self.advanced=Some(task);ctx.request_repaint_after(std::time::Duration::from_millis(500));return;}
        let mut result=(||->anyhow::Result<Response>{
            let response=bridge::read::<Response>(&task.path.with_file_name("result.json"))?;
            response.validate_for(&task.request)?;Ok(response)
        })();
        let browse=result.as_ref().ok().and_then(|r|r.browse.as_ref());
        let directory=browse.map(|b|b.library.as_path()).unwrap_or(&task.request.library);
        let ids=browse.map(|b|b.selected.as_slice()).unwrap_or_else(||result.as_ref().map(|r|r.selected.as_slice()).unwrap_or(&task.request.selected));
        if let Err(e)=self.library.resume(directory,ids){self.message=format!("照片库暂未恢复：{e:#}；交接文件保留在 {}",task.path.display());self.advanced=Some(task);ctx.request_repaint_after(std::time::Duration::from_millis(500));return;}
        let recovered_without_result=result.is_err() && !task.path.with_file_name("result.json").exists() && task.path.with_file_name("resolved-request.json").exists();
        if recovered_without_result{
            result=(||->anyhow::Result<Response>{let bound=bridge::resolved_request(&task.path)?;let lib=self.library.library.as_ref().ok_or_else(||anyhow::anyhow!("照片库未恢复"))?;let mut response=bridge::reconstruct(&bound,&lib.catalog)?;response.exports=Some(bridge::export_batches(&task.path)?);Ok(response)})();
            if let Ok(response)=&mut result{match bridge::latest_browse(&task.path,&task.request){Ok(browse)=>response.browse=browse,Err(e)=>result=Err(e)}}
            if let Some(browse)=result.as_ref().ok().and_then(|r|r.browse.as_ref()){
                if let Err(e)=self.library.resume(&browse.library,&browse.selected){self.message=format!("最后浏览库暂未恢复：{e:#}");self.advanced=Some(task);return;}
            }
        }
        match result {
            Ok(response)=>match response.merge_existing(&task.request,&self.project).and_then(|next| {
                let queue=if task.request.purpose==Purpose::Develop{response.import_queue(&next)?}else{Vec::new()};
                Ok((next,queue))
            }) {
                Ok((next,queue))=>{
                    let switched=response.browse.is_some();
                    let mut retained=HashMap::new();
                    for (before,after) in self.project.photos.iter().zip(&next.photos){if before.edit==after.edit && before.cached_denoise==after.cached_denoise{if let Some(texture)=self.thumbnails.get(&before.thumbnail_key()){retained.insert(after.thumbnail_key(),texture.clone());}}}
                    for (n,(before,after)) in self.project.photos.iter().zip(&next.photos).enumerate(){if before.edit!=after.edit{self.undo.entry(n).or_default().push(before.edit.clone());self.redo.remove(&n);}}
                    self.dirty|=self.project.digest().ok()!=next.digest().ok();self.project=next;
                    self.pending_checkpoint=Some((task.request.clone(),task.path.clone()));
                    let paths=queue.iter().map(|r|r.path.clone()).collect::<Vec<_>>();self.import_bindings=queue;
                    self.bridge_return=if task.request.purpose==Purpose::Develop{Some(response)}else{None};self.texture=None;self.thumbnails=retained;self.before=false;
                    self.frame=None;self.brush=Brush::None;self.grouping=None;self.heal_source=None;
                    if let Some(photo)=self.project.photos.get(self.active){let c=photo.edit.develop.crop.geometry;self.crop=[c.rect.x0,c.rect.y0,c.rect.x1,c.rect.y1,c.angle];}
                    if task.request.purpose!=Purpose::Develop || switched {self.mode=Mode::Library;self.library.message="完整版导入/管理已返回，照片库已刷新；工程保留，可继续选片编辑。".into();}
                    else if !paths.is_empty(){self.mode=Mode::Develop;self.submit(Job::Import(paths),"载入完整版选片和显影参数…");}
                    else if !self.project.photos.is_empty(){self.mode=Mode::Develop;self.render();}else{self.mode=Mode::Library;}
                    self.message="已返回主界面，照片库与高级显影参数已更新；人像修饰保留。".into();
                    if recovered_without_result{self.library.message="完整版异常退出，已从已保存的照片库恢复显影；人像修饰保留。".into();}
                    if !self.busy{self.finish_checkpoint();}
                },Err(e)=>self.message=format!("工程未部分覆盖：{e:#}；高级数据保留在 {}",task.path.display()),
            },Err(e)=>self.message=format!("完整版未返回有效交接：{e:#}；工程恢复文件保留在 {}",task.path.display()),
        }
        self.refresh_recoveries();
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }
}
