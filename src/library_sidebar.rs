//! Main-window library sources and folder context menus; owned catalog operations only.
use super::*;
use luminacraft::library::{FolderPlan,FolderChoices};

pub(super) enum FolderOutcome {Preview(FolderPlan),Commit{revision:u64,ops:Vec<Op>}}
pub(super) struct FolderTask {rx:mpsc::Receiver<Result<FolderOutcome,String>>,cancel:Arc<AtomicBool>}
impl Drop for FolderTask{fn drop(&mut self){self.cancel.store(true,Ordering::Relaxed);}}
#[derive(Default)]
pub(super) struct FolderUi {
    pub task:Option<FolderTask>,plan:Option<FolderPlan>,choices:FolderChoices,
    remove:Option<(String,Vec<PhotoId>,u64)>,create:Option<(PathBuf,String)>,
    pub project_paths_changed:bool,
    pub show_parents:bool,
    sources:Sources,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(browser:&mut LibraryUi,ctx:&egui::Context,events:Vec<egui::Event>)->Vec<(String,egui::Rect)>{
        let result=ctx.run_ui(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,Vec2::new(1380.0,900.0))),events,..Default::default()},|ui|{browser.show(ui,false);});result.drop_without_applying_deltas();lightcraft_ui_egui::widgets::take_registry(ctx)
    }
    fn click(browser:&mut LibraryUi,ctx:&egui::Context,rect:egui::Rect,button:egui::PointerButton)->Vec<(String,egui::Rect)>{
        let pos=rect.center();frame(browser,ctx,vec![egui::Event::PointerMoved(pos)]);
        frame(browser,ctx,vec![egui::Event::PointerButton{pos,button,pressed:true,modifiers:Default::default()}]);
        frame(browser,ctx,vec![egui::Event::PointerButton{pos,button,pressed:false,modifiers:Default::default()}])
    }
    #[test]
    fn folder_context_removal_uses_folder_scope_and_supports_undo(){
        let tmp=tempfile::tempdir().unwrap();let folder=tmp.path().join("photos");std::fs::create_dir_all(folder.join("nested")).unwrap();let other=tmp.path().join("outside.png");
        let paths=[folder.join("one.png"),folder.join("nested/two.png"),other];
        for (n,path) in paths.iter().enumerate(){image::RgbImage::from_pixel(24,16,image::Rgb([n as u8*30,70,100])).save(path).unwrap();}
        let ctx=egui::Context::default();crate::typography::install(&ctx);let mut browser=LibraryUi::from_opened(&ctx,Library::open(&tmp.path().join("catalog")));
        browser.library.as_mut().unwrap().add(paths.iter().map(|p|library::index_photo(p).unwrap()).collect()).unwrap();
        frame(&mut browser,&ctx,vec![]);let widgets=frame(&mut browser,&ctx,vec![]);
        let key=lightcraft_catalog::query::folder_key(&folder.to_string_lossy());let rect=widgets.iter().find(|(id,_)|id.strip_prefix("main.folder:").is_some_and(|p|lightcraft_catalog::query::folder_key(p)==key)).unwrap().1;
        let menu=click(&mut browser,&ctx,rect,egui::PointerButton::Secondary);let remove=menu.iter().find(|(id,_)|id=="main.folder.remove").unwrap().1;
        let dialog=click(&mut browser,&ctx,remove,egui::PointerButton::Primary);let confirm=dialog.iter().find(|(id,_)|id=="main.folder.confirmRemove").unwrap().1;
        click(&mut browser,&ctx,confirm,egui::PointerButton::Primary);
        assert_eq!(browser.library.as_ref().unwrap().catalog.photos().filter(|p|p.deleted).count(),2);assert!(paths.iter().all(|p|p.exists()));
        browser.undo_catalog(false);assert_eq!(browser.library.as_ref().unwrap().catalog.photos().filter(|p|p.deleted).count(),0);
        browser.folder_label(folder.to_str().unwrap(),Some(ColorLabel::Green));assert_eq!(browser.library.as_ref().unwrap().catalog.folder_color_label(folder.to_str().unwrap()),Some(ColorLabel::Green));
        let album=AlbumId(1);browser.apply_named(vec![Op::AddAlbum{album:Album::new(album,"empty album")}],"创建相册");
        browser.filter=Filter{date:Some("2025".into()),keyword:Some("unrelated".into()),imported_from:Some("2026".into()),..Default::default()};browser.select_album_source(album);
        assert_eq!(browser.filter.album,Some(album));assert!(browser.filter.date.is_none()&&browser.filter.keyword.is_none()&&browser.filter.imported_from.is_none());
    }
}
#[derive(Default)]
struct Sources {revision:Option<u64>,total:usize,picks:usize,deleted:usize,folders:Arc<Vec<FolderNode>>,dates:Arc<Vec<lightcraft_catalog::query::DateGroup>>,keywords:Arc<Vec<lightcraft_catalog::keywords::KeywordNode>>}
impl LibraryUi {
    pub(super) fn select_album_source(&mut self,id:AlbumId){self.filter=Filter{album:Some(id),..Default::default()};}
    pub(super) fn cancel_folder_task(&self){if let Some(t)=&self.folders.task{t.cancel.store(true,Ordering::Relaxed);}}
    pub fn take_path_changes(&mut self)->bool{std::mem::take(&mut self.folders.project_paths_changed)}
    fn refresh_sources(&mut self){
        let Some(lib)=&self.library else{return};if self.folders.sources.revision==Some(lib.catalog.revision){return;}
        self.folders.sources=Sources{revision:Some(lib.catalog.revision),
            total:lib.catalog.query(&Filter::default(),&Sort::default()).len(),
            picks:lib.catalog.query(&Filter{flag:Some(Flag::Pick),..Default::default()},&Sort::default()).len(),
            deleted:lib.catalog.query(&Filter{deleted:true,..Default::default()},&Sort::default()).len(),
            folders:Arc::new(lib.catalog.folder_tree()),dates:Arc::new(lib.catalog.date_groups()),keywords:Arc::new(lib.catalog.keyword_tree())};
    }
    pub(super) fn basic_sources_ui(&mut self,ui:&mut egui::Ui){
        self.refresh_sources();let total=self.folders.sources.total;
        let plain=self.filter.album.is_none()&&self.filter.library_folder.is_none()&&self.filter.date.is_none()&&self.filter.keyword.is_none()&&self.filter.imported_from.is_none()&&!self.filter.deleted&&self.filter.flag!=Some(Flag::Pick);
        if crate::chrome::side_tab(ui,"main.source.all",&format!("所有照片   {total}"),plain).clicked(){self.filter=Filter::default();}
        let recent=Self::recent_filter();
        if crate::chrome::side_tab(ui,"main.source.recent","最近添加",self.filter.imported_from.is_some()).on_hover_text("最近 7 天导入照片库的照片").clicked(){self.filter=recent;}
        let picks=self.folders.sources.picks;
        if crate::chrome::side_tab(ui,"main.source.picks",&format!("留用   {picks}"),self.filter.flag==Some(Flag::Pick)).clicked(){self.filter=Filter{flag:Some(Flag::Pick),..Default::default()};}
        let deleted=self.folders.sources.deleted;
        if crate::chrome::side_tab(ui,"main.source.deleted",&format!("最近删除   {deleted}"),self.filter.deleted).on_hover_text("只移出照片库；原片和 XMP 保留，可恢复").clicked(){self.filter=Filter{deleted:true,..Default::default()};}
    }
    fn recent_filter()->Filter{
        let secs=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs().saturating_sub(7*86400);
        Filter{imported_from:Some(lightcraft_engine::import::civil(secs as i64)),..Default::default()}
    }
    pub(super) fn folder_sections_ui(&mut self,ui:&mut egui::Ui,editor_busy:bool){
        let ready=!self.scanning&&!editor_busy;
        ui.add_space(16.0);egui::CollapsingHeader::new("本地").id_salt("main-local").default_open(false).show(ui,|ui|{
            let home=std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
            for (name,path) in [("图片",home.join("Pictures")),("桌面",home.join("Desktop")),("下载",home.join("Downloads")),("主文件夹",home)]{
                if ui.add_enabled(ready,egui::Button::new(format!("▸ {name}…"))).on_hover_text("选择此位置的照片文件夹，添加引用到照片库").clicked(){
                    if let Some(dir)=rfd::FileDialog::new().set_title("选择照片文件夹（添加引用）").set_directory(path).pick_folder(){self.scan(vec![dir],ui.ctx());}
                }
            }
            if ui.add_enabled(ready,egui::Button::new("浏览文件夹…")).clicked(){if let Some(dir)=rfd::FileDialog::new().set_title("添加照片文件夹（保留原位置）").pick_folder(){self.scan(vec![dir],ui.ctx());}}
        });
        self.refresh_sources();let dates=self.folders.sources.dates.clone();
        ui.add_space(16.0);egui::CollapsingHeader::new("按日期").id_salt("main-dates").default_open(false).show(ui,|ui|{
            for year in dates.iter(){
                egui::CollapsingHeader::new(format!("{} 年   {}",year.year,year.count)).id_salt(("year",&year.year)).show(ui,|ui|{
                    if ui.selectable_label(self.filter.date.as_ref()==Some(&year.year),"全年").clicked(){self.filter=Filter{date:Some(year.year.clone()),..Default::default()};}
                    for (month,count) in &year.months {
                        egui::CollapsingHeader::new(format!("{} 月   {count}",month.get(5..).unwrap_or(month))).id_salt(("month",month)).show(ui,|ui|{
                            if ui.selectable_label(self.filter.date.as_ref()==Some(month),"整月").clicked(){self.filter=Filter{date:Some(month.clone()),..Default::default()};}
                            for (day,count) in year.days.iter().filter(|(day,_)|day.starts_with(month)){
                                if ui.selectable_label(self.filter.date.as_ref()==Some(day),format!("{} 日   {count}",day.get(8..).unwrap_or(day))).clicked(){self.filter=Filter{date:Some(day.clone()),..Default::default()};}
                            }
                        });
                    }
                });
            }
        });
        let folders=self.folders.sources.folders.clone();
        ui.add_space(16.0);egui::CollapsingHeader::new("文件夹").id_salt("main-folders").default_open(true).show(ui,|ui|{
            ui.checkbox(&mut self.folders.show_parents,"显示完整父级路径");
            self.folder_tree_ui(ui,&folders,ready,false);
        });
        let keywords=self.folders.sources.keywords.clone();
        ui.add_space(16.0);egui::CollapsingHeader::new("关键词").id_salt("main-keywords").default_open(false).show(ui,|ui|{
            if keywords.is_empty(){ui.weak("还没有关键词");}
            self.keyword_tree_ui(ui,&keywords);
        });
    }
    fn keyword_tree_ui(&mut self,ui:&mut egui::Ui,nodes:&[lightcraft_catalog::keywords::KeywordNode]){
        for node in nodes{
            if node.children.is_empty(){if ui.selectable_label(self.filter.keyword.as_ref()==Some(&node.path),format!("{}   {}",node.name,node.count)).clicked(){self.filter=Filter{keyword:Some(node.path.clone()),..Default::default()};}}
            else{egui::CollapsingHeader::new(&node.name).id_salt(("main-keyword",&node.path)).show(ui,|ui|{if ui.selectable_label(self.filter.keyword.as_ref()==Some(&node.path),format!("全部   {}",node.count)).clicked(){self.filter=Filter{keyword:Some(node.path.clone()),..Default::default()};}self.keyword_tree_ui(ui,&node.children);});}
        }
    }
    fn folder_tree_ui(&mut self,ui:&mut egui::Ui,nodes:&[FolderNode],ready:bool,compact:bool){
        for node in nodes{
            let mut node=node;
            if compact{while node.own==0 && node.children.len()==1{node=&node.children[0];}}
            if node.children.is_empty(){self.folder_row(ui,node,ready);}else{
                let id=ui.make_persistent_id(("main-folder",&node.path));
                egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(),id,true).show_header(ui,|ui|{self.folder_row(ui,node,ready);}).body(|ui|self.folder_tree_ui(ui,&node.children,ready,node.volume&&!self.folders.show_parents));
            }
        }
    }
    fn folder_row(&mut self,ui:&mut egui::Ui,node:&FolderNode,ready:bool){
        let mut text=RichText::new(format!("{}   {}",node.name,node.count));if let Some(label)=node.label{if let Some((_,color))=label_colors().iter().find(|(l,_)|*l==label){text=text.color(*color);}}
        let response=ui.add(egui::Button::selectable(self.filter.library_folder.as_ref()==Some(&node.path),text).truncate()).on_hover_text(&node.path);
        lightcraft_ui_egui::widgets::register(ui.ctx(),format!("main.folder:{}",node.path),response.rect);
        if response.clicked(){self.filter=Filter{library_folder:Some(node.path.clone()),..Default::default()};}
        response.context_menu(|ui|{
            ui.add_enabled(false,egui::Button::new("重命名文件夹…")).on_hover_text("原片保护模式下暂不可用");
            ui.add_enabled(false,egui::Button::new("将文件夹移动到…")).on_hover_text("原片保护模式下暂不可用");
            if ui.add_enabled(ready&&std::path::Path::new(&node.path).is_dir(),egui::Button::new("在磁盘上新建文件夹…")).clicked(){self.folders.create=Some((PathBuf::from(&node.path),String::new()));ui.close();}
            ui.add_enabled(false,egui::Button::new("将所选照片移到此处")).on_hover_text("归档移动请使用“导入与归档…”");
            if ui.button("在文件资源管理器中显示").clicked(){
                #[cfg(windows)]{let result=std::process::Command::new("explorer.exe").arg(&node.path).spawn();if let Err(e)=result{self.message=format!("无法打开资源管理器：{e}");}}
                ui.close();
            }
            ui.add_enabled_ui(ready,|ui|{ui.menu_button("设置颜色标签",|ui|{
                for (label,_) in label_colors(){if ui.button(label_name(Some(label))).clicked(){self.folder_label(&node.path,Some(label));ui.close();}}
                if ui.button("清除").clicked(){self.folder_label(&node.path,None);ui.close();}
            });});
            if ui.add_enabled(ready&&!node.volume,egui::Button::new("定位缺失的文件…")).clicked(){
                if let Some(destination)=rfd::FileDialog::new().set_title("选择原照片移动后的文件夹").pick_folder(){self.start_folder_scan(node.path.clone(),Some(destination),ui.ctx());}ui.close();
            }
            let sync=ui.add_enabled(ready&&!node.volume,egui::Button::new("同步文件夹…"));lightcraft_ui_egui::widgets::register(ui.ctx(),"main.folder.sync",sync.rect);
            if sync.clicked(){self.start_folder_scan(node.path.clone(),None,ui.ctx());ui.close();}
            ui.separator();let remove=ui.add_enabled(ready&&!node.volume,egui::Button::new("从图库中移除…"));lightcraft_ui_egui::widgets::register(ui.ctx(),"main.folder.remove",remove.rect);
            if remove.clicked(){self.request_folder_remove(&node.path);ui.close();}
        });
    }
    fn folder_label(&mut self,path:&str,label:Option<ColorLabel>){
        if let Some(lib)=&self.library{let op=lib.catalog.folder_label_op(path,label);self.apply_named(vec![op],"设置文件夹颜色");}
    }
    fn request_folder_remove(&mut self,path:&str){
        if let Some(lib)=&self.library{match library::folder_remove_ops(&lib.catalog,path){Ok(_)=>self.folders.remove=Some((path.into(),library::folder_ids(&lib.catalog,path),lib.catalog.revision)),Err(e)=>self.message=e.to_string()}}
    }
    fn start_folder_scan(&mut self,path:String,destination:Option<PathBuf>,ctx:&egui::Context){
        if self.scanning{return;}let Some(lib)=&self.library else{return};let catalog=lib.catalog.clone();let skip=lib.directory.clone();
        let (tx,rx)=mpsc::channel();let cancel=Arc::new(AtomicBool::new(false));let flag=cancel.clone();let ctx=ctx.clone();
        std::thread::spawn(move||{let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||match destination{Some(to)=>library::locate_folder(&catalog,&path,&to,&flag),None=>library::scan_folder(&catalog,&path,&skip,flag)})).unwrap_or_else(|_|Err(anyhow::anyhow!("文件夹扫描异常，照片库未改变"))).map(FolderOutcome::Preview).map_err(|e|e.to_string());let _=tx.send(result);ctx.request_repaint();});
        self.folders.task=Some(FolderTask{rx,cancel});self.scanning=true;self.message="正在扫描文件夹；尚未修改照片库…".into();
    }
    fn apply_folder_plan(&mut self,ctx:&egui::Context){
        if self.scanning{self.message="请等当前扫描结束后再应用预览。".into();return;}
        let Some(plan)=self.folders.plan.take()else{return};let Some(lib)=&self.library else{return};let catalog=lib.catalog.clone();let choices=self.folders.choices;
        let (tx,rx)=mpsc::channel();let cancel=Arc::new(AtomicBool::new(false));let flag=cancel.clone();let ctx=ctx.clone();
        std::thread::spawn(move||{let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{anyhow::ensure!(!flag.load(Ordering::Relaxed),"操作已取消");let ops=plan.ops(&catalog,choices)?;anyhow::ensure!(!flag.load(Ordering::Relaxed),"操作已取消");Ok::<_,anyhow::Error>(FolderOutcome::Commit{revision:plan.revision,ops})})).unwrap_or_else(|_|Err(anyhow::anyhow!("校验异常，照片库未改变"))).map_err(|e|e.to_string());let _=tx.send(result);ctx.request_repaint();});
        self.folders.task=Some(FolderTask{rx,cancel});self.scanning=true;self.message="正在校验同步预览…".into();
    }
    pub(super) fn poll_folder_task(&mut self,ctx:&egui::Context){
        let result=match self.folders.task.as_ref().map(|t|t.rx.try_recv()){
            Some(Ok(result))=>Some(result),Some(Err(mpsc::TryRecvError::Disconnected))=>Some(Err("文件夹后台线程已停止，照片库未改变".into())),_=>None
        };
        if let Some(result)=result{
            let cancelled=self.folders.task.as_ref().is_some_and(|t|t.cancel.load(Ordering::Relaxed));self.folders.task=None;self.scanning=false;
            if cancelled{self.message="已取消文件夹操作；照片库和原片未改变。".into();return;}
            match result{
                Ok(FolderOutcome::Preview(plan))=>{self.message=format!("发现 {} 张新照片、{} 个缺失记录、{} 个可重新关联记录",plan.new_photos.len(),plan.missing.len(),plan.relinks.len());self.folders.choices=FolderChoices::default();self.folders.plan=Some(plan);},
                Ok(FolderOutcome::Commit{revision,ops})=>{
                    if self.library.as_ref().is_none_or(|l|l.catalog.revision!=revision){self.message="预览期间照片库已改变，请重新扫描；没有应用本次同步。".into();return;}
                    let has_relinks=ops.iter().any(|op|matches!(op,Op::Relink{..}));
                    if ops.is_empty(){self.message="没有需要同步的修改。".into();}
                    else if self.apply_named(ops,"同步文件夹"){
                        self.folders.project_paths_changed|=has_relinks;self.reset_folder_thumbnails();self.refresh_query();
                    }
                },Err(e)=>self.message=e,
            }
        }
        if self.folders.task.is_some(){ctx.request_repaint_after(std::time::Duration::from_millis(50));}
    }
    fn reset_folder_thumbnails(&mut self){self.generation=self.generation.wrapping_add(1);self.textures.clear();self.previews.clear();self.pending.clear();self.preview_pending.clear();self.order.clear();self.preview_order.clear();self.failed.clear();}
    pub(super) fn folder_dialogs(&mut self,ctx:&egui::Context){
        if self.folders.task.is_some(){egui::Window::new("扫描与校验文件夹").collapsible(false).resizable(false).show(ctx,|ui|{if ui.style().animation_time>0.0{ui.spinner();}ui.label(&self.message);if ui.button("取消").clicked(){if let Some(t)=&self.folders.task{t.cancel.store(true,Ordering::Relaxed);}}});}
        if let Some(plan)=&self.folders.plan{
            let path=plan.folder.clone();let (new,missing,moved)=(plan.new_photos.len(),plan.missing.len(),plan.relinks.len());let errors=plan.errors.clone();let locate=plan.destination.is_some();let mut apply=false;let mut cancel=false;
            egui::Window::new(if locate{"定位缺失的文件"}else{"同步文件夹"}).collapsible(false).default_width(430.0).show(ctx,|ui|{
                ui.label(path);ui.label(format!("新路径 {new}（含可关联记录）· 缺失记录 {missing} · 内容匹配 {moved}"));
                if !locate{ui.checkbox(&mut self.folders.choices.import_new,"添加新照片（原位置引用）");}
                ui.checkbox(&mut self.folders.choices.relink_moved,"关联内容一致的原片，保留编辑与评级");
                if !locate{ui.add_enabled(errors.is_empty(),egui::Checkbox::new(&mut self.folders.choices.remove_missing,"将仍缺失的记录移到最近删除"));}
                ui.small("不移动或删除磁盘文件，不回写原 XMP。");
                if !errors.is_empty(){ui.collapsing(format!("{} 项扫描错误",errors.len()),|ui|{for e in errors.iter().take(30){ui.label(e);}});}
                ui.horizontal(|ui|{let r=ui.button("确认应用");lightcraft_ui_egui::widgets::register(ui.ctx(),"main.folder.apply",r.rect);apply=r.clicked();cancel=ui.button("取消").clicked();});
            });
            if cancel{self.folders.plan=None;}else if apply{self.apply_folder_plan(ctx);}
        }
        if let Some((path,ids,revision))=self.folders.remove.clone(){
            let mut apply=false;let mut cancel=false;
            egui::Window::new("从图库中移除文件夹").collapsible(false).show(ctx,|ui|{ui.label(&path);ui.label(format!("将此文件夹及子文件夹的 {} 条记录移到最近删除。原片、XMP 与工程保留，可撤销。",ids.len()));ui.horizontal(|ui|{let r=ui.button("移出照片库");lightcraft_ui_egui::widgets::register(ui.ctx(),"main.folder.confirmRemove",r.rect);apply=r.clicked();cancel=ui.button("取消").clicked();});});
            if cancel{self.folders.remove=None;}else if apply{
                if self.library.as_ref().is_some_and(|l|l.catalog.revision==revision){if self.apply_named(ids.into_iter().map(|id|Op::SetDeleted{id,deleted:true}).collect(),"移出文件夹"){
                    if self.filter.library_folder.as_deref().is_some_and(|p|lightcraft_catalog::query::folder_within(p,&path)){self.filter=Filter::default();}self.refresh_query();
                }}else{self.message="照片库已改变，请重新确认移除范围。".into();}self.folders.remove=None;
            }
        }
        if let Some((parent,name))=&mut self.folders.create{
            let mut create=false;let mut cancel=false;
            egui::Window::new("新建子文件夹").collapsible(false).show(ctx,|ui|{ui.label(parent.display().to_string());ui.text_edit_singleline(name);ui.horizontal(|ui|{create=ui.button("创建").clicked();cancel=ui.button("取消").clicked();});});
            if cancel{self.folders.create=None;}else if create{match library::create_child_folder(parent,name){Ok(path)=>{self.message=format!("已创建 {}；添加照片后会显示在图库文件夹树中。",path.display());self.folders.create=None;},Err(e)=>self.message=e.to_string()}}
        }
    }
}
