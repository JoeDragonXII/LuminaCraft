use super::*;
use luminacraft::catalog_io::{self,Export};
#[derive(Default)]
pub(super) struct CatalogUi { dialog:Option<Dialog>, lightroom:Option<LightroomDialog>, pub last_export:Option<PathBuf>, root:Option<PathBuf>, source:Option<PathBuf>, revision:u64,pending:Option<Instant> }
struct Dialog { parent:PathBuf,name:String,copy:bool,selected:bool,new:bool,single:bool }
struct LightroomDialog {template:PathBuf,path:PathBuf,selected:bool}
impl StudioApp {
    pub(super) fn open_catalog(&mut self,path:PathBuf){
        if self.busy||self.library.scanning||self.ai.task.is_some()||self.ai.export_busy(){self.message="请等当前任务完成后切换目录。".into();return;}
        if self.library.library.as_ref().is_some_and(|lib|if luminacraft::catalog_file::is_file(&path){luminacraft::catalog_file::backing(&lib.directory).ok().flatten().is_some_and(|b|luminacraft::bridge::same_path(&b.source,&path))}else{catalog_io::resolve(&path).is_ok_and(|p|luminacraft::bridge::same_path(&p,&lib.directory))}){self.mode=Mode::Library;self.message="该目录已打开。".into();return;}
        if self.dirty && rfd::MessageDialog::new().set_title("打开照片库目录").set_description("当前编辑尚未保存。放弃这些修改并打开其他目录？").set_buttons(rfd::MessageButtons::YesNo).show()!=rfd::MessageDialogResult::Yes{return;}
        if !self.flush_catalog_file(){return;}
        self.submit(Job::CatalogOpen(path),"打开照片库目录…");
    }
    pub(super) fn catalog_menu(&mut self,ui:&mut egui::Ui){
        let response=ui.menu_button("目录",|ui|{
            let enabled=!self.busy&&!self.library.scanning&&self.ai.task.is_none()&&!self.ai.export_busy();
            ui.add_enabled_ui(enabled,|ui|{
                if ui.button("打开目录文件…").clicked(){ui.close();if let Some(path)=rfd::FileDialog::new().add_filter("LuminaCraft 目录",&["lumcat","lccatalog"]).pick_file(){self.open_catalog(path);}}
                let save=ui.add_enabled(self.catalog_ui.source.is_some(),egui::Button::new("保存目录文件"));lightcraft_ui_egui::widgets::register(ui.ctx(),"main.catalog.save",save.rect);
                if save.clicked(){ui.close();if !self.dirty||self.project.photos.is_empty(){self.flush_catalog_file();}else{self.save();}}
                if ui.button("打开 LightCraft 目录文件夹…").clicked(){ui.close();if let Some(path)=rfd::FileDialog::new().pick_folder(){self.open_catalog(path);}}
                for (title,new,selected) in [("新建目录…",true,false),("目录另存为 / 导出全部…",false,false),("将选中照片导出为目录…",false,true)] {
                    let available=new||self.library.library.is_some()&&(!selected||!self.library.selected_ids().is_empty());
                    if ui.add_enabled(available,egui::Button::new(title)).clicked(){
                        ui.close();if let Some(parent)=rfd::FileDialog::new().set_title("选择目录文件的保存位置").pick_folder(){self.catalog_ui.dialog=Some(Dialog{parent,name:if new{"新照片库"}else{"照片库副本"}.into(),copy:false,selected,new,single:true});}
                    }
                }
                if ui.add_enabled(self.library.library.is_some(),egui::Button::new("导出 Lightroom 目录 .lrcat（实验）…")).clicked(){
                    ui.close();if let Some(template)=rfd::FileDialog::new().set_title("选择旧 Lightroom 目录作为结构模板（只读）").add_filter("Lightroom 目录",&["lrcat"]).pick_file(){
                        if let Some(path)=rfd::FileDialog::new().set_title("保存新的实验目录，不覆盖已有文件").add_filter("Lightroom 目录",&["lrcat"]).set_file_name("照片库.lrcat").save_file(){self.catalog_ui.lightroom=Some(LightroomDialog{template,path,selected:false});}
                    }
                }
                if let Some(path)=self.catalog_ui.last_export.clone(){if ui.button("打开上次导出的目录").clicked(){ui.close();self.open_catalog(path);}}
            });
            if let Some(lib)=&self.library.library{ui.separator();ui.small(self.catalog_ui.source.as_ref().unwrap_or(&lib.directory).display().to_string());}
        });
        lightcraft_ui_egui::widgets::register(ui.ctx(),"main.catalog.menu",response.response.rect);
    }
    pub(super) fn catalog_dialog(&mut self,ctx:&egui::Context){
        self.lightroom_dialog(ctx);
        let Some(dialog)=&mut self.catalog_ui.dialog else{return;};
        let mut apply=false;let mut cancel=false;
        egui::Window::new(if dialog.new{"新建照片库目录"}else{"导出照片库目录"}).collapsible(false).resizable(false).show(ctx,|ui|{
            ui.label(dialog.parent.display().to_string());ui.label("目录名称");ui.text_edit_singleline(&mut dialog.name);
            ui.checkbox(&mut dialog.single,"单文件目录 .lumcat");
            if !dialog.new {ui.checkbox(&mut dialog.copy,"复制原片及原 XMP");ui.small(if dialog.copy{"复制原片；原文件保留。"}else{"引用原片当前位置，迁移后可重新定位。"});
                if dialog.selected{ui.small("导出选中照片；虚拟副本会同时包含其原始记录。");}
            }
            ui.small(if dialog.single{"只需携带一个 .lumcat 文件，工作目录由程序自动管理。"}else{"保存为独立文件夹，使用其中的 catalog.lccatalog 打开。"});
            ui.horizontal(|ui|{let response=ui.button(if dialog.new{"创建并打开"}else{"导出目录"});lightcraft_ui_egui::widgets::register(ui.ctx(),"main.catalog.confirm",response.rect);apply=response.clicked();cancel=ui.button("取消").clicked();});
        });
        if cancel{self.catalog_ui.dialog=None;return;}
        if apply {
            let outcome=(||->anyhow::Result<(Export,PathBuf,bool)>{
                let name=dialog.name.trim();
                anyhow::ensure!(!name.is_empty() && name.len()<=180 && name.chars().all(|c|!c.is_control()&&!"<>:\"/\\|?*".contains(c)) && !name.ends_with(['.',' ']) && name!="." && name!="..","请使用有效的单层目录名称");
                let lib=self.library.library.as_ref();
                let export=Export{catalog:if dialog.new{Default::default()}else{lib.ok_or_else(||anyhow::anyhow!("照片库未打开"))?.catalog.clone()},library:lib.map(|l|l.directory.clone()).unwrap_or_else(||dialog.parent.clone()),project:if dialog.new{None}else{Some(self.project.clone())},selected:dialog.selected.then(||self.library.selected_ids().into_iter().map(lightcraft_catalog::PhotoId).collect()),copy_originals:dialog.copy};
                let name=if dialog.single&&!name.to_lowercase().ends_with(".lumcat"){format!("{name}.lumcat")}else{name.to_string()};
                Ok((export,dialog.parent.join(name),dialog.new))
            })();
            match outcome {Ok((export,path,new))=>{if new&&self.dirty&&rfd::MessageDialog::new().set_title("新建目录").set_description("当前编辑尚未保存。放弃修改并创建新目录？").set_buttons(rfd::MessageButtons::YesNo).show()!=rfd::MessageDialogResult::Yes{return;}
                self.catalog_ui.dialog=None;self.submit(Job::CatalogExport(export,path,new),"正在保存独立目录…");},Err(e)=>self.message=e.to_string()}
        }
    }
    pub(super) fn catalog_diagnostic(&mut self,params:&serde_json::Value)->anyhow::Result<()> {
        anyhow::ensure!(!self.busy&&!self.library.scanning&&self.ai.task.is_none()&&!self.ai.export_busy(),"当前任务尚未结束");
        let path=PathBuf::from(params.get("path").and_then(|v|v.as_str()).ok_or_else(||anyhow::anyhow!("path required"))?);
        if params.get("action").and_then(|v|v.as_str())==Some("lightroom") {self.catalog_ui.lightroom=Some(LightroomDialog{template:PathBuf::from(params.get("template").and_then(|v|v.as_str()).ok_or_else(||anyhow::anyhow!("template required"))?),path,selected:params.get("selected").and_then(|v|v.as_bool()).unwrap_or(false)});}
        else if params.get("action").and_then(|v|v.as_str())==Some("open"){self.open_catalog(path);}else{
            self.catalog_ui.dialog=Some(Dialog{parent:path.parent().ok_or_else(||anyhow::anyhow!("parent required"))?.into(),name:path.file_name().ok_or_else(||anyhow::anyhow!("name required"))?.to_string_lossy().into_owned(),copy:params.get("copy").and_then(|v|v.as_bool()).unwrap_or(false),selected:params.get("selected").and_then(|v|v.as_bool()).unwrap_or(false),new:params.get("new").and_then(|v|v.as_bool()).unwrap_or(false),single:luminacraft::catalog_file::is_file(&path)});
        }Ok(())
    }
    fn lightroom_dialog(&mut self,ctx:&egui::Context){
        let Some(dialog)=&mut self.catalog_ui.lightroom else{return};let mut apply=false;let mut cancel=false;
        egui::Window::new("导出 .lrcat（实验）").collapsible(false).resizable(false).show(ctx,|ui|{
            ui.label(format!("保存到：{}",dialog.path.display()));ui.small(format!("只读模板：{}",dialog.template.display()));
            ui.label("包含照片路径、评级、旗标、颜色标签、关键词、集合和虚拟副本。");
            ui.label("不包含显影、蒙版、修图、历史和 AI 结果；完整编辑请另存 .lumcat。");
            ui.small("原片保持原位置；智能集合保存当前成员。支持旧目录版本 1400000。");
            ui.small("导出后自动用 LightCraft 回读核对；Lightroom Classic 实际打开尚待验证。");
            ui.checkbox(&mut dialog.selected,"仅导出选中照片（自动包含原始记录）");
            ui.horizontal(|ui|{let response=ui.add_enabled(!self.busy,egui::Button::new("导出并回读验证"));lightcraft_ui_egui::widgets::register(ui.ctx(),"main.catalog.lightroom.confirm",response.rect);apply=response.clicked();cancel=ui.button("取消").clicked();});
        });
        if cancel{self.catalog_ui.lightroom=None;return;}
        if apply{if let Some(lib)=&self.library.library{let export=luminacraft::lightroom_export::Export{catalog:lib.catalog.clone(),selected:dialog.selected.then(||self.library.selected_ids().into_iter().map(lightcraft_catalog::PhotoId).collect()),template:dialog.template.clone()};let path=dialog.path.clone();self.catalog_ui.lightroom=None;self.submit(Job::LightroomExport(export,path),"正在生成 .lrcat 并用 LightCraft 回读核对…");}else{self.message="请先打开照片库".into();}}
    }
    pub(super) fn flush_catalog_file(&mut self)->bool {
        let Some(lib)=&mut self.library.library else{return true};
        match lib.save_file(){Ok(_)=>true,Err(e)=>{self.message=format!("目录文件未保存，工作区数据保留：{e:#}");self.library.message=self.message.clone();self.allow_close=false;false}}
    }
    pub(super) fn poll_catalog_file(&mut self,ctx:&egui::Context){
        let Some(lib)=&self.library.library else{return};
        if self.catalog_ui.root.as_ref()!=Some(&lib.directory){self.catalog_ui.root=Some(lib.directory.clone());self.catalog_ui.source=luminacraft::catalog_file::backing(&lib.directory).ok().flatten().map(|b|b.source);self.catalog_ui.revision=lib.catalog.revision;self.catalog_ui.pending=self.catalog_ui.source.as_ref().map(|_|Instant::now());}
        if self.catalog_ui.source.is_none(){return;}
        if self.catalog_ui.revision!=lib.catalog.revision{self.catalog_ui.revision=lib.catalog.revision;self.catalog_ui.pending=Some(Instant::now());}
        if let Some(pending)=self.catalog_ui.pending {ctx.request_repaint_after(std::time::Duration::from_millis(200));if pending.elapsed().as_secs()>=2&&!self.busy&&!self.library.scanning&&self.ai.task.is_none()&&!self.ai.export_busy(){self.catalog_ui.pending=None;self.flush_catalog_file();}}
    }
    pub(super) fn catalog_opened(&mut self,lib:luminacraft::library::Library,project:Option<Project>,warning:Option<String>){
        let dir=lib.directory.clone();self.library.replace_library(lib,&[]);
        self.catalog_ui.root=None;
        let writable=project.is_some()||!dir.join("project.json").exists();self.project=project.unwrap_or_default();self.directory=writable.then_some(dir);
        self.active=0;self.before=false;self.zoom=1.0;self.crop=self.project.photos.first().map(|p|{let c=p.edit.develop.crop.geometry;[c.rect.x0,c.rect.y0,c.rect.x1,c.rect.y1,c.angle]}).unwrap_or([0.0,0.0,1.0,1.0,0.0]);self.mode=Mode::Library;self.dirty=self.project.saved_digest.as_ref().is_some_and(|saved|self.project.digest().is_ok_and(|h|&h!=saved));self.ai.reset_view();self.undo.clear();self.redo.clear();self.texture=None;self.thumbnails.clear();self.frame=None;self.last_export=None;self.export_view=None;self.export_texture=None;self.pending_preview=None;self.brush=Brush::None;self.grouping=None;
        self.pending_checkpoint=None;self.active_checkpoint=None;self.import_bindings.clear();self.bridge_return=None;self.import_errors.clear();
        self.message=warning.unwrap_or_else(||"目录已打开，主页面和完整版使用同一照片库。".into());self.library.message=self.message.clone();
    }
}

