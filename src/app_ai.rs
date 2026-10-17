use super::*;
use luminacraft::ai_style as ai;
#[path="app_style.rs"] mod workbench;

#[derive(Clone,Copy,Debug,PartialEq,Eq,Default)]
enum TaskState {#[default] Idle,Working(ai::GenerationStage),Complete,Cancelled,Failed}

enum UiEvent {Preview(String,bool,Result<image::RgbaImage,String>),Connection(Result<(),String>),Export(String,Result<PublishRecord,String>)}
pub(super) struct AiUi {
    pub settings:ai::Settings,pub load_error:Option<String>,pub task:Option<ai::Task>,
    pub selected:Option<String>,pub compare:bool,pub texture:Option<(String,bool,egui::TextureHandle)>,
    template:Option<String>,prompt:String,map_location:String,map_photo:Option<(String,PathBuf)>,settings_open:bool,draft:ai::Settings,
    service_index:usize,template_index:usize,tab:bool,keys:HashMap<String,String>,
    deleted:Vec<String>,rx:mpsc::Receiver<UiEvent>,tx:mpsc::Sender<UiEvent>,connection_busy:bool,
    export_busy:bool,pub status:String,
    state:TaskState,generation_status:String,task_owner:Option<(String,project::Photo)>,
    sidebar_open:bool,compact:bool,photo_picker:bool,sample:Option<egui::TextureHandle>,
    pub(super) motion_enabled:bool,pub(super) motion_checked:Instant,
}
impl AiUi {
    pub fn new()->Self{
        let (tx,rx)=mpsc::channel();let (settings,load_error)=match ai::Settings::load(&ai::config_dir()){Ok(s)=>(s,None),Err(e)=>(ai::Settings::default(),Some(format!("AI 配置读取失败：{e}；原文件保留")))};
        Self{draft:settings.clone(),settings,load_error,task:None,selected:None,compare:false,texture:None,template:Some(luminacraft::map_style::ID.into()),prompt:String::new(),map_location:String::new(),map_photo:None,settings_open:false,service_index:0,template_index:0,tab:false,keys:HashMap::new(),deleted:vec![],rx,tx,connection_busy:false,export_busy:false,status:String::new(),state:TaskState::Idle,generation_status:String::new(),task_owner:None,sidebar_open:true,compact:false,photo_picker:false,sample:None,motion_enabled:native_motion::enabled(),motion_checked:Instant::now()}
    }
    pub(super) fn open_settings(&mut self){self.draft=self.settings.clone();self.keys.clear();self.deleted.clear();self.settings_open=true;}
    pub(super) fn export_busy(&self)->bool{self.export_busy}
    pub fn reset_view(&mut self){self.selected=None;self.texture=None;self.compare=false;}
    pub(super) fn diagnostic(&self)->serde_json::Value {
        serde_json::json!({"running":self.task.is_some(),"status":self.status,"state":format!("{:?}",self.state),"selected":self.selected,"compare":self.compare,"previewReady":self.texture.is_some(),"sidebarOpen":self.sidebar_open,"location":self.map_location,"template":self.template,"motionEnabled":self.motion_enabled})
    }
}
impl StudioApp {
    pub(super) fn select_ai_template(&mut self,id:String,prompt:String){self.ai.template=Some(id);self.ai.prompt=prompt;}
    pub(super) fn poll_ai(&mut self,ctx:&egui::Context){
        let events=self.ai.task.as_ref().map(|t|t.rx.try_iter().collect::<Vec<_>>()).unwrap_or_default();
        for event in events {match event {
            ai::Event::Stage(s)=>{self.ai.state=TaskState::Working(s);self.ai.status=s.label().into();},
            ai::Event::Finished(result)=>{
                let cancelled=self.ai.task.as_ref().is_some_and(|t|t.cancel.load(Ordering::Relaxed));
                self.ai.task=None;
                match result {
                    Ok(record)=>{
                        self.ai.state=TaskState::Complete;
                        if record.project_id==self.project.id {
                            let id=record.id.clone();let active=self.project.photos.get(self.active).is_some_and(|p|record.matches(p));
                            if !self.project.style_outputs.iter().any(|r|r.id==id){self.project.style_outputs.push(record);self.dirty=true;}
                            self.ai.status="风格版本已保存，可对比或导出。".into();
                            if active {self.ai.selected=Some(id);self.ai.compare=false;self.load_style_preview(ctx);}
                        }else {self.ai.status="结果已保存在提交时的工程目录，重新打开该工程即可恢复。".into();}
                    },Err(e)=>{self.ai.state=if cancelled{TaskState::Cancelled}else{TaskState::Failed};self.ai.status=e;},
                }
            }
        }self.ai.generation_status=self.ai.status.clone();}
        while let Ok(event)=self.ai.rx.try_recv(){match event {
            UiEvent::Preview(id,before,result)=>{
                if self.ai.selected.as_ref()!=Some(&id) || self.ai.compare!=before{continue;}
                match result {Ok(image)=>{
                    let color=egui::ColorImage::from_rgba_unmultiplied([image.width()as usize,image.height()as usize],image.as_raw());
                    self.ai.texture=Some((id,before,ctx.load_texture("ai-style",color,egui::TextureOptions::LINEAR)));
                },Err(e)=>{self.ai.texture=None;self.ai.status=e;}}
            },
            UiEvent::Connection(result)=>{self.ai.connection_busy=false;self.ai.status=match result{Ok(())=>"连接与模型列表检查通过；尚未生成图片。".into(),Err(e)=>e};},
            UiEvent::Export(owner,result)=>{self.ai.export_busy=false;match result{Ok(record)=>{self.ai.status=format!("已导出：{}",record.output.display());if owner==self.project.id{self.record_local_export(record,false);}},Err(e)=>self.ai.status=e}},
        }}
        if self.ai.task.is_some() || self.ai.connection_busy || self.ai.export_busy{ctx.request_repaint_after(std::time::Duration::from_millis(100));}
    }
    pub(super) fn load_style_preview(&mut self,ctx:&egui::Context){
        self.ai.texture=None;
        let Some(record)=self.project.style_outputs.iter().find(|r|Some(&r.id)==self.ai.selected.as_ref() && self.project.photos.get(self.active).is_some_and(|p|r.matches(p))).cloned()else{return};
        let tx=self.ai.tx.clone();let ctx=ctx.clone();let before=self.ai.compare;
        std::thread::spawn(move||{let result=record.preview(before).map_err(|e|e.to_string());let _=tx.send(UiEvent::Preview(record.id,before,result));ctx.request_repaint();});
    }
    pub(super) fn start_style(&mut self,ctx:&egui::Context){
        if self.ai.task.is_some() || self.busy || self.advanced.is_some(){return;}
        let result=(||->anyhow::Result<ai::Generate>{
            anyhow::ensure!(self.ai.load_error.is_none(),"请先修复 AI 配置文件");
            let builtin=ai::Template{id:luminacraft::map_style::ID.into(),name:luminacraft::map_style::NAME.into(),prompt:String::new(),service_id:None};
            let template=if self.ai.template.as_deref()==Some(luminacraft::map_style::ID){&builtin}else{self.ai.settings.templates.iter().find(|t|Some(&t.id)==self.ai.template.as_ref()).ok_or_else(||anyhow::anyhow!("请先添加并选择风格模板"))?};
            let service=self.ai.settings.service_for(template).ok_or_else(||anyhow::anyhow!("请在设置中为风格选择服务"))?.clone();
            service.validate()?;
            let photo=self.project.photos.get(self.active).ok_or_else(||anyhow::anyhow!("请先载入照片"))?.clone();
            let directory=self.directory.clone().ok_or_else(||anyhow::anyhow!("请先保存工程，再生成风格版本"))?;
            anyhow::ensure!(self.project.style_outputs.len()<1000,"风格版本已满");
            Ok(ai::Generate{project_id:self.project.id.clone(),directory,photo,service,template_name:template.name.clone(),prompt:if template.id==luminacraft::map_style::ID{self.ai.map_location.clone()}else{ai::validate_prompt(&self.ai.prompt)?;self.ai.prompt.clone()}})
        })();
        match result {Ok(request)=>{self.ai.task_owner=Some((request.project_id.clone(),request.photo.clone()));self.ai.state=TaskState::Working(ai::GenerationStage::Preparing);self.ai.status="准备生成…".into();self.ai.task=Some(ai::start(request,ctx.clone()));},Err(e)=>{self.ai.state=TaskState::Failed;self.ai.status=e.to_string();}}
        self.ai.generation_status=self.ai.status.clone();
    }
    fn export_style(&mut self,ctx:&egui::Context){
        if self.ai.export_busy||self.busy{return;}
        if self.project.publishes.len()>=1000{self.ai.status="导出历史已满，请先在导出页移除旧记录。".into();return;}
        let Some(record)=self.project.style_outputs.iter().find(|r|Some(&r.id)==self.ai.selected.as_ref() && self.project.photos.get(self.active).is_some_and(|p|r.matches(p))).cloned()else{return};
        let Some(path)=rfd::FileDialog::new().set_file_name(format!("{}-style.png",record.id)).add_filter("PNG",&["png"]).add_filter("JPEG",&["jpg","jpeg"]).save_file()else{return};
        let ext=path.extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(),"png"|"jpg"|"jpeg"){self.ai.status="请选择 PNG 或 JPEG 文件扩展名。".into();return;}
        let tx=self.ai.tx.clone();let ctx=ctx.clone();self.ai.export_busy=true;
        let owner=self.project.id.clone();
        std::thread::spawn(move||{let result=(||->anyhow::Result<PublishRecord>{record.export(&path,ext!="png")?;Ok(PublishRecord{lumina_base:None,source_sha256:record.source_sha256.clone(),edit_sha256:record.edit_sha256.clone(),output_sha256:project::hash_file(&path)?,output:path,asset_id:None,collection_id:None,origin:Some(format!("ai-style:{}",record.id))})})().map_err(|e|e.to_string());let _=tx.send(UiEvent::Export(owner,result));ctx.request_repaint();});
    }
    pub(super) fn ai_settings_ui(&mut self,ctx:&egui::Context){
        if !self.ai.settings_open{return;}
        let mut open=true;
        egui::Window::new("AI 服务与风格").open(&mut open).default_width(700.0).default_height(520.0).show(ctx,|ui|{
            ui.horizontal(|ui|{ui.selectable_value(&mut self.ai.tab,false,"OpenAI 服务");ui.selectable_value(&mut self.ai.tab,true,"风格模板");});ui.separator();
            egui::ScrollArea::vertical().max_height(510.0).show(ui,|ui|{
                if !self.ai.tab {
                    ui.horizontal_wrapped(|ui|{for (i,s) in self.ai.draft.services.iter().enumerate(){if ui.selectable_label(self.ai.service_index==i,&s.name).clicked(){self.ai.service_index=i;}}});
                    if ui.button("新增服务配置").clicked(){let s=ai::Service::default();if self.ai.draft.default_service.is_none(){self.ai.draft.default_service=Some(s.id.clone());}self.ai.draft.services.push(s);self.ai.service_index=self.ai.draft.services.len()-1;}
                    let mut remove=None;
                    if let Some(s)=self.ai.draft.services.get_mut(self.ai.service_index){
                        ui.label("名称");ui.text_edit_singleline(&mut s.name);ui.label("API 基础地址（含 /v1）");ui.text_edit_singleline(&mut s.base_url);ui.label("图片模型 ID");ui.text_edit_singleline(&mut s.model);
                        ui.label("API 密钥");ui.add(egui::TextEdit::singleline(self.ai.keys.entry(s.id.clone()).or_default()).password(true).hint_text("留空保留已保存的密钥"));
                        ui.small("密钥仅存入 Windows 凭据存储。DeepSeek 原生出图暂未启用。");
                        let mut default=self.ai.draft.default_service.as_ref()==Some(&s.id);if ui.checkbox(&mut default,"默认服务").changed(){self.ai.draft.default_service=if default{Some(s.id.clone())}else{None};}
                        ui.horizontal(|ui|{
                            if ui.add_enabled(!self.ai.connection_busy && self.ai.keys.get(&s.id).is_none_or(|k|k.is_empty()),egui::Button::new("测试已保存密钥的连接")).clicked(){
                                self.ai.connection_busy=true;let s=s.clone();let tx=self.ai.tx.clone();let ctx=ctx.clone();
                                std::thread::spawn(move||{let result=ai::test_connection(&s,&ai::WindowsSecrets).map_err(|e|e.to_string());let _=tx.send(UiEvent::Connection(result));ctx.request_repaint();});
                            }
                            if ui.button("删除此配置").clicked(){remove=Some(s.id.clone());}
                        });ui.small("测试只读取模型列表，不上传照片；更改地址后需重新填写密钥。");
                    }
                    if let Some(id)=remove{self.ai.draft.services.retain(|s|s.id!=id);self.ai.deleted.push(id.clone());self.ai.keys.remove(&id);for t in &mut self.ai.draft.templates{if t.service_id.as_ref()==Some(&id){t.service_id=None;}}if self.ai.draft.default_service.as_ref()==Some(&id){self.ai.draft.default_service=self.ai.draft.services.first().map(|s|s.id.clone());}self.ai.service_index=0;}
                }else{
                    ui.horizontal_wrapped(|ui|{for (i,t) in self.ai.draft.templates.iter().enumerate(){if ui.selectable_label(self.ai.template_index==i,&t.name).clicked(){self.ai.template_index=i;}}});
                    ui.horizontal(|ui|{
                        if ui.button("新增风格").clicked(){self.ai.draft.templates.push(ai::Template::default());self.ai.template_index=self.ai.draft.templates.len()-1;}
                        if ui.button("复制风格").clicked(){if let Some(t)=self.ai.draft.templates.get(self.ai.template_index){let mut copy=t.clone();copy.id=ai::new_id();copy.name=format!("{} 副本",copy.name);self.ai.draft.templates.push(copy);self.ai.template_index=self.ai.draft.templates.len()-1;}}
                    });
                    let mut remove=false;let services=self.ai.draft.services.clone();
                    if let Some(t)=self.ai.draft.templates.get_mut(self.ai.template_index){
                        ui.label("风格名称");ui.text_edit_singleline(&mut t.name);
                        egui::ComboBox::from_id_salt("ai-template-service").selected_text(t.service_id.as_ref().and_then(|id|services.iter().find(|s|&s.id==id)).map(|s|s.name.as_str()).unwrap_or("使用默认服务")).show_ui(ui,|ui|{ui.selectable_value(&mut t.service_id,None,"使用默认服务");for s in &services{ui.selectable_value(&mut t.service_id,Some(s.id.clone()),&s.name);}});
                        ui.label("提示词（原样发送）");ui.add(egui::TextEdit::multiline(&mut t.prompt).desired_rows(12).desired_width(f32::INFINITY));
                        remove=ui.button("删除此风格").clicked();
                    }
                    if remove{self.ai.draft.templates.remove(self.ai.template_index);self.ai.template_index=0;}
                }
            });
            ui.separator();ui.label(&self.ai.status);
            ui.horizontal(|ui|{
                if ui.add_enabled(self.ai.load_error.is_none(),egui::Button::new("保存配置与模板")).clicked(){
                    match self.save_ai_settings(){Ok(())=>{self.ai.settings_open=false;},Err(e)=>self.ai.status=e.to_string()}
                }
                if self.ai.load_error.is_some() && ui.button("重新读取配置文件").clicked(){match ai::Settings::load(&ai::config_dir()){Ok(s)=>{self.ai.settings=s.clone();self.ai.draft=s;self.ai.load_error=None;self.ai.status="配置已重新载入。".into();},Err(e)=>self.ai.status=e.to_string()}}
                if ui.button("取消更改").clicked(){self.ai.settings_open=false;self.ai.keys.clear();}
            });
        });
        if !open{self.ai.settings_open=false;self.ai.keys.clear();}
    }
    fn save_ai_settings(&mut self)->anyhow::Result<()>{
        let warnings=self.ai.draft.save_with_secrets(&ai::config_dir(),&self.ai.settings,&self.ai.keys,&ai::WindowsSecrets)?;
        self.ai.settings=self.ai.draft.clone();self.ai.keys.clear();self.ai.deleted.clear();
        self.ai.status=if warnings.is_empty(){"配置与模板已保存。".into()}else{format!("配置已保存；{}",warnings.join("；"))};
        // Keep the current invocation's edited prompt until the user selects a template again.
        if self.ai.template.as_ref().is_some_and(|id|id!=luminacraft::map_style::ID && !self.ai.settings.templates.iter().any(|t|&t.id==id)){self.ai.template=None;self.ai.prompt.clear();}
        Ok(())
    }
}
