use eframe::egui::{self,Color32,RichText,Vec2};
use luminacraft::{engine,project::{self,Command,EditState,Heal,MaskDab,Project,PublishRecord},skills::{self,Skill},worker::{self,Job,Outcome,Request,Response}};
use std::{collections::HashMap,path::PathBuf,sync::{Arc,mpsc,atomic::{AtomicBool,Ordering}},time::Instant};
use lightcraft_geom::Point;

const ACCENT:Color32=crate::chrome::ACCENT;
#[path="app_bridge.rs"] mod bridge_ui;
use bridge_ui::AdvancedTask;
#[path="app_recovery.rs"] mod recovery_ui;
#[path="main_control.rs"] mod main_control;
#[path="app_ai.rs"] mod ai_ui;
#[path="app_catalog.rs"] mod catalog_ui;
#[path="native_motion.rs"] mod native_motion;
#[path="app_exports.rs"] mod exports_ui;
#[path="app_history.rs"] mod history_ui;
#[path="app_status.rs"] mod status_ui;
#[derive(PartialEq,Clone,Copy,Debug)] enum Mode {Library,Develop,Portrait,Style,Export}
impl Mode {
    fn is_editor(self)->bool{matches!(self,Self::Develop|Self::Portrait|Self::Style)}
    fn editor_title(self)->&'static str{match self{Self::Develop=>"修图－基础",Self::Portrait=>"修图－人像",Self::Style=>"修图－风格",Self::Export=>"本地成片",_=>"修图"}}
}
#[derive(PartialEq,Clone,Copy)] enum Brush {None,Add,Erase,Source,Heal}
pub struct StudioApp {
    brand_texture:egui::TextureHandle,
    catalog_ui:catalog_ui::CatalogUi,
    ai:ai_ui::AiUi,
    advanced:Option<AdvancedTask>, bridge_return:Option<luminacraft::bridge::Response>,
    import_bindings:Vec<luminacraft::bridge::ReturnedPhoto>,
    pending_edit_focus:Option<(PathBuf,u64)>,
    last_library_selection:Option<(PathBuf,Vec<u64>,Option<u64>)>,
    recovery_paths:Vec<PathBuf>, recovery_dialog:bool, pending_checkpoint:Option<(luminacraft::bridge::Request,PathBuf)>, active_checkpoint:Option<PathBuf>,
    main_control:Option<main_control::MainControl>,
    library:crate::library_ui::LibraryUi,
    import_errors:Vec<String>,
    project:Project, directory:Option<PathBuf>, active:usize, mode:Mode, editor_mode:Mode, brush:Brush,
    commit_gate:Arc<luminacraft::task_gate::CommitGate>,
    export_view:Option<PublishRecord>,export_texture:Option<(PathBuf,String,egui::TextureHandle)>,
    tx:mpsc::Sender<Request>, rx:mpsc::Receiver<Response>, id:u64, cancel:Arc<AtomicBool>, busy:bool, blocking:bool, message:String,
    texture:Option<egui::TextureHandle>, thumbnails:HashMap<String,egui::TextureHandle>, frame:Option<lightcraft_pipeline::geometry::Frame>, image_size:(usize,usize),
    before:bool, zoom:f32, radius:f64, heal_source:Option<Point>, dirty:bool, close_prompt:bool, allow_close:bool,
    undo:HashMap<usize,Vec<EditState>>, redo:HashMap<usize,Vec<EditState>>, grouping:Option<String>, pending_preview:Option<Instant>,
    skills:Vec<Skill>, selected_skill:usize, confirm_skill:bool,
    export_tiff:bool, keep_gps:bool, last_export:Option<PublishRecord>,
    crop:[f64;5], pending_open:Option<PathBuf>, elapsed_ms:u128, screenshot_requested:bool, started:Instant,
}
impl StudioApp {
    pub fn new(cc:&eframe::CreationContext<'_>,input:Option<PathBuf>)->Self {
        crate::keyboard::configure(&cc.egui_ctx);
        crate::typography::install(&cc.egui_ctx);
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let (tx,rx)=worker::start(cc.egui_ctx.clone());
        let catalog_input=input.as_ref().is_some_and(|p|luminacraft::catalog_file::is_file(p)||p.file_name().is_some_and(|n|n==luminacraft::catalog_io::MARKER)||p.join("catalog.snap").is_file()||p.join("catalog.log").is_file());
        let mut app=Self {brand_texture:crate::branding::texture(&cc.egui_ctx),last_library_selection:None,pending_edit_focus:None,catalog_ui:Default::default(),ai:ai_ui::AiUi::new(),import_bindings:vec![],recovery_paths:vec![],recovery_dialog:false,pending_checkpoint:None,active_checkpoint:None,main_control:main_control::MainControl::new(&cc.egui_ctx),advanced:None,bridge_return:None,import_errors:vec![],library:if catalog_input{crate::library_ui::LibraryUi::unopened(&cc.egui_ctx)}else{crate::library_ui::LibraryUi::new(&cc.egui_ctx)},project:Project::default(),directory:None,active:0,mode:Mode::Library,editor_mode:Mode::Develop,brush:Brush::None,commit_gate:Arc::new(Default::default()),export_view:None,export_texture:None,tx,rx,id:0,cancel:Arc::new(AtomicBool::new(false)),busy:false,blocking:false,message:"导入原片，开始一段完整的摄影流程。".into(),texture:None,thumbnails:HashMap::new(),frame:None,image_size:(0,0),before:false,zoom:1.0,radius:0.012,heal_source:None,dirty:false,close_prompt:false,allow_close:false,undo:HashMap::new(),redo:HashMap::new(),grouping:None,pending_preview:None,skills:skills::builtins(),selected_skill:0,confirm_skill:false,export_tiff:false,keep_gps:false,last_export:None,crop:[0.0,0.0,1.0,1.0,0.0],pending_open:None,elapsed_ms:0,screenshot_requested:false,started:Instant::now()};
        if let Some(input)=input {if luminacraft::catalog_file::is_file(&input)||input.file_name().is_some_and(|s|s==luminacraft::catalog_io::MARKER)||input.join("catalog.snap").is_file()||input.join("catalog.log").is_file(){app.submit(Job::CatalogOpen(input),"打开照片库目录…");}else if input.is_dir(){if input.join("project.json").is_file(){app.pending_open=Some(input.clone());app.mode=Mode::Develop;app.submit(Job::Open(input),"读取项目…");}else{app.library.scan(vec![input],&cc.egui_ctx);}}else{app.mode=Mode::Develop;app.submit(Job::Import(vec![input]),"读取原片…");}}
        app.refresh_recoveries();
        app
    }
    fn submit(&mut self,job:Job,message:&str){if self.blocking{self.message="请等待当前任务完成。".into();return;}self.cancel.store(true,Ordering::Relaxed);self.id+=1;self.cancel=Arc::new(AtomicBool::new(false));self.busy=true;self.blocking=!matches!(job,Job::Preview(..)|Job::LocalPreview(..));self.message=message.into();if matches!(job,Job::Preview(..)|Job::Import(..)|Job::Open(..)|Job::CatalogOpen(..)){self.pending_preview=None;}self.commit_gate=Arc::new(Default::default());if self.tx.send(Request{id:self.id,job:Job::Managed(Box::new(job),self.commit_gate.clone()),cancel:self.cancel.clone()}).is_err(){self.busy=false;self.blocking=false;self.message="后台处理线程已停止，请先保存项目。".into();}}
    fn render(&mut self){if self.blocking{self.pending_preview=Some(Instant::now());return;}if let Some(photo)=self.project.photos.get(self.active){self.submit(Job::Preview(photo.clone(),self.before,if self.zoom>1.2{3200}else{1200}),"渲染预览…");}}
    fn remember(&mut self){if let Some(p)=self.project.photos.get(self.active){let history=self.undo.entry(self.active).or_default();history.push(p.edit.clone());if history.len()>100{history.remove(0);}self.redo.entry(self.active).or_default().clear();}}
    fn edit_library_photos(&mut self,paths:Vec<PathBuf>){
        self.edit_library_photos_to(paths,self.editor_mode);
    }
    fn edit_library_photos_to(&mut self,paths:Vec<PathBuf>,destination:Mode){
        if self.busy || paths.is_empty(){return;}
        let records=self.library.editing_records();let library=self.library.library.as_ref().map(|l|l.directory.clone());
        self.last_library_selection=self.library.selection_identity();
        let focus=self.library.edit_focus_id();
        let mut imports=Vec::new();let mut existing=None;let mut bindings=Vec::new();
        for (path,record) in paths.into_iter().zip(records){
            let matched=self.project.photos.iter().position(|p|p.catalog_ref.as_ref().is_some_and(|c|Some(&c.library).zip(library.as_ref()).is_some_and(|(a,b)|luminacraft::bridge::same_path(a,b)) && c.id==record.id)
                || (p.catalog_ref.is_none() && record.copy_name.is_none() && luminacraft::bridge::same_path(&p.source.path,&path)));
            if let Some(n)=matched{if existing.is_none()||Some(record.id)==focus{existing=Some(n);}if self.project.photos[n].catalog_ref.is_none(){if let Some(lib)=&library{self.project.photos[n].catalog_ref=Some(project::CatalogRef{library:lib.clone(),id:record.id,copy_name:record.copy_name.clone()});self.dirty=true;}}}
            else{imports.push(path);bindings.push(record);}
        }
        let was_before=self.before;
        self.mode=destination;self.before=false;self.brush=Brush::None;self.heal_source=None;self.grouping=None;
        self.export_view=None;self.export_texture=None;
        if !imports.is_empty(){
            self.pending_edit_focus=library.zip(focus);self.import_bindings=bindings;
            self.texture=None;self.zoom=1.0;self.ai.reset_view();
            self.submit(Job::Import(imports),"从照片库载入照片…");
        }else if let Some(n)=existing{
            let changed=n!=self.active;self.active=n;
            let c=self.project.photos[n].edit.develop.crop.geometry;self.crop=[c.rect.x0,c.rect.y0,c.rect.x1,c.rect.y1,c.angle];
            if changed{self.texture=None;self.zoom=1.0;self.ai.reset_view();}
            if changed||was_before||self.texture.is_none(){self.render();}
        }
    }
    fn command(&mut self,cmd:Command,group:Option<&str>){
        let Some(current)=self.project.photos.get(self.active)else{return;};
        if let Command::Set{control,value}=&cmd{if lightcraft_develop::controls::get(&current.edit.develop,control).is_some_and(|old|(old-value).abs()<=1e-9){return;}}
        let mut candidate=current.clone();
        if let Err(e)=project::execute(&mut candidate,&cmd){self.message=e.to_string();return;}
        if candidate.edit==current.edit{return;}
        if group.is_none() || self.grouping.as_deref()!=group {self.remember();}
        self.project.photos[self.active].edit=candidate.edit;self.dirty=true;self.before=false;self.grouping=group.map(str::to_owned);self.pending_preview=Some(Instant::now());
    }
    fn undo(&mut self,redo:bool){
        let cursor=self.undo.get(&self.active).map_or(0,Vec::len);
        if redo{self.jump_history(cursor+1);}else if cursor>0{self.jump_history(cursor-1);}
    }
    fn save(&mut self){
        if self.project.photos.is_empty()&&self.directory.as_ref().is_some_and(|d|luminacraft::catalog_file::backing(d).ok().flatten().is_some()){self.flush_catalog_file();return;}
        if let (Some(dir),Some(lib))=(&self.directory,&mut self.library.library){if dir.join(luminacraft::catalog_io::MARKER).is_file()&&luminacraft::bridge::same_path(dir,&lib.directory){if let Err(e)=luminacraft::catalog_io::save_project_develop(&self.project,lib){self.message=format!("目录保存失败，当前编辑保留：{e:#}");return;}}}
        let dir=self.directory.clone().or_else(||rfd::FileDialog::new().set_title("选择项目保存目录").pick_folder());
        if let Some(dir)=dir{self.submit(Job::Save(self.project.clone(),dir),"保存项目…");}
    }
    fn parameter_row(ui:&mut egui::Ui,label:&str,value:&mut f64,range:std::ops::RangeInclusive<f64>,step:f64,decimals:usize)->(egui::Response,bool){
        ui.label(RichText::new(label).size(12.0).color(ui.visuals().weak_text_color()));
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(),30.0),egui::Layout::left_to_right(egui::Align::Center),|ui|{
            ui.spacing_mut().slider_width=(ui.available_width()-78.0).max(60.0);
            let original=*value;
            let slider=ui.add(egui::Slider::new(value,range.clone()).step_by(step).show_value(false));
            let interactive=slider.dragged()||slider.drag_stopped()||slider.clicked()||(slider.has_focus()&&ui.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Key{pressed:true,..}))));
            let number=ui.add_sized(Vec2::new(68.0,30.0),egui::DragValue::new(value).range(range).speed(step).min_decimals(decimals).max_decimals(decimals));
            let changed=(slider.changed()&&interactive)||number.changed();
            if !changed{*value=original;}
            (slider.union(number),changed)
        }).inner
    }
    fn slider(&mut self,ui:&mut egui::Ui,label:&str,id:&str,range:std::ops::RangeInclusive<f64>){
        let Some(photo)=self.project.photos.get(self.active)else{return;};
        let mut value=lightcraft_develop::controls::get(&photo.edit.develop,id).unwrap_or_default();
        let (response,changed)=Self::parameter_row(ui,label,&mut value,range,if id=="light.exposure"{0.05}else{1.0},if id=="light.exposure"{2}else{0});
        lightcraft_ui_egui::widgets::register(ui.ctx(),format!("luminacraft.editor.control.{id}"),response.rect);
        if changed{self.command(Command::Set{control:id.into(),value},(response.dragged()||response.drag_stopped()).then_some(id));}
        if response.drag_stopped(){self.grouping=None;}
    }
    fn receive(&mut self,ctx:&egui::Context){
        while let Ok(response)=self.rx.try_recv(){if response.id!=self.id{continue;}self.busy=false;self.blocking=false;
            match response.result {
                Err(e)=>{self.pending_edit_focus=None;self.message=e;self.library.message=self.message.clone();self.allow_close=false;},
                Ok(Outcome::Imported(photos,errors))=>{
                    self.import_errors=errors;
                    let mut count=0;for mut photo in photos {
                        if let Some(n)=self.import_bindings.iter().position(|r|luminacraft::bridge::same_path(&r.path,&photo.source.path)) {
                            let returned=self.import_bindings.remove(n);
                            let library=self.bridge_return.as_ref().map(|r|r.library.clone()).or_else(||self.library.library.as_ref().map(|l|l.directory.clone()));
                            if let Some(library)=library{if let Err(e)=luminacraft::bridge::bind_imported(&mut photo,&returned,&library){self.import_errors.push(e.to_string());continue;}}
                            if returned.copy_name.is_some(){if let Some(master)=self.project.photos.iter().find(|p|p.source.sha256==photo.source.sha256 && p.catalog_ref.as_ref().is_none_or(|c|c.copy_name.is_none())){photo.edit.retouch=master.edit.retouch.clone();}}
                        }
                        else if let Some(response)=&self.bridge_return {response.apply_imported(&mut photo);}
                        else if let Some(lib)=&self.library.library {
                            if let Some(p)=lib.catalog.photos().find(|p|p.copy_of.is_none() && luminacraft::library::photo_path(p).is_some_and(|p|luminacraft::bridge::same_path(std::path::Path::new(p),&photo.source.path))){photo.edit.develop=(*p.develop).clone();photo.baseline=(*p.develop).clone();photo.catalog_ref=Some(project::CatalogRef{library:lib.directory.clone(),id:p.id.0,copy_name:p.copy_name.clone()});}
                        }
                        if photo.cached_denoise.is_none(){if let Some(reference)=&photo.catalog_ref{
                            match luminacraft::resources::latest(&reference.library,reference.id,&photo.source.sha256){Ok(cached)=>photo.cached_denoise=cached,Err(e)=>{self.import_errors.push(e.to_string());continue;}}
                        }}
                        if !self.project.photos.iter().any(|p|p.same_identity(&photo)){self.project.photos.push(photo);count+=1;}
                    }
                    self.bridge_return=None;
                    self.import_bindings.clear();
                    self.finish_checkpoint();
                    self.dirty=count>0||self.dirty;
                    self.active=if let Some((library,id))=self.pending_edit_focus.take(){self.project.photos.iter().position(|p|p.catalog_ref.as_ref().is_some_and(|c|c.id==id&&luminacraft::bridge::same_path(&c.library,&library))).unwrap_or_else(||self.project.photos.len().saturating_sub(1))}else{self.project.photos.len().saturating_sub(1)};
                    self.message=format!("已导入 {count} 张；{} 项错误，重复原片已跳过。",self.import_errors.len());
                    if let Some(photo)=self.project.photos.get(self.active){let c=photo.edit.develop.crop.geometry;self.crop=[c.rect.x0,c.rect.y0,c.rect.x1,c.rect.y1,c.angle];}
                    self.render();
                }
                Ok(Outcome::CatalogExported(report,open))=>{self.catalog_ui.last_export=Some(report.directory.clone());self.message=format!("已保存目录：{} · {} 张照片 · {} 个资源文件",report.directory.display(),report.photos,report.files);self.library.message=self.message.clone();if open{self.submit(Job::CatalogOpen(report.directory),"打开新目录…");}},
                Ok(Outcome::CatalogOpened(lib,project,warning))=>self.catalog_opened(lib,project,warning),
                Ok(Outcome::LightroomExported(report))=>{self.message=format!("已导出 {} · {} 条照片记录 · LightCraft 回读通过。{}",report.output.display(),report.photos,report.warnings.join(" "));self.library.message=self.message.clone();},
                Ok(Outcome::Opened(project))=>{self.project=project;self.last_export=None;self.export_view=None;self.export_texture=None;self.before=false;self.zoom=1.0;self.frame=None;self.crop=self.project.photos.first().map(|p|{let c=p.edit.develop.crop.geometry;[c.rect.x0,c.rect.y0,c.rect.x1,c.rect.y1,c.angle]}).unwrap_or([0.0,0.0,1.0,1.0,0.0]);self.directory=self.pending_open.take();self.active=0;self.dirty=self.project.saved_digest.as_ref().is_some_and(|saved|self.project.digest().is_ok_and(|h|&h!=saved));self.ai.reset_view();self.undo.clear();self.redo.clear();self.thumbnails.clear();self.texture=None;self.render();}
                Ok(Outcome::Saved(dir,digest))=>{self.directory=Some(dir);self.dirty=self.project.digest().is_ok_and(|h|h!=digest);self.project.saved_digest=Some(digest);if !self.dirty{self.mark_checkpoint_saved();}if !self.flush_catalog_file(){self.allow_close=false;self.close_prompt=true;continue;}self.message="项目已保存；原片和原 XMP 保持原样。".into();if self.allow_close{self.allow_close=!self.dirty;if self.allow_close{ctx.send_viewport_cmd(egui::ViewportCommand::Close);}else{self.close_prompt=true;}}}
                Ok(Outcome::Preview(out))=>{
                    self.elapsed_ms=out.elapsed_ms;self.image_size=(out.image.width,out.image.height);self.frame=Some(out.frame);
                    let rgba:Vec<u8>=out.image.data.iter().flatten().copied().collect();
                    let color=egui::ColorImage::from_rgba_unmultiplied([out.image.width,out.image.height],&rgba);
                    self.texture=Some(ctx.load_texture("photo",color,egui::TextureOptions::LINEAR));
                    if let Some(p)=self.project.photos.get(self.active){
                        let raw=image::RgbaImage::from_raw(out.image.width as u32,out.image.height as u32,rgba);
                        if let Some(raw)=raw {if self.thumbnails.len()>=256{if let Some(key)=self.thumbnails.keys().next().cloned(){self.thumbnails.remove(&key);}}let thumb=image::imageops::thumbnail(&raw,144,96);self.thumbnails.insert(p.thumbnail_key(),ctx.load_texture(p.thumbnail_key(),egui::ColorImage::from_rgba_unmultiplied([thumb.width() as usize,thumb.height() as usize],thumb.as_raw()),egui::TextureOptions::LINEAR));}
                    }
                    self.message=if self.before{"调整前".into()}else{"预览已更新".into()};
                    self.finish_checkpoint();
                }
                Ok(Outcome::Faces(faces))=>{let n=faces.len();let changed=self.project.photos.get(self.active).is_some_and(|p|p.edit.retouch.faces!=faces);if changed{self.remember();self.project.photos[self.active].edit.retouch.faces=faces;self.dirty=true;self.render();}self.message=if n==0{"没有可靠识别到人脸，可使用蒙版画笔指定皮肤区域。".into()}else{format!("已识别 {n} 个人像区域；可用画笔增减。")};}
                Ok(Outcome::Xmp(photo))=>{if self.project.photos.get(self.active).is_some_and(|p|p.edit!=photo.edit){self.remember();}if let Some(p)=self.project.photos.get_mut(self.active){*p=photo;}self.dirty=true;self.render();}
                Ok(Outcome::Exported(record,report))=>{self.message=format!("已导出 {} × {} · {} 位",report["width"],report["height"],report["bits"]);self.add_local_export(record);}
                Ok(Outcome::LocalPreview(path,hash,image))=>{
                    if self.export_view.as_ref().is_some_and(|r|r.output==path&&r.output_sha256==hash){
                        let color=egui::ColorImage::from_rgba_unmultiplied([image.width()as usize,image.height()as usize],image.as_raw());
                        self.export_texture=Some((path,hash,ctx.load_texture("local-export",color,egui::TextureOptions::LINEAR)));
                        self.message="本地成片预览已就绪。".into();
                    }
                },
                Ok(Outcome::LocalCopied(record))=>{self.add_local_export(record);},
                Ok(Outcome::Connected(_)|Outcome::Published(_))=>{self.message="当前版本仅支持本地导出。".into();},
            }
        }
        if !self.blocking && !(self.mode==Mode::Export&&self.export_view.is_some()) && self.pending_preview.is_some_and(|t|t.elapsed().as_millis()>100){self.render();}
    }
    fn inspector(&mut self,ui:&mut egui::Ui){
        if self.mode==Mode::Export{self.export_ui(ui);return;}
        if self.mode==Mode::Style{self.ai_panel(ui);return;}
        ui.add_space(12.0);
        if !self.import_errors.is_empty(){egui::CollapsingHeader::new(format!("{} 项载入错误",self.import_errors.len())).default_open(true).show(ui,|ui|{for error in &self.import_errors{ui.colored_label(Color32::LIGHT_RED,error);}});}
        if self.project.photos.is_empty(){ui.heading("从原片开始");ui.label("ARW · JPEG · TIFF\n参数和修饰单独保存，随时回退。");return;}
        match self.mode {
            Mode::Develop=>{
                ui.heading("基础修图");ui.add_space(10.0);
                if ui.add_enabled(!self.busy,egui::Button::new("导入 Camera Raw XMP")).clicked(){if let Some(path)=rfd::FileDialog::new().add_filter("XMP",&["xmp"]).pick_file(){let p=self.project.photos[self.active].clone();self.submit(Job::Xmp(p,path),"读取 XMP 参数…");}}
                let flow=egui::CollapsingHeader::new("处理流程").id_salt("editor-flows").show(ui,|ui|self.flow_tools(ui));
                lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.editor.flows",flow.header_response.rect);
                egui::CollapsingHeader::new("白平衡").default_open(true).show(ui,|ui|{self.slider(ui,"色温","wb.temp",1000.0..=50000.0);self.slider(ui,"色调","wb.tint",-150.0..=150.0);});
                egui::CollapsingHeader::new("光线").default_open(true).show(ui,|ui|{
                    for (label,id) in [("曝光","light.exposure"),("对比度","light.contrast"),("高光","light.highlights"),("阴影","light.shadows"),("白色","light.whites"),("黑色","light.blacks")]{self.slider(ui,label,id,if id=="light.exposure"{-5.0..=5.0}else{-100.0..=100.0});}
                });
                egui::CollapsingHeader::new("色彩").show(ui,|ui|{self.slider(ui,"自然饱和度","color.vibrance",-100.0..=100.0);self.slider(ui,"饱和度","color.saturation",-100.0..=100.0);});
                egui::CollapsingHeader::new("曲线").show(ui,|ui|{
                    let curve=self.project.photos[self.active].edit.develop.curve.master.clone();let mut y=if curve.len()==3{curve[1].y}else{0.5};
                    let (rect,_)=ui.allocate_exact_size(Vec2::new(240.0,120.0),egui::Sense::hover());ui.painter().rect_filled(rect,8,Color32::from_rgb(14,18,23));
                    let points=if curve.len()>=2{curve}else{vec![Point::new(0.0,0.0),Point::new(1.0,1.0)]};
                    for pair in points.windows(2){ui.painter().line_segment([egui::pos2(rect.left()+pair[0].x as f32*rect.width(),rect.bottom()-pair[0].y as f32*rect.height()),egui::pos2(rect.left()+pair[1].x as f32*rect.width(),rect.bottom()-pair[1].y as f32*rect.height())],egui::Stroke::new(1.5,ACCENT));}
                    let response=ui.add(egui::Slider::new(&mut y,0.05..=0.95).text("中间调"));
                    if response.changed(){let group=(response.dragged()||response.drag_stopped()).then_some("curve.master");if group.is_none()||self.grouping.as_deref()!=group{self.remember();}self.project.photos[self.active].edit.develop.curve.master=vec![Point::new(0.0,0.0),Point::new(0.5,y),Point::new(1.0,1.0)];self.grouping=group.map(str::to_owned);self.dirty=true;self.pending_preview=Some(Instant::now());}
                    if response.drag_stopped(){self.grouping=None;}
                });
                egui::CollapsingHeader::new("裁剪与旋转").show(ui,|ui|{
                    ui.horizontal(|ui|{if ui.button("左转").clicked(){self.command(Command::Rotate{clockwise:false},None);}if ui.button("右转").clicked(){self.command(Command::Rotate{clockwise:true},None);}});
                    for (n,label) in ["左","上","右","下"].iter().enumerate(){ui.add(egui::Slider::new(&mut self.crop[n],0.0..=1.0).text(*label));}ui.add(egui::Slider::new(&mut self.crop[4],-45.0..=45.0).text("校正角度"));
                    if ui.button("应用裁剪").clicked(){self.command(Command::Crop{left:self.crop[0],top:self.crop[1],right:self.crop[2],bottom:self.crop[3],angle:self.crop[4]},None);}
                });
                egui::CollapsingHeader::new("XMP 导入报告").show(ui,|ui|{let r=&self.project.photos[self.active].report;ui.label(format!("已应用 {} · 近似映射 {} · 未支持 {}",r.applied.len(),r.approximate.len(),r.unsupported.len()));for (name,items) in [("已应用",&r.applied),("近似映射",&r.approximate),("未支持",&r.unsupported)]{if !items.is_empty(){ui.label(RichText::new(name).color(ACCENT));ui.label(items.join(" · "));}}ui.small("调色引擎不同，XMP 参数不保证与 Adobe 画质相同。");});
            }
            Mode::Portrait=>{
                ui.heading("人像修饰");ui.small("识别初始区域，再用画笔微调。");
                if ui.add_enabled(!self.busy,egui::Button::new("识别人像区域")).clicked(){self.submit(Job::Faces(self.project.photos[self.active].clone()),"识别人像区域…");}
                let r=&self.project.photos[self.active].edit.retouch;let mut smooth=r.smooth;let mut tone=r.tone;let faces=r.faces.len();ui.label(format!("{faces} 个区域"));
                let(a,ac)=Self::parameter_row(ui,"磨皮",&mut smooth,0.0..=1.0,0.01,2);let(b,bc)=Self::parameter_row(ui,"肤色统一",&mut tone,0.0..=1.0,0.01,2);
                if ac||bc{let group=if a.dragged()||a.drag_stopped(){Some("portrait.smooth")}else if b.dragged()||b.drag_stopped(){Some("portrait.tone")}else{None};self.command(Command::Portrait{smooth,tone},group);}if a.drag_stopped()||b.drag_stopped(){self.grouping=None;}
                ui.add_space(16.0);ui.label("蒙版画笔");ui.horizontal_wrapped(|ui|{for (brush,id,name) in [(Brush::None,"view","查看"),(Brush::Add,"add","添加皮肤"),(Brush::Erase,"erase","排除区域")]{let r=ui.selectable_value(&mut self.brush,brush,name);lightcraft_ui_egui::widgets::register(ui.ctx(),format!("luminacraft.editor.brush.{id}"),r.rect);}});
                Self::parameter_row(ui,"画笔大小",&mut self.radius,0.003..=0.04,0.0001,4);
                ui.add_space(16.0);ui.label("局部修复");ui.horizontal_wrapped(|ui|{ui.selectable_value(&mut self.brush,Brush::Source,"选取来源");ui.selectable_value(&mut self.brush,Brush::Heal,"修复画笔");});
                ui.small("先点击一块干净皮肤作为来源，再点击需要修复的位置。未设置来源时使用邻近区域。");
                ui.add_space(16.0);if ui.button("继续 AI 风格化 →").clicked(){self.select_editor(Mode::Style);}
            }
            Mode::Export=>self.export_ui(ui),
            Mode::Style=>{},
            Mode::Library=>{},
        }
        ui.add_space(20.0);if !self.mode.is_editor(){ui.separator();}
        egui::CollapsingHeader::new("工程版本").show(ui,|ui|{
            if ui.button("保留当前版本").clicked(){let label=format!("版本 {}",self.project.versions.len()+1);self.project.version(self.active,&label);self.dirty=true;}
            let versions:Vec<_>=self.project.versions.iter().enumerate().filter(|(_,v)|v.photo==self.active).map(|(n,v)|(n,v.label.clone())).collect();
            for (n,label) in versions {if ui.button(format!("恢复 {label}")).clicked(){let state=self.project.versions[n].edit.clone();if state!=self.project.photos[self.active].edit{self.remember();self.project.photos[self.active].edit=state;self.dirty=true;self.before=false;self.render();}}}
        });
    }
    fn flow_tools(&mut self,ui:&mut egui::Ui){
                ui.small("先看步骤，再执行；结果可以撤销。");
                for (n,skill) in self.skills.iter().enumerate(){if ui.selectable_label(self.selected_skill==n,&skill.manifest.title).clicked(){self.selected_skill=n;self.confirm_skill=false;}}
                if ui.button("导入 Skill 文件夹").clicked(){if let Some(path)=rfd::FileDialog::new().pick_folder(){match Skill::load(&path){Ok(skill)=>{self.selected_skill=self.skills.len();self.skills.push(skill);},Err(e)=>self.message=e.to_string()}}}
                ui.add_space(16.0);if let Some(skill)=self.skills.get(self.selected_skill){ui.label(&skill.instructions);for step in skill.preview(){ui.small(step);}let export=skill.manifest.export;let skill=skill.clone();
                let r=ui.checkbox(&mut self.confirm_skill,"确认应用这些步骤");lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.editor.flow-confirm",r.rect);
                    let r=ui.add_enabled(self.confirm_skill&&!self.busy,egui::Button::new("执行流程"));lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.editor.flow-apply",r.rect);if r.clicked(){let mut candidate=self.project.photos[self.active].clone();match skill.apply(&mut candidate){Ok(())=>{if candidate.edit!=self.project.photos[self.active].edit{self.remember();self.project.photos[self.active]=candidate;self.dirty=true;}self.confirm_skill=false;if export{self.mode=Mode::Export;self.export_tiff=false;}else if skill.manifest.id=="natural-portrait"&&self.project.photos[self.active].edit.retouch.faces.is_empty()&&self.project.photos[self.active].edit.retouch.mask.is_empty(){self.submit(Job::Faces(self.project.photos[self.active].clone()),"识别人像区域…");}else{self.render();}},Err(e)=>self.message=e.to_string()}}
                }
    }
    fn export_ui(&mut self,ui:&mut egui::Ui){
        ui.heading("导出当前照片");if let Some(photo)=self.project.photos.get(self.active){ui.small(photo.source.path.file_name().unwrap_or_default().to_string_lossy());}ui.horizontal(|ui|{ui.selectable_value(&mut self.export_tiff,false,"JPEG");ui.selectable_value(&mut self.export_tiff,true,"16 位 TIFF");});ui.small("完整分辨率 · sRGB · 不覆盖已有文件");ui.checkbox(&mut self.keep_gps,"保留拍摄位置 GPS");
        if ui.add_enabled(!self.busy&&!self.project.photos.is_empty(),egui::Button::new("选择目录并导出")).clicked(){if let Some(dir)=rfd::FileDialog::new().pick_folder(){let stem=self.project.photos[self.active].source.path.file_stem().and_then(|s|s.to_str()).unwrap_or("photo");let path=engine::unique_output(&dir,stem,self.export_tiff);if let Err(error)=self.start_local_export(path){self.message=error.to_string();}}}
        if !self.project.advanced_outputs.is_empty(){
            ui.add_space(16.0);ui.label("高级编辑成片");ui.small("这些成片由完整版导出，不含主界面的人像修饰。");
            let mut selected=None;
            for (n,record) in self.project.advanced_outputs.iter().enumerate(){if ui.selectable_label(self.last_export.as_ref().is_some_and(|r|r.output==record.output),record.output.file_name().and_then(|n|n.to_str()).unwrap_or("成片")).clicked(){selected=Some(n);}}
            if let Some(n)=selected{self.select_local_export(self.project.advanced_outputs[n].clone());}
        }
        if !self.project.style_outputs.is_empty(){
            ui.add_space(16.0);ui.label("AI 风格成片");
            let mut selection=None;
            for record in &self.project.style_outputs{if ui.button(format!("{} · 查看 / 导出",record.template_name)).clicked(){selection=Some(record.id.clone());}}
            if let Some(id)=selection{
                if let Some(record)=self.project.style_outputs.iter().find(|r|r.id==id){if let Some(index)=self.project.photos.iter().position(|p|record.matches(p)){self.active=index;self.mode=Mode::Style;self.ai.selected=Some(id);self.ai.compare=false;self.load_style_preview(ui.ctx());}}
            }
        }
        if let Some(record)=&self.last_export {ui.small(record.output.file_name().and_then(|s|s.to_str()).unwrap_or("成片"));}
        ui.add_space(16.0);ui.label("本地导出历史");
        let records=self.project.publishes.iter().cloned().collect::<Vec<_>>();
        for (index,record) in records.into_iter().enumerate().rev(){
            let title=record.output.file_name().unwrap_or_default().to_string_lossy().to_string();
            let selected=self.export_view.as_ref().is_some_and(|r|r.output==record.output&&r.output_sha256==record.output_sha256);
            let response=ui.add_enabled(!self.blocking,egui::Button::selectable(selected,title).truncate()).on_hover_text(record.output.display().to_string());
            lightcraft_ui_egui::widgets::register(ui.ctx(),format!("luminacraft.export.history.{index}"),response.rect);
            if response.clicked(){self.select_local_export(record);}
        }
        if self.project.publishes.is_empty(){ui.small("导出后保存在这里，随工程保存。");}
        if let Some(record)=self.export_view.clone(){
            ui.add_space(12.0);ui.small(record.output.display().to_string());
            let copy=ui.add_enabled(!self.busy&&!self.ai.export_busy()&&self.project.publishes.len()<1000,egui::Button::new("另存副本…"));lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.export.copy",copy.rect);
            if copy.clicked(){if let Some(path)=rfd::FileDialog::new().set_file_name(record.output.file_name().unwrap_or_default().to_string_lossy()).save_file(){self.submit(Job::LocalCopy(record.clone(),path),"保存成片副本…");}}
            if ui.add_enabled(!self.busy,egui::Button::new("移除记录")).on_hover_text("仅移除工程中的导出记录，磁盘文件保留").clicked(){self.project.publishes.retain(|r|r.output!=record.output||r.output_sha256!=record.output_sha256);self.export_view=None;self.export_texture=None;self.last_export=None;self.dirty=true;}
            if !record.output.is_file(){ui.small("文件已离线或移动，记录仍保留。");}
        }

    }
}
impl eframe::App for StudioApp {
    fn raw_input_hook(&mut self,_:&egui::Context,input:&mut egui::RawInput){if let Some(c)=&mut self.main_control{c.inject(input);}}
    fn logic(&mut self,ctx:&egui::Context,_:&mut eframe::Frame){
        self.poll_main_control(ctx);
        self.poll_advanced(ctx);
        if self.advanced.is_some(){
            if ctx.input(|i|i.viewport().close_requested()){ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);self.message="请先在完整版完成操作并返回主界面。".into();}
            return;
        }
        self.receive(ctx);
        self.poll_ai(ctx);
        self.library.poll(ctx);
        self.poll_catalog_file(ctx);
        if self.library.take_path_changes(){if let Some(lib)=&self.library.library{
            match luminacraft::library::follow_catalog_paths(&mut self.project,lib){
                Ok(n) if n>0=>{self.dirty=true;self.message=format!("已更新 {n} 张工程原片关联；编辑与版本保留。");},
                Ok(_)=>{},Err(e)=>{self.message=format!("库关联已保存；工程关联未改写：{e}");self.library.message=self.message.clone();}
            }
        }}
        if self.mode!=Mode::Library&&!ctx.text_edit_focused()&&ctx.input(|i|!i.modifiers.command&&i.key_pressed(egui::Key::G)){self.select_page(Mode::Library);self.library.open_grid();}
        if ctx.input(|i|i.viewport().close_requested()) && !self.allow_close && (self.dirty||self.busy||self.ai.task.is_some()||self.ai.export_busy()){ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);self.close_prompt=true;}
        if ctx.input(|i|i.viewport().close_requested())&&!self.busy&&(self.allow_close||!self.dirty){if !self.flush_catalog_file(){ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);}}
        ctx.input(|i|{if i.modifiers.command&&i.key_pressed(egui::Key::S)&&!self.busy {self.save();}if self.mode.is_editor()&&i.modifiers.command&&i.key_pressed(egui::Key::Z)&&!self.blocking{self.undo(i.modifiers.shift);}});
        if self.busy || self.pending_preview.is_some(){ctx.request_repaint_after(std::time::Duration::from_millis(30));}
        if let Ok(path)=std::env::var("LUMINACRAFT_SCREENSHOT") {
            if !self.screenshot_requested {ctx.request_repaint_after(std::time::Duration::from_millis(100));}
            if !self.screenshot_requested && (if self.mode==Mode::Library{self.library.screenshot_ready()}else{self.texture.is_some()}) && !self.busy && self.started.elapsed().as_secs()>2 {self.screenshot_requested=true;ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));}
            let screenshot=ctx.input(|i|i.events.iter().find_map(|e|if let egui::Event::Screenshot{image,..}=e{Some(image.clone())}else{None}));
            if let Some(image)=screenshot {let pixels:Vec<u8>=image.pixels.iter().flat_map(|p|p.to_array()).collect();let _=image::save_buffer(&path,&pixels,image.width() as u32,image.height() as u32,image::ColorType::Rgba8);self.allow_close=true;ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
        }
    }
    fn ui(&mut self,ui:&mut egui::Ui,_:&mut eframe::Frame){
        let ctx=ui.ctx().clone();
        ctx.set_theme(egui::Theme::Dark);
        if self.ai.motion_checked.elapsed().as_secs()>=2{self.ai.motion_enabled=native_motion::enabled();self.ai.motion_checked=Instant::now();}
        crate::chrome::apply(ui,self.ai.motion_enabled);
        if self.advanced.is_some(){
            egui::CentralPanel::default().show(ui,|ui|{ui.vertical_centered(|ui|{ui.add_space(70.0);ui.heading("完整版正在处理");ui.label("完成导入或高级编辑后，点击完整版顶部的“返回 LuminaCraft 主界面”。");if self.ai.motion_enabled{ui.spinner();}});});return;
        }
        if self.mode.is_editor(){self.editor_mode=self.mode;}
        self.recovery_ui(&ctx);
        self.ai_settings_ui(&ctx);
        self.catalog_dialog(&ctx);
        egui::Panel::top("header").show_separator_line(false).show(ui,|ui|self.editor_header(ui));
        let status=if self.mode==Mode::Library{self.library.message.as_str()}else if self.blocking{self.message.as_str()}else{""};
        let metadata=self.project.photos.get(self.active).map(|p|format!("{} × {} · {}",p.source.width,p.source.height,if p.source.sensor{"RAW 显影"}else{"照片"})).unwrap_or_default();
        if status_ui::show(ui,status_ui::Status{message:status,metadata:&metadata,elapsed_ms:self.elapsed_ms,blocking:self.busy&&self.blocking,motion:self.ai.motion_enabled,committing:self.commit_gate.committing(),cancelled:self.commit_gate.cancelled()})&&self.commit_gate.request_cancel(){self.cancel.store(true,Ordering::Relaxed);self.message="正在取消，等待后台确认…".into();}
        if self.mode==Mode::Library {
            if let Some(paths)=self.library.show(ui,self.busy){self.edit_library_photos(paths);}
        }else{
            self.style_workbench(ui);
        }
        if self.close_prompt {egui::Window::new("修改尚未保存").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER,Vec2::ZERO).show(&ctx,|ui|{ui.label("保存项目后，参数、蒙版和导出记录可以继续编辑。");if self.ai.task.is_some(){ui.label("AI 正在处理。立即关闭会停止接收结果；服务端可能继续处理。");}if self.busy||self.ai.export_busy(){ui.small(if self.ai.export_busy(){"正在导出风格成片，完成后可关闭。"}else if self.commit_gate.committing(){"正在写入文件，完成后可关闭。"}else{"请等任务结束，或返回工作台取消计算。"});}ui.horizontal(|ui|{if ui.add_enabled(!self.busy&&!self.ai.export_busy(),egui::Button::new("保存并关闭")).clicked(){self.close_prompt=false;self.save();self.allow_close=self.busy;}if ui.add_enabled(!self.busy&&!self.ai.export_busy(),egui::Button::new("放弃并关闭")).clicked(){self.dismiss_checkpoint();self.cancel.store(true,Ordering::Relaxed);self.allow_close=true;ctx.send_viewport_cmd(egui::ViewportCommand::Close);}if ui.button("继续编辑").clicked(){self.close_prompt=false;}});});}
        self.record_main_widgets(&ctx);
    }
}






