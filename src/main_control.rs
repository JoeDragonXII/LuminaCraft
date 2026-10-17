//! Opt-in localhost diagnostics for native workflow acceptance; disabled normally.
use super::*;
use lightcraft_ui_egui::control::{ControlRequest,ControlResponse};
pub(super) struct MainControl {
    rx:mpsc::Receiver<ControlRequest>,
    shot:Option<(PathBuf,mpsc::Sender<ControlResponse>)>,
    widgets:Vec<(String,egui::Rect)>,synthetic:Vec<egui::Event>,
    layout_samples:std::collections::VecDeque<serde_json::Value>,
}
impl MainControl {
    pub(super) fn new(ctx:&egui::Context)->Option<Self>{
        let port=std::env::var("LUMINACRAFT_MAIN_CONTROL_PORT").ok()?.parse().ok()?;
        Some(Self{rx:crate::upstream_desktop::start_main_control(port,ctx.clone()),shot:None,widgets:vec![],synthetic:vec![],layout_samples:Default::default()})
    }
    pub(super) fn inject(&mut self,input:&mut egui::RawInput){input.events.append(&mut self.synthetic);}
}
impl StudioApp {
    pub(super) fn record_main_widgets(&mut self,ctx:&egui::Context){let widgets=lightcraft_ui_egui::widgets::take_registry(ctx);if let Some(c)=&mut self.main_control{
        let rect=|id:&str|widgets.iter().find(|(name,_)|name==id).map(|(_,r)|[r.min.x,r.min.y,r.max.x,r.max.y]);
        if c.layout_samples.len()>=512{c.layout_samples.pop_front();}
        c.layout_samples.push_back(serde_json::json!({"busy":self.busy,"blocking":self.blocking,"pending":self.pending_preview.is_some(),"canvas":rect("luminacraft.style.canvas"),"footer":rect("luminacraft.status"),"cancel":rect("luminacraft.task.cancel")}));
        c.widgets=widgets;
    }}
    pub(super) fn poll_main_control(&mut self,ctx:&egui::Context){
        let Some(control)=&mut self.main_control else{return};
        let requests:Vec<_>=control.rx.try_iter().collect();
        if let Some(image)=ctx.input(|i|i.events.iter().find_map(|e|if let egui::Event::Screenshot{image,..}=e{Some(image.clone())}else{None})){
            if let Some((path,reply))=control.shot.take(){
                let result=(||->anyhow::Result<()>{
                    use image::ImageEncoder;
                    let file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
                    let bytes:Vec<_>=image.pixels.iter().flat_map(|p|p.to_array()).collect();
                    image::codecs::png::PngEncoder::new(file).write_image(&bytes,image.width() as u32,image.height() as u32,image::ExtendedColorType::Rgba8)?;Ok(())
                })();
                let _=reply.send(match result{Ok(())=>serde_json::json!({"ok":true,"result":null}),Err(e)=>serde_json::json!({"ok":false,"error":e.to_string()})});
            }
        }
        for request in requests {
            if !request.take(){continue;}
            let mut delayed=false;
            let result=match request.method.as_str(){
                "workflow.layoutSamples"=>{
                    let c=self.main_control.as_mut().unwrap();
                    let samples=if request.params.get("clear").and_then(|v|v.as_bool())==Some(true){c.layout_samples.drain(..).collect::<Vec<_>>()}else{c.layout_samples.iter().cloned().collect()};
                    serde_json::json!({"ok":true,"result":samples})
                },
                "workflow.history"=>serde_json::json!({"ok":true,"result":self.history_diagnostic()}),
                "workflow.selection"=>{
                    let photos:Vec<_>=self.library.library.as_ref().map(|lib|self.library.selected_ids().into_iter().filter_map(|id|lib.catalog.photo(lightcraft_catalog::PhotoId(id))).map(|p|serde_json::json!({"id":p.id.0,"rating":p.rating,"flag":p.flag,"label":p.label})).collect()).unwrap_or_default();
                    serde_json::json!({"ok":true,"result":photos})
                },
                "workflow.catalog"=>match self.catalog_diagnostic(&request.params){Ok(())=>serde_json::json!({"ok":true,"result":null}),Err(e)=>serde_json::json!({"ok":false,"error":e.to_string()})},
                "workflow.ai"=>{
                    self.mode=Mode::Style;self.brush=Brush::None;
                    if request.params.get("settings").and_then(|v|v.as_bool())==Some(true){self.ai.open_settings();}
                    if let Some(t)=self.ai.settings.templates.iter().find(|t|request.params.get("template").and_then(|v|v.as_str())==Some(t.id.as_str())).cloned(){self.select_ai_template(t.id,t.prompt);}
                    if request.params.get("template").and_then(|v|v.as_str())==Some(luminacraft::map_style::ID){self.select_ai_template(luminacraft::map_style::ID.into(),String::new());}
                    let mut result=self.ai.diagnostic();result["outputs"]=self.project.style_outputs.len().into();result["zoom"]=self.zoom.into();
                    serde_json::json!({"ok":true,"result":result})
                },
                "ui.resize"=>{
                    let w=request.params.get("width").and_then(|v|v.as_f64()).unwrap_or(1380.0).clamp(980.0,2560.0);
                    let h=request.params.get("height").and_then(|v|v.as_f64()).unwrap_or(900.0).clamp(680.0,1800.0);
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w as f32,h as f32)));
                    serde_json::json!({"ok":true,"result":null})
                },
                "ui.text"=>{
                    if let Some(text)=request.params.get("text").and_then(|v|v.as_str()).filter(|s|s.len()<=2000){if let Some(c)=&mut self.main_control{if request.params.get("replace").and_then(|v|v.as_bool())==Some(true){for pressed in [true,false]{c.synthetic.push(egui::Event::Key{key:egui::Key::A,physical_key:None,pressed,repeat:false,modifiers:egui::Modifiers{ctrl:true,command:true,..Default::default()}});}}c.synthetic.push(egui::Event::Text(text.into()));}}
                    serde_json::json!({"ok":true,"result":null})
                },
                "ui.widgets"=>serde_json::json!({"ok":true,"result":self.main_control.as_ref().map(|c|c.widgets.iter().map(|(id,r)|serde_json::json!({"id":id,"rect":[r.min.x,r.min.y,r.max.x,r.max.y]})).collect::<Vec<_>>())}),
                "ui.scroll"=>{
                    if let Some(c)=&mut self.main_control{let at=egui::pos2(request.params.get("x").and_then(|v|v.as_f64()).unwrap_or(110.0)as f32,request.params.get("y").and_then(|v|v.as_f64()).unwrap_or(600.0)as f32);c.synthetic.extend([egui::Event::PointerMoved(at),egui::Event::MouseWheel{unit:egui::MouseWheelUnit::Point,phase:egui::TouchPhase::Move,delta:egui::vec2(0.0,request.params.get("dy").and_then(|v|v.as_f64()).unwrap_or(-300.0)as f32),modifiers:Default::default()}]);}serde_json::json!({"ok":true,"result":null})
                },
                "ui.clickWidget"=>{
                    let rect=self.main_control.as_ref().and_then(|c|c.widgets.iter().find(|(id,_)|request.params.get("id").and_then(|v|v.as_str())==Some(id.as_str())).map(|(_,r)|*r));
                    if let (Some(r),Some(c))=(rect,&mut self.main_control){let fraction=request.params.get("xFraction").and_then(|v|v.as_f64()).unwrap_or(0.5).clamp(0.01,0.99)as f32;let at=egui::pos2(r.left()+r.width()*fraction,r.center().y);let button=if request.params.get("button").and_then(|v|v.as_str())==Some("right"){egui::PointerButton::Secondary}else{egui::PointerButton::Primary};let phase=request.params.get("phase").and_then(|v|v.as_str()).unwrap_or("click");c.synthetic.push(egui::Event::PointerMoved(at));if phase!="release"&&phase!="move"{c.synthetic.push(egui::Event::PointerButton{pos:at,button,pressed:true,modifiers:Default::default()});}if phase!="press"&&phase!="move"{c.synthetic.push(egui::Event::PointerButton{pos:at,button,pressed:false,modifiers:Default::default()});}serde_json::json!({"ok":true,"result":null})}else{serde_json::json!({"ok":false,"error":"widget not visible"})}
                },
                "workflow.inspect"=>serde_json::json!({"ok":true,"result":{"busy":self.busy,"blocking":self.blocking,"pendingPreview":self.pending_preview.is_some(),"luminaConnected":false,"committing":self.commit_gate.committing(),"exportSelected":self.export_view.as_ref().map(|r|r.output.clone()),"exportPreviewReady":self.export_texture.is_some(),"activePhoto":self.active,"mode":format!("{:?}",self.mode),"editorMode":format!("{:?}",self.editor_mode),"zoom":self.zoom,"before":self.before,"previewReady":self.texture.is_some(),"jobRevision":self.id,"project":self.project,"dirty":self.dirty,"message":self.message,"advancedPid":self.advanced.as_ref().map(|a|a.child.id()),"libraryOpen":self.library.library.is_some(),"photosInLibrary":self.library.library.as_ref().map(|l|l.catalog.len()),"visibleInLibrary":self.library.library.as_ref().map(|l|l.catalog.query(&lightcraft_catalog::Filter::default(),&lightcraft_catalog::Sort::default()).len()),"deletedInLibrary":self.library.library.as_ref().map(|l|l.catalog.photos().filter(|p|p.deleted).count()),"recoveries":self.recovery_paths,"checkpoint":self.active_checkpoint}}),
                "workflow.reconnect"=>{self.reconnect_library();serde_json::json!({"ok":self.library.library.is_some(),"result":self.message})},
                "workflow.recover"=>{
                    let outcome=request.params.get("path").and_then(|v|v.as_str()).map(PathBuf::from).ok_or_else(||anyhow::anyhow!("path required")).and_then(|path|self.recover_edit(path));
                    match outcome{Ok(())=>serde_json::json!({"ok":true,"result":null}),Err(e)=>serde_json::json!({"ok":false,"error":e.to_string()})}
                },
                "workflow.exportLocal"=>{
                    let result=request.params.get("path").and_then(|v|v.as_str()).map(PathBuf::from).ok_or_else(||anyhow::anyhow!("path required")).and_then(|path|self.start_local_export(path));
                    match result{Ok(())=>serde_json::json!({"ok":true,"result":null}),Err(e)=>serde_json::json!({"ok":false,"error":e.to_string()})}
                },
                "workflow.copyExport"=>{
                    let result=(||->anyhow::Result<()>{anyhow::ensure!(!self.busy&&!self.ai.export_busy()&&self.project.publishes.len()<1000,"busy or history full");let record=self.export_view.clone().ok_or_else(||anyhow::anyhow!("no selection"))?;let path=PathBuf::from(request.params.get("path").and_then(|v|v.as_str()).ok_or_else(||anyhow::anyhow!("path required"))?);self.submit(Job::LocalCopy(record,path),"保存成片副本…");Ok(())})();
                    match result{Ok(())=>serde_json::json!({"ok":true,"result":null}),Err(e)=>serde_json::json!({"ok":false,"error":e.to_string()})}
                },
                "workflow.command"=>{
                    if self.busy||self.advanced.is_some()||self.project.photos.is_empty(){serde_json::json!({"ok":false,"error":"workflow busy or empty"})}
                    else{match serde_json::from_value::<Command>(request.params.clone()){Ok(command)=>{self.command(command,None);serde_json::json!({"ok":true,"result":null})},Err(e)=>serde_json::json!({"ok":false,"error":e.to_string()})}}
                },
                "workflow.openAdvanced"=>{
                    if self.busy||self.advanced.is_some(){serde_json::json!({"ok":false,"error":"workflow busy"})}
                    else{let purpose=match request.params.get("purpose").and_then(|v|v.as_str()){Some("import")=>luminacraft::bridge::Purpose::Import,Some("library")=>luminacraft::bridge::Purpose::Library,_=>luminacraft::bridge::Purpose::Develop};self.open_advanced(purpose);serde_json::json!({"ok":self.advanced.is_some(),"result":null,"error":self.message})}
                },
                "workflow.screenshot"=>{
                    if let Some(path)=request.params.get("path").and_then(|v|v.as_str()){
                        if let Some(control)=&mut self.main_control{control.shot=Some((PathBuf::from(path),request.reply.clone()));}
                        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));delayed=true;
                        serde_json::Value::Null
                    }else{serde_json::json!({"ok":false,"error":"path required"})}
                },
                "workflow.quit"=>{
                    if self.advanced.is_some(){serde_json::json!({"ok":false,"error":"return from advanced first"})}
                    else{if request.params.get("discard").and_then(|v|v.as_bool())==Some(true){self.dismiss_checkpoint();self.allow_close=true;}
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);serde_json::json!({"ok":true,"result":null})}
                },
                _=>serde_json::json!({"ok":false,"error":"unknown workflow diagnostic"}),
            };
            if !delayed{let _=request.reply.send(result);}
            ctx.request_repaint();
        }
    }
}
