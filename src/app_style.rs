//! Shared native photo editor. Layout state is ephemeral; source files are unchanged.
use super::*;
use lightcraft_ui_egui::icons::{self,Icon};

use crate::chrome::*;
use crate::chrome::ACCENT;

fn navigation_row(ui:&mut egui::Ui,id:&str,label:&str,icon:Icon,selected:bool,enabled:bool)->egui::Response{
    let (r,fg)=control(ui,id,label,selected,enabled,Vec2::new(ui.available_width(),40.0));
    let rect=r.rect;
    icons::paint(ui.painter(),egui::Rect::from_center_size(egui::pos2(rect.left()+22.0,rect.center().y),Vec2::splat(18.0)),icon,fg);
    ui.painter().text(egui::pos2(rect.left()+44.0,rect.center().y),egui::Align2::LEFT_CENTER,label,crate::typography::body(),fg);
    r.on_hover_text(label)
}

fn registered(ui:&mut egui::Ui,id:&str,text:&str,enabled:bool)->egui::Response {
    let mut button=egui::Button::new(text).min_size(Vec2::new(ui.available_width(),34.0));
    if id=="luminacraft.ai.generate" && enabled{button=egui::Button::new(RichText::new(text).color(ON_ACCENT)).fill(ACCENT).min_size(Vec2::new(ui.available_width(),38.0));}
    let response=ui.add_enabled(enabled,button);
    lightcraft_ui_egui::widgets::register(ui.ctx(),id,response.rect);
    response
}

impl StudioApp {
    pub(in crate::app) fn select_page(&mut self,mode:Mode){
        if self.blocking||self.mode==mode{return;}
        if self.mode==Mode::Library&&(mode.is_editor()||mode==Mode::Export){
            let selection=self.library.selection_identity();
            let changed=selection!=self.last_library_selection;
            self.last_library_selection=selection;
            if changed&&mode==Mode::Export&&self.library.edit_focus_id().is_some(){self.export_view=None;self.export_texture=None;}
            if let Some(id)=self.library.edit_focus_id().filter(|_|changed){
                let current=self.project.photos.get(self.active).and_then(|p|p.catalog_ref.as_ref());
                let matches=current.is_some_and(|c|c.id==id&&self.library.library.as_ref().is_some_and(|lib|luminacraft::bridge::same_path(&c.library,&lib.directory)));
                if !matches{let paths=self.library.selected_paths();if !paths.is_empty(){self.edit_library_photos_to(paths,mode);return;}}
            }
        }
        if mode==Mode::Library&&self.last_library_selection.is_none(){self.last_library_selection=self.library.selection_identity();}
        if self.mode.is_editor(){self.editor_mode=self.mode;}
        let was_before=self.before;
        self.mode=mode;
        if mode.is_editor(){self.editor_mode=mode;}
        self.brush=Brush::None;self.heal_source=None;self.grouping=None;
        // Carry the before flag through the library so returning knows to refresh it.
        if mode!=Mode::Library{self.before=false;}
        if mode==Mode::Export&&self.export_texture.is_none(){
            if let Some(record)=self.export_view.clone(){
                if was_before{self.pending_preview=Some(Instant::now());}
                self.select_local_export(record);return;
            }
        }
        // Existing preview and zoom survive navigation. Only a before-preview needs rendering.
        if (mode.is_editor()||mode==Mode::Export)&&(was_before||self.texture.is_none())&&!self.project.photos.is_empty(){self.render();}
    }

    pub(in crate::app) fn select_editor(&mut self,mode:Mode){if mode.is_editor(){self.select_page(mode);}}

    pub(crate) fn editor_header(&mut self,ui:&mut egui::Ui){
        ui.add_space(6.0);
        centered_row(ui,34.0,|ui|{
            ui.add(egui::Image::new(&self.brand_texture).fit_to_exact_size(Vec2::splat(30.0))).on_hover_text("LuminaCraft");
            ui.label(RichText::new("LuminaCraft").font(crate::typography::semibold(19.0)).color(INK));
            ui.add_space(8.0);
            for (mode,id,title) in [(Mode::Library,"luminacraft.editor.library","照片库"),(self.editor_mode,"luminacraft.editor.edit","修图"),(Mode::Export,"luminacraft.editor.export","导出")]{
                let selected=if mode.is_editor(){self.mode.is_editor()}else{self.mode==mode};
                if tab(ui,id,title,selected,!self.blocking).clicked(){self.select_page(mode);}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                let more=text_button(ui,"luminacraft.editor.more",if self.dirty{"更多 ·"}else{"更多"},72.0,false,true);
                egui::Popup::menu(&more).show(|ui|{
                    ui.weak("工程与目录");
                    let save=ui.add_enabled(!self.busy&&!self.project.photos.is_empty(),egui::Button::new(if self.dirty{"保存工程 *"}else{"保存工程"})).on_hover_text("保存修图参数与导出历史；不会导出图片 · Ctrl+S");
                    lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.editor.save",save.rect);
                    if save.clicked(){ui.close();self.save();}
                    if ui.add_enabled(!self.busy&&self.ai.task.is_none()&&!self.ai.export_busy(),egui::Button::new("新建工程…")).clicked(){
                        ui.close();if !self.dirty||rfd::MessageDialog::new().set_title("新建工程").set_description("当前工程修改尚未保存。放弃这些修改并新建？照片库记录会保留。").set_buttons(rfd::MessageButtons::YesNo).show()==rfd::MessageDialogResult::Yes{
                            self.project=Project::default();self.ai.reset_view();self.directory=None;self.active=0;self.dirty=false;self.undo.clear();self.redo.clear();self.texture=None;self.thumbnails.clear();self.last_export=None;self.export_view=None;self.export_texture=None;self.frame=None;self.import_errors.clear();self.pending_preview=None;self.brush=Brush::None;self.before=false;self.heal_source=None;self.grouping=None;self.mode=Mode::Library;self.editor_mode=Mode::Develop;
                        }
                    }
                    if ui.add_enabled(!self.busy&&self.ai.task.is_none()&&!self.ai.export_busy(),egui::Button::new("打开工程…")).clicked(){
                        ui.close();if !self.dirty||rfd::MessageDialog::new().set_title("打开工程").set_description("当前修改尚未保存。放弃这些修改？").set_buttons(rfd::MessageButtons::YesNo).show()==rfd::MessageDialogResult::Yes{if let Some(dir)=rfd::FileDialog::new().pick_folder(){self.pending_open=Some(dir.clone());self.mode=Mode::Develop;self.submit(Job::Open(dir),"读取项目…");}}
                    }
                    self.catalog_menu(ui);
                    if !self.recovery_paths.is_empty()&&ui.button(format!("恢复编辑 ({})",self.recovery_paths.len())).clicked(){ui.close();self.recovery_dialog=true;}
                    ui.separator();
                    if ui.button("AI 设置…").clicked(){ui.close();self.ai.open_settings();}
                    if ui.add_enabled(!self.busy&&!self.library.scanning&&self.ai.task.is_none()&&!self.ai.export_busy(),egui::Button::new("导入与归档…")).clicked(){ui.close();self.open_advanced(luminacraft::bridge::Purpose::Import);}
                    if ui.add_enabled(!self.busy&&!self.library.scanning&&self.ai.task.is_none()&&!self.ai.export_busy(),egui::Button::new("完整照片库 / 高级编辑…")).clicked(){ui.close();self.open_advanced(if self.mode==Mode::Library{luminacraft::bridge::Purpose::Library}else{luminacraft::bridge::Purpose::Develop});}
                    if self.library.library.is_none()&&ui.button("重新连接照片库").clicked(){ui.close();self.reconnect_library();}
                });
                more.on_hover_text(if self.dirty{"工程有未保存的修改 · Ctrl+S 保存"}else{"工程、目录与设置"});
                let library=self.mode==Mode::Library;
                let can_redo=if library{self.library.can_undo(true)}else{self.redo.get(&self.active).is_some_and(|h|!h.is_empty())};
                let can_undo=if library{self.library.can_undo(false)}else{self.undo.get(&self.active).is_some_and(|h|!h.is_empty())};
                if self.mode!=Mode::Export{
                    if tab(ui,"luminacraft.editor.redo","重做",false,!self.blocking&&can_redo).clicked(){if library{self.library.undo_catalog(true);}else{self.undo(true);}}
                    if tab(ui,"luminacraft.editor.undo","撤销",false,!self.blocking&&can_undo).clicked(){if library{self.library.undo_catalog(false);}else{self.undo(false);}}
                }
            });
        });ui.add_space(6.0);
    }

    pub(crate) fn style_workbench(&mut self,ui:&mut egui::Ui){
        self.sync_style_photo();
        if self.mode==Mode::Style && self.before{self.before=false;self.texture=None;self.render();}
        let compact=ui.available_width()<1160.0;
        if compact!=self.ai.compact{self.ai.compact=compact;self.ai.sidebar_open=!compact;}
        egui::Panel::left("style-tools").exact_size(196.0).resizable(false).show_separator_line(false)
            .frame(surface_frame(12)).show(ui,|ui|{egui::ScrollArea::vertical().id_salt("editor-tools-scroll").show(ui,|ui|self.style_tools(ui));});
        let width=if self.ai.sidebar_open{if compact{288.0}else{320.0}}else{64.0};
        egui::Panel::right("style-operations").exact_size(width).resizable(false).show_separator_line(false)
            .frame(surface_frame(if self.ai.sidebar_open{12}else{8})).show(ui,|ui|{
                if self.ai.sidebar_open {
                    centered_row(ui,34.0,|ui|{
                        ui.label(RichText::new(if self.mode==Mode::Export{"导出"}else{"调整"}).font(crate::typography::semibold(15.0)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                            let r=icon_button(ui,"luminacraft.style.sidebar",Icon::Sidebar,"收起操作栏",false,true);
                            if r.clicked(){self.ai.sidebar_open=false;}
                        });
                    });ui.add_space(12.0);
                    if matches!(self.mode,Mode::Develop|Mode::Portrait){
                        egui::Panel::bottom("edit-operation-history").default_size(210.0).size_range(135.0..=280.0).resizable(true).show_separator_line(false)
                            .frame(egui::Frame::new().inner_margin(egui::Margin{left:0,right:0,top:12,bottom:0})).show(ui,|ui|self.history_panel(ui));
                    }
                    ui.add_enabled_ui(!self.blocking,|ui|{egui::ScrollArea::vertical().id_salt(("style-operations-scroll",self.mode as u8)).show(ui,|ui|{
                        if self.mode==Mode::Style{self.ai_panel(ui);}else{self.inspector(ui);}
                    });});
                }else{
                    let label=if self.mode==Mode::Export{"展开导出栏"}else{"展开调整栏"};
                    let r=icon_button(ui,"luminacraft.style.sidebar",Icon::Sidebar,label,false,true);
                    if r.clicked(){self.ai.sidebar_open=true;}
                }
            });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(SURFACE)).show(ui,|ui|{
            if self.mode==Mode::Style{
                egui::Panel::bottom("style-presets").exact_size(168.0).resizable(false).show_separator_line(false)
                    .frame(surface_frame(10)).show(ui,|ui|self.style_cards(ui));
            }
            egui::Panel::bottom("style-progress").exact_size(44.0).resizable(false).show_separator_line(false)
                .frame(egui::Frame::new().fill(PANEL).corner_radius(CARD_RADIUS)
                    .outer_margin(egui::Margin{left:6,right:6,top:2,bottom:2}).inner_margin(4)).show(ui,|ui|{
                        if self.mode==Mode::Style{self.style_progress(ui);}else{centered_row(ui,ui.available_height(),|ui|{if self.busy && self.blocking && self.ai.motion_enabled{ui.spinner();}ui.add(egui::Label::new(&self.message).truncate());});}
                    });
            egui::CentralPanel::default().frame(egui::Frame::new().fill(CANVAS).corner_radius(SHELL_RADIUS).outer_margin(6).inner_margin(20))
                .show(ui,|ui|self.style_canvas(ui));
        });
        self.style_photo_picker(ui.ctx());
    }

    fn sync_style_photo(&mut self){
        let identity=self.project.photos.get(self.active).map(|p|(format!("{}:{}",self.project.id,p.thumbnail_key()),p.source.path.clone()));
        if self.ai.map_photo!=identity{
            self.ai.map_photo=identity;self.ai.map_location.clear();self.ai.reset_view();self.zoom=1.0;
        }
    }

    fn style_select_photo(&mut self,index:usize){
        if self.blocking || index>=self.project.photos.len() || index==self.active{return;}
        self.active=index;self.export_view=None;self.export_texture=None;self.brush=Brush::None;self.before=false;self.grouping=None;self.heal_source=None;
        self.ai.reset_view();self.texture=None;self.zoom=1.0;
        let c=self.project.photos[index].edit.develop.crop.geometry;
        self.crop=[c.rect.x0,c.rect.y0,c.rect.x1,c.rect.y1,c.angle];
        self.sync_style_photo();self.render();
    }

    fn style_tools(&mut self,ui:&mut egui::Ui){
        if self.mode==Mode::Export{self.export_tools(ui);return;}
        ui.add_space(4.0);ui.label(RichText::new("修图工作台").font(crate::typography::semibold(14.0)));ui.add_space(10.0);
        for (mode,id,label,icon) in [(Mode::Develop,"basic","修图－基础",Icon::Sliders),(Mode::Portrait,"portrait","修图－人像",Icon::Brush),(Mode::Style,"style","修图－风格",Icon::Presets)]{
            if navigation_row(ui,&format!("luminacraft.editor.{id}"),label,icon,self.mode==mode,!self.blocking).clicked(){self.select_editor(mode);}
        }
        ui.add_space(16.0);
        self.photo_view_tools(ui);
        ui.add_space(16.0);
        let has_photo=!self.project.photos.is_empty();
        let style=self.mode==Mode::Style;
        let valid=if style{self.style_record().is_some()}else{has_photo};
        let r=ui.add_enabled(valid && !self.blocking,egui::Button::selectable(if style{self.ai.compare}else{self.before},if style{"生成前 / 后"}else{"修图前 / 后"}).min_size(Vec2::new(ui.available_width(),34.0)))
            .on_hover_text(if style{"切换所选版本实际发送的输入照片与返回结果"}else{"切换原始状态与当前修图；不会改写参数"});
        lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.ai.compare",r.rect);
        if r.clicked(){if style{self.ai.compare=!self.ai.compare;self.load_style_preview(ui.ctx());}else{self.before=!self.before;self.render();}}
        ui.add_space(8.0);
        if registered(ui,"luminacraft.editor.advanced","高级编辑…",!self.busy && self.advanced.is_none()&&self.ai.task.is_none()&&!self.ai.export_busy()).on_hover_text("打开完整引擎，保存后返回当前工程").clicked(){self.open_advanced(luminacraft::bridge::Purpose::Develop);}
    }

    pub(in crate::app) fn photo_view_tools(&mut self,ui:&mut egui::Ui){
        if registered(ui,"luminacraft.style.photos","选择照片…",!self.blocking).on_hover_text("从当前工程切换照片").clicked(){self.ai.photo_picker=true;}
        centered_row(ui,34.0,|ui|{
            ui.spacing_mut().item_spacing.x=6.0;
            if icon_button(ui,"luminacraft.style.previous",Icon::ChevronLeft,"上一张",false,!self.blocking&&self.active>0).clicked(){self.style_select_photo(self.active-1);}
            ui.add_sized(Vec2::new((ui.available_width()-40.0).max(1.0),34.0),egui::Label::new(format!("{} / {}",if self.project.photos.is_empty(){0}else{self.active+1},self.project.photos.len())));
            if icon_button(ui,"luminacraft.style.next",Icon::ChevronRight,"下一张",false,!self.blocking&&self.active+1<self.project.photos.len()).clicked(){self.style_select_photo(self.active+1);}
        });
        ui.add_space(16.0);
        let has_photo=!self.project.photos.is_empty();
        centered_row(ui,34.0,|ui|{
            ui.spacing_mut().item_spacing.x=6.0;
            if icon_button(ui,"luminacraft.style.zoom-out",Icon::Minus,"缩小",false,has_photo&&self.zoom>0.5).clicked(){self.style_zoom((self.zoom/1.25).max(0.5));}
            let width=(ui.available_width()-40.0).max(1.0);
            let label=if (self.zoom-1.0).abs()<0.001{"适应".into()}else{format!("{:.0}%",self.zoom*100.0)};
            if text_button(ui,"luminacraft.style.fit",&label,width,false,has_photo).on_hover_text("适应窗口 · 完整显示照片").clicked(){self.style_zoom(1.0);}
            if icon_button(ui,"luminacraft.style.zoom-in",Icon::Plus,"放大",false,has_photo&&self.zoom<3.0).clicked(){self.style_zoom((self.zoom*1.25).min(3.0));}
        });
    }

    fn style_zoom(&mut self,value:f32){if self.mode==Mode::Export&&self.export_view.is_some(){self.zoom=value;return;}let old=self.zoom;self.zoom=value;if (self.mode!=Mode::Style||self.ai.selected.is_none()) && (old>1.2)!=(value>1.2){self.render();}}

    fn style_photo_picker(&mut self,ctx:&egui::Context){
        if !self.ai.photo_picker{return;}
        let mut open=true;let mut selected=None;
        egui::Window::new("选择照片").open(&mut open).collapsible(false).resizable(true).default_width(420.0)
            .show(ctx,|ui|{
                egui::ScrollArea::vertical().max_height(400.0).show_rows(ui,48.0,self.project.photos.len(),|ui,rows|{
                    for index in rows{
                        let photo=&self.project.photos[index];
                        ui.horizontal(|ui|{
                            if let Some(t)=self.thumbnails.get(&photo.thumbnail_key()){ui.add(egui::Image::new(t).max_size(Vec2::new(48.0,36.0)).corner_radius(CONTROL_RADIUS));}
                            let name=photo.source.path.file_name().unwrap_or_default().to_string_lossy();
                            let title=photo.catalog_ref.as_ref().and_then(|r|r.copy_name.as_ref()).map(|n|format!("{name} · {n}")).unwrap_or_else(||name.into_owned());
                            let r=ui.add_enabled(!self.blocking,egui::Button::selectable(index==self.active,title).truncate());
                            lightcraft_ui_egui::widgets::register(ui.ctx(),format!("luminacraft.photo.{index}"),r.rect);
                            if r.clicked(){selected=Some(index);}
                        });
                    }
                });
                if self.project.photos.is_empty(){ui.label("当前工程还没有照片。");}
            });
        self.ai.photo_picker=open;
        if let Some(index)=selected{self.ai.photo_picker=false;self.style_select_photo(index);}
    }

    fn style_record(&self)->Option<&ai::StyleOutput>{
        if self.mode!=Mode::Style{return None;}
        self.project.style_outputs.iter().find(|r|Some(&r.id)==self.ai.selected.as_ref() && self.project.photos.get(self.active).is_some_and(|p|r.matches(p)))
    }

    fn style_canvas(&mut self,ui:&mut egui::Ui){
        ui.horizontal(|ui|{
            let name=self.project.photos.get(self.active).and_then(|p|p.source.path.file_name()).map(|s|s.to_string_lossy().into_owned()).unwrap_or_else(||if self.mode==Mode::Export{"导出预览".into()}else{"修图工作台".into()});
            let name=if self.mode==Mode::Export{self.export_view.as_ref().and_then(|r|r.output.file_name()).map(|n|n.to_string_lossy().into_owned()).unwrap_or(name)}else{name};
            ui.add(egui::Label::new(RichText::new(name).color(MUTED).size(12.0)).truncate());
            let version=if self.mode==Mode::Export{if self.export_view.is_some(){"本地成片"}else{"待导出照片"}}else if self.mode==Mode::Style && self.style_record().is_some(){if self.ai.compare{"输入照片"}else{"风格版本"}}else{self.mode.editor_title()};
            ui.label(RichText::new(version).color(ACCENT).size(12.0));
        });ui.add_space(12.0);
        let record=if self.mode==Mode::Style{self.style_record()}else{None};
        let selected=record.is_some();
        let history=self.mode==Mode::Export&&self.export_view.is_some();
        let texture=if history{self.export_texture.as_ref().filter(|(path,hash,_)|self.export_view.as_ref().is_some_and(|r|&r.output==path&&&r.output_sha256==hash)).map(|(_,_,t)|t.clone())}else if let Some(r)=record{
            self.ai.texture.as_ref().filter(|(id,before,_)|*id==r.id && *before==self.ai.compare).map(|(_,_,t)|t.clone())
        }else{self.texture.clone()};
        if let Some(t)=texture{
            let space=ui.available_size();
            let size=fit_image(space,Vec2::new(t.size()[0]as f32,t.size()[1]as f32))*self.zoom;
            let mut pointer=None;
            egui::ScrollArea::both().id_salt("style-canvas-scroll").auto_shrink([false,false]).show(ui,|ui|{
                let extent=Vec2::new(space.x.max(size.x),space.y.max(size.y));
                let (rect,_)=ui.allocate_exact_size(extent,egui::Sense::hover());
                let image_rect=egui::Rect::from_center_size(rect.center(),size);
                let response=ui.put(image_rect,egui::Image::new(&t).fit_to_exact_size(size).corner_radius(CONTROL_RADIUS).sense(egui::Sense::click_and_drag()));
                if self.mode==Mode::Portrait && !self.before && !self.busy && self.pending_preview.is_none(){
                    if response.clicked()||response.dragged(){if let Some(pos)=response.interact_pointer_pos(){pointer=Some(Point::new(((pos.x-image_rect.left())/image_rect.width())as f64,((pos.y-image_rect.top())/image_rect.height())as f64));}}
                    if self.brush!=Brush::None && response.hovered(){if let Some(pos)=response.hover_pos(){ui.painter().circle_stroke(pos,(self.radius*image_rect.width().max(image_rect.height())as f64)as f32,egui::Stroke::new(1.0,Color32::WHITE));}}
                }
                if response.drag_stopped(){self.grouping=None;}
                lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.style.canvas",image_rect);
            });
            if let (Some(p),Some(frame))=(pointer,&self.frame){
                let p=luminacraft::retouch::output_to_source(frame,p,self.image_size.0,self.image_size.1);
                if (0.0..=1.0).contains(&p.x)&&(0.0..=1.0).contains(&p.y){match self.brush{
                    Brush::None=>{},Brush::Source=>{self.heal_source=Some(p);self.brush=Brush::Heal;self.message="修复来源已选取。".into();},
                    Brush::Add|Brush::Erase=>self.command(Command::Mask{dab:MaskDab{center:p,radius:self.radius,erase:self.brush==Brush::Erase}},Some("mask")),
                    Brush::Heal=>{let source=self.heal_source.unwrap_or(Point::new((p.x+self.radius*3.0).min(0.98),p.y));self.command(Command::Heal{heal:Heal{target:p,source,radius:self.radius}},None);}
                }}
            }
        }else{
            ui.centered_and_justified(|ui|{
                if history&&!self.busy{ui.label(RichText::new("无法预览所选成片，请检查文件是否存在或已被修改。").color(Color32::LIGHT_GRAY));}
                else if selected || self.busy{if self.ai.motion_enabled{ui.spinner();}ui.label("载入预览…");}
                else{ui.vertical_centered(|ui|{ui.label(RichText::new("从一张照片开始").size(20.0).color(Color32::from_gray(228)));ui.label(RichText::new(match self.mode{Mode::Export=>"在照片库选择照片进入修图，再到这里导出成片。",Mode::Style=>"选择工程中的照片，再选择下方风格。",_=>"在照片库选择照片，开始修图。"}).color(Color32::from_gray(178)));});}
            });
        }
    }

    fn style_progress(&mut self,ui:&mut egui::Ui){
        centered_row(ui,ui.available_height(),|ui|{
            let running=self.ai.task.is_some();
            let label=match self.ai.state{TaskState::Idle=>"准备就绪",TaskState::Working(s)=>s.label(),TaskState::Complete=>"生成完成",TaskState::Cancelled=>"已取消",TaskState::Failed=>"生成失败"};
            if running{if self.ai.motion_enabled{ui.spinner();}}else{ui.colored_label(if self.ai.state==TaskState::Failed{ERROR}else{INK},label);}
            if running{
                ui.label(label);
                if let Some((_,photo))=&self.ai.task_owner{ui.add(egui::Label::new(RichText::new(photo.source.path.file_name().unwrap_or_default().to_string_lossy()).small().color(MUTED)).truncate());}
                let cancelled=self.ai.task.as_ref().is_some_and(|t|t.cancel.load(Ordering::Relaxed));
                let r=ui.add_enabled(!cancelled,egui::Button::new(if cancelled{"取消中…"}else{"取消"}));
                lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.ai.cancel",r.rect);
                if r.clicked(){if let Some(t)=&self.ai.task{t.cancel.store(true,Ordering::Relaxed);}self.ai.status="已请求取消；服务端可能仍在处理，不会自动重试。".into();self.ai.generation_status=self.ai.status.clone();}
            }else if !self.ai.generation_status.is_empty(){ui.add(egui::Label::new(RichText::new(&self.ai.generation_status).small().color(MUTED)).truncate()).on_hover_text(&self.ai.generation_status);}
        });
    }

    fn style_cards(&mut self,ui:&mut egui::Ui){
        if self.ai.sample.is_none(){
            if let Ok(im)=image::load_from_memory(include_bytes!("../assets/style-map-reference.jpg")){
                // Keep the licensed source intact. UVs show only the public-domain photograph.
                let im=im.thumbnail(1200,750).into_rgba8();
                self.ai.sample=Some(ui.ctx().load_texture("style-reference",egui::ColorImage::from_rgba_unmultiplied([im.width()as usize,im.height()as usize],im.as_raw()),egui::TextureOptions::LINEAR));
            }
        }
        let mut styles=vec![(luminacraft::map_style::ID.to_string(),luminacraft::map_style::NAME.to_string(),String::new(),self.ai.sample.clone())];
        styles.extend(self.ai.settings.templates.iter().map(|t|(t.id.clone(),t.name.clone(),t.prompt.clone(),None)));
        egui::ScrollArea::horizontal().id_salt("style-card-scroll").show(ui,|ui|{
            ui.horizontal(|ui|{
                for (id,name,prompt,preview) in styles{
                    let chosen=self.ai.template.as_ref()==Some(&id);
                    let r=ui.allocate_ui_with_layout(Vec2::new(208.0,118.0),egui::Layout::top_down(egui::Align::Min),|ui|{
                        ui.spacing_mut().item_spacing.y=4.0;
                        let frame=egui::Frame::new().fill(if chosen{SELECTED}else{CONTROL})
                            .corner_radius(CARD_RADIUS).inner_margin(8);
                        frame.show(ui,|ui|{
                            ui.set_width(192.0);
                            if let Some(t)=preview{
                                let uv=egui::Rect::from_min_max(egui::pos2(0.121,0.235),egui::pos2(0.682,0.507));
                                ui.add(egui::Image::new(&t).uv(uv).maintain_aspect_ratio(false).fit_to_exact_size(Vec2::new(192.0,58.0)).corner_radius(CONTROL_RADIUS));
                            }else{ui.allocate_ui(Vec2::new(192.0,58.0),|ui|{ui.centered_and_justified(|ui|{ui.label(RichText::new("自定义风格").small().color(MUTED));});});}
                            ui.add(egui::Label::new(RichText::new(&name).size(crate::typography::BODY).color(INK)).truncate());
                            ui.label(RichText::new(if id==luminacraft::map_style::ID{"风格示意 · 非生成效果"}else{"已保存模板"}).size(11.0).color(MUTED));
                        }).response.rect
                    });
                    let response=ui.interact(r.inner,egui::Id::new(("style-card",&id)),egui::Sense::click()).on_hover_text(if id==luminacraft::map_style::ID{"内置规则 · 代码维护；样片为 Ansel Adams 公有领域作品，不代表生成效果"}else{&name});
                    if chosen{ui.painter().rect_stroke(response.rect,CARD_RADIUS,egui::Stroke::new(1.0,INK),egui::StrokeKind::Inside);}
                    let hover=ui.ctx().animate_bool_with_time(response.id.with("card-hover"),response.hovered(),ui.style().animation_time);
                    if hover>0.0{ui.painter().rect_stroke(response.rect,CARD_RADIUS,egui::Stroke::new(1.0,INK.gamma_multiply(hover)),egui::StrokeKind::Inside);}
                    if response.has_focus(){ui.painter().rect_stroke(response.rect,CARD_RADIUS,egui::Stroke::new(2.0,ACCENT),egui::StrokeKind::Inside);}
                    lightcraft_ui_egui::widgets::register(ui.ctx(),format!("luminacraft.style.card.{id}"),response.rect);
                    if response.clicked(){self.select_ai_template(id,prompt);}
                }
            });
        });
    }

    pub(crate) fn ai_panel(&mut self,ui:&mut egui::Ui){
        let builtin=self.ai.template.as_deref()==Some(luminacraft::map_style::ID);
        let template=self.ai.settings.templates.iter().find(|t|Some(&t.id)==self.ai.template.as_ref()).cloned();
        ui.label(RichText::new(if builtin{luminacraft::map_style::NAME}else{template.as_ref().map(|t|t.name.as_str()).unwrap_or("选择下方风格")}).size(17.0));
        if builtin{
            ui.small("内置风格 · 根据场景匹配地图语言");ui.add_space(8.0);
            ui.label("拍摄地点");
            let r=ui.add_sized(Vec2::new(ui.available_width(),36.0),egui::TextEdit::singleline(&mut self.ai.map_location).vertical_align(egui::Align::Center).hint_text("无 GPS 时补充地点"));
            lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.style.location",r.rect);
            ui.small("读取原片 GPS；仅地名按近似区域。地点会随生成请求发送。");
        }else if template.is_some(){
            ui.collapsing("本次提示词",|ui|{ui.add(egui::TextEdit::multiline(&mut self.ai.prompt).desired_rows(5).desired_width(f32::INFINITY));});
        }
        let service=template.as_ref().and_then(|t|self.ai.settings.service_for(t)).or_else(||if builtin{self.ai.settings.default_service.as_ref().and_then(|id|self.ai.settings.services.iter().find(|s|&s.id==id))}else{None}).cloned();
        if let Some(s)=&service{ui.add(egui::Label::new(RichText::new(format!("{} · {}",s.name,s.model)).size(12.0).color(MUTED)).truncate()).on_hover_text(format!("{} · {}",s.name,s.model));}else{ui.colored_label(WARNING,"尚未配置生成服务");}
        if let Some(error)=&self.ai.load_error{ui.colored_label(ERROR,error);}
        ui.add_space(12.0);
        if self.directory.is_none(){if registered(ui,"luminacraft.style.save","先保存工程…",!self.busy && !self.project.photos.is_empty()).clicked(){self.save();}}
        let can_generate=!self.busy && !self.blocking && self.advanced.is_none() && self.ai.task.is_none() && self.directory.is_some() && !self.project.photos.is_empty() && service.is_some() && self.ai.load_error.is_none() && self.ai.template.is_some();
        let r=registered(ui,"luminacraft.ai.generate","生成风格版本",can_generate).on_hover_text("向所选服务发送当前成片副本及提示词；地图风格包含 GPS 或补充地点，可能产生 API 费用");
        if r.clicked(){self.start_style(ui.ctx());}
        if self.ai.state==TaskState::Failed{ui.colored_label(ERROR,&self.ai.generation_status);}
        if self.ai.task.is_some(){ui.small("生成独立版本，可继续浏览其他照片。");}
        ui.add_space(20.0);ui.label("历史版本");
        let r=navigation_row(ui,"luminacraft.style.current","当前成片",Icon::Single,self.ai.selected.is_none(),true);
        lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.style.current",r.rect);
        if r.clicked(){self.ai.reset_view();self.before=false;}
        let records=self.project.style_outputs.iter().filter(|r|self.project.photos.get(self.active).is_some_and(|p|r.matches(p)))
            .map(|r|(r.id.clone(),r.template_name.clone(),r.edit_sha256.clone())).collect::<Vec<_>>();
        if records.is_empty(){ui.small("生成后保存在这里");}
        let current_hash=if records.is_empty(){None}else{self.project.photos.get(self.active).and_then(|p|project::photo_edit_hash(p).ok())};
        for (index,(id,name,hash)) in records.into_iter().enumerate(){
            let r=ui.add(egui::Button::selectable(self.ai.selected.as_ref()==Some(&id),format!("{} · {name}",index+1)).min_size(Vec2::new(ui.available_width(),36.0)).truncate());
            lightcraft_ui_egui::widgets::register(ui.ctx(),format!("luminacraft.ai.result.{id}"),r.rect);
            if r.clicked(){self.ai.selected=Some(id);self.ai.compare=false;self.load_style_preview(ui.ctx());}
            if current_hash.as_ref().is_some_and(|h|h!=&hash){ui.small("基于先前编辑生成");}
        }
        ui.add_space(12.0);
        if registered(ui,"luminacraft.style.export","导出风格图片…",self.style_record().is_some() && !self.ai.export_busy).on_hover_text("PNG / JPEG，不覆盖已有文件").clicked(){self.export_style(ui.ctx());}
        if self.ai.export_busy{if self.ai.motion_enabled{ui.spinner();}else{ui.small("正在导出…");}}
        if !self.ai.status.is_empty() && self.ai.status!=self.ai.generation_status{ui.small(&self.ai.status);}
        ui.add_space(16.0);
        ui.collapsing("服务与详情",|ui|{
            if ui.button("管理服务与自定义风格…").clicked(){self.ai.open_settings();}
            if let Some(r)=self.style_record(){ui.small(format!("{} · {}",r.service_name,r.model));ui.label(&r.prompt);}
            if builtin{ui.small("真实地理细节仍需可靠地图依据；当前未接入地图服务。");}
        });
    }
}

fn fit_image(space:Vec2,image:Vec2)->Vec2{
    let scale=(space.x.max(1.0)/image.x.max(1.0)).min(space.y.max(1.0)/image.y.max(1.0));
    image*scale
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn fitted_photos_keep_ratio_and_fit_both_axes(){
        for image in [Vec2::new(1800.0,1200.0),Vec2::new(1200.0,1800.0)]{
            for space in [Vec2::new(800.0,500.0),Vec2::new(450.0,280.0)]{
                let fit=fit_image(space,image);assert!(fit.x<=space.x+0.01 && fit.y<=space.y+0.01);
                assert!((fit.x/fit.y-image.x/image.y).abs()<0.001);
            }
        }
    }
}
