use super::*;
use luminacraft::bridge;
impl StudioApp {
    pub(super) fn refresh_recoveries(&mut self) {
        let directory=self.library.library.as_ref().map(|l|l.directory.clone()).or_else(||luminacraft::library::default_directory().ok());
        if let Some(directory)=directory {
            match bridge::pending(&directory){Ok(paths)=>self.recovery_paths=paths.into_iter().filter(|p|self.active_checkpoint.as_ref()!=Some(p)).collect(),Err(e)=>self.message=format!("无法读取恢复记录：{e:#}")}
        }
    }
    pub(super) fn reconnect_library(&mut self) {
        let result=(||->anyhow::Result<()>{let dir=luminacraft::library::default_directory()?;anyhow::ensure!(!bridge::live_handoff(&dir)?,"仍有窗口参与交接，请等它退出后重试");self.library.resume(&dir,&[])?;Ok(())})();
        if let Err(e)=result{self.message=e.to_string();self.library.message=self.message.clone();}else{self.refresh_recoveries();}
    }
    pub(super) fn finish_checkpoint(&mut self) {
        let Some((request,path))=&self.pending_checkpoint else{return};
        match bridge::checkpoint(path,request,&self.project,self.directory.clone()) {
            Ok(())=>{self.active_checkpoint=Some(path.clone());self.pending_checkpoint=None;self.refresh_recoveries();},
            Err(e)=>self.message=format!("工程在窗口中保留，但恢复检查点未完成：{e:#}"),
        }
    }
    pub(super) fn mark_checkpoint_saved(&mut self) {
        if let Some(path)=&self.active_checkpoint{
            let marker=path.with_file_name("project-saved.json");
            if !marker.exists(){if let Err(e)=bridge::write_new(&marker,&serde_json::json!({"project":self.project.id,"digest":self.project.saved_digest})){self.message=format!("工程已保存，恢复确认未完成：{e}");}}
        }
    }
    pub(super) fn dismiss_checkpoint(&mut self) {
        if let Some(path)=&self.active_checkpoint{
            let marker=path.with_file_name("dismissed.json");
            if !marker.exists(){if let Err(e)=bridge::write_new(&marker,&serde_json::json!({"project":self.project.id,"discarded":true})){self.message=format!("未能记录放弃操作：{e}");}}
        }
    }
    pub(super) fn recover_edit(&mut self,path:PathBuf)->anyhow::Result<()> {
        anyhow::ensure!(!self.busy && self.advanced.is_none() && !self.dirty,"请先保存当前工程，再恢复其他编辑");
        let lease=bridge::HandoffLease::recover(&path)?;
        let expected=bridge::resolved_request(&path)?;
        if self.library.library.as_ref().is_some_and(|l|!bridge::same_path(&l.directory,&expected.library)){self.library.resume(&expected.library,&[])?;}
        let lib=self.library.library.as_ref().ok_or_else(||anyhow::anyhow!("请先重新连接照片库"))?;
        let (request,response,project,directory)=bridge::recover_project(&path,&lib.directory,&lib.catalog)?;
        let queue=if request.purpose==bridge::Purpose::Develop{response.import_queue(&project)?}else{Vec::new()};
        if let Some(browse)=&response.browse{self.library.resume(&browse.library,&browse.selected)?;}
        self.project=project;self.directory=directory;self.active=0;
        self.dirty=self.project.saved_digest.as_ref()!=Some(&self.project.digest()?);
        self.undo.clear();self.redo.clear();self.thumbnails.clear();self.texture=None;self.frame=None;self.brush=Brush::None;self.grouping=None;self.before=false;
        let paths=queue.iter().map(|r|r.path.clone()).collect::<Vec<_>>();self.import_bindings=queue;
        self.pending_checkpoint=Some((request.clone(),path));self.bridge_return=if paths.is_empty(){None}else{Some(response)};
        if !paths.is_empty(){self.mode=Mode::Develop;self.submit(Job::Import(paths),"恢复完整版选片…");}
        else {self.finish_checkpoint();if self.project.photos.is_empty(){self.mode=Mode::Library;}else{self.mode=Mode::Develop;self.render();}}
        drop(lease);self.recovery_dialog=false;self.refresh_recoveries();Ok(())
    }
    pub(super) fn recovery_ui(&mut self,ctx:&egui::Context) {
        if !self.recovery_dialog{return;}
        let mut selected=None;
        egui::Window::new("恢复交接编辑").collapsible(false).show(ctx,|ui|{
            ui.label("恢复上次交接的工程与人像修饰；异常退出时从照片库找回显影。");
            if self.dirty{ui.label("当前工程尚未保存，请先保存再恢复其他工程。");}
            for path in &self.recovery_paths {
                match bridge::read::<bridge::Request>(path){Ok(request)=>{if ui.add_enabled(!self.dirty&&!self.busy&&self.library.library.is_some(),egui::Button::new(format!("{} · {} 张",request.project.title,request.project.photos.len()))).clicked(){selected=Some(path.clone());}},Err(_)=>{ui.label("一条恢复记录无法读取，原文件已保留。");}}
            }
            if ui.button("稍后恢复").clicked(){self.recovery_dialog=false;}
        });
        if let Some(path)=selected{if let Err(e)=self.recover_edit(path){self.message=format!("恢复未覆盖当前工程：{e:#}");}}
    }
}

