//! Local output selection is independent from the editable source photograph.
use super::*;
impl StudioApp{
    pub(super) fn export_tools(&mut self,ui:&mut egui::Ui){
        ui.add_space(4.0);ui.label(RichText::new("导出工作台").font(crate::typography::semibold(14.0)));ui.add_space(16.0);
        self.photo_view_tools(ui);
        if self.export_view.is_some(){
            ui.add_space(16.0);
            let r=ui.add_enabled(!self.blocking,egui::Button::new("查看当前照片").min_size(Vec2::new(ui.available_width(),34.0))).on_hover_text("离开历史成片预览，查看当前照片的修图结果");
            lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.export.current",r.rect);
            if r.clicked(){self.export_view=None;self.export_texture=None;self.before=false;self.zoom=1.0;self.render();}
        }
    }
    pub(super) fn start_local_export(&mut self,path:PathBuf)->anyhow::Result<()>{
        anyhow::ensure!(!self.busy&&!self.ai.export_busy()&&self.advanced.is_none(),"请等待当前任务完成");
        anyhow::ensure!(self.project.publishes.len()<1000,"导出历史已满，请先移除旧记录");
        let ext=path.extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase();
        anyhow::ensure!(if self.export_tiff{matches!(ext.as_str(),"tif"|"tiff")}else{matches!(ext.as_str(),"jpg"|"jpeg")},"文件扩展名与所选导出格式不符");
        let photo=self.project.photos.get(self.active).ok_or_else(||anyhow::anyhow!("请先选择照片"))?.clone();
        self.export_view=None;self.export_texture=None;
        self.submit(Job::Export(photo,path,self.export_tiff,self.keep_gps),"生成完整分辨率成片…");Ok(())
    }
    pub(super) fn add_local_export(&mut self,record:PublishRecord){
        self.record_local_export(record,true);
    }
    pub(super) fn record_local_export(&mut self,record:PublishRecord,select:bool){
        if !self.project.publishes.iter().any(|r|r.output==record.output&&r.output_sha256==record.output_sha256){
            if self.project.publishes.len()<1000{self.project.publishes.push(record.clone());self.dirty=true;}
            else{self.message="成片已保存；导出历史已满，请移除旧记录后保存工程。".into();}
        }
        if select{self.last_export=Some(record.clone());if self.mode==Mode::Export{self.select_local_export(record);}}
    }
    pub(super) fn select_local_export(&mut self,record:PublishRecord){
        if self.blocking{return;}
        self.export_texture=None;self.export_view=Some(record.clone());self.last_export=Some(record.clone());self.zoom=1.0;
        self.submit(Job::LocalPreview(record),"载入本地成片…");
    }
}
