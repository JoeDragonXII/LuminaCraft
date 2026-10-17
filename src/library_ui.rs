use crate::chrome;
use eframe::egui::{self, Color32, RichText, Vec2};
use lightcraft_catalog::{Album, AlbumId, ColorLabel, Filter, Flag, FolderNode, Op, PhotoId, Sort, SortKey};
use luminacraft::library::{self, Library, ScanEvent};
use std::{collections::{BTreeSet, HashMap, HashSet, VecDeque}, path::PathBuf, sync::{Arc, mpsc, atomic::{AtomicBool, Ordering}}};
#[path="library_workflow.rs"] mod workflow;
#[path="library_sidebar.rs"] mod sidebar;
#[path="library_toolbar.rs"] mod toolbar;
use workflow::{BrowserView,HistoryEntry,PendingRemoval,AlbumDialog,LibraryAction};

struct ThumbRequest { id: PhotoId, path: String, key: String, cache: PathBuf, edge:usize, generation:u64 }
struct ThumbResult { id: PhotoId, image: Result<image::RgbaImage, String>, edge:usize, generation:u64 }

pub struct LibraryUi {
    folders:sidebar::FolderUi,
    details_open:bool,details_compact:bool,
    generation:u64,
    view:BrowserView, auto_advance:bool, thumb_size:f32, scroll_to:Option<PhotoId>,
    grid_row_height:f32,
    compare_pair:Option<(PhotoId,PhotoId)>,
    previews:HashMap<PhotoId,egui::TextureHandle>, preview_order:VecDeque<PhotoId>, preview_pending:HashSet<PhotoId>,
    undo:Vec<HistoryEntry>,redo:Vec<HistoryEntry>,pending_remove:Option<PendingRemoval>,
    pending_disk:Option<luminacraft::deletion::Plan>,
    album_dialog:Option<AlbumDialog>,delete_album:Option<AlbumId>,
    keyword_for:Option<PhotoId>,keyword_input:String,
    saved_view:Vec<u8>,view_changed:Option<std::time::Instant>,prefs_disabled:bool,
    pub library: Option<Library>,
    pub message: String,
    pub scanning: bool,
    filter: Filter,
    sort: Sort,
    selected: BTreeSet<PhotoId>,
    active: Option<PhotoId>,
    album_name: String,
    album_target: Option<AlbumId>,
    recursive: bool,
    restore_removed:bool,restored:usize,
    errors: Vec<String>,
    added: usize,
    seen: usize,
    scan_rx: Option<mpsc::Receiver<ScanEvent>>,
    cancel: Arc<AtomicBool>,
    thumb_tx: mpsc::SyncSender<ThumbRequest>,
    thumb_rx: mpsc::Receiver<ThumbResult>,
    pending: HashSet<PhotoId>,
    textures: HashMap<PhotoId, egui::TextureHandle>,
    order: VecDeque<PhotoId>,
    failed: HashMap<PhotoId, String>,
    query_revision: u64,
    query_filter: Filter,
    query_sort: Sort,
    visible: Vec<PhotoId>,
    #[cfg(test)]
    card_rects: HashMap<PhotoId, egui::Rect>,
    #[cfg(test)]
    menu_action_rect: Option<egui::Rect>,
    #[cfg(test)] confirm_rect:Option<egui::Rect>,
    #[cfg(test)] album_rects:HashMap<AlbumId,egui::Rect>,
}

impl LibraryUi {
    pub fn selected_ids(&self)->Vec<u64>{self.selected.iter().map(|id|id.0).collect()}
    pub fn editing_records(&self)->Vec<luminacraft::bridge::ReturnedPhoto>{
        self.library.as_ref().map(|lib|self.targets().iter().filter_map(|id|lib.catalog.photo(*id)).filter_map(|p|{
            let path=library::photo_path(p).map(PathBuf::from)?;
            Some(luminacraft::bridge::ReturnedPhoto{id:p.id.0,path,sha256:String::new(),develop:(*p.develop).clone(),origin:None,copy_name:p.copy_name.clone(),cached_denoise:None})
        }).collect()).unwrap_or_default()
    }
    pub fn suspend(&mut self)->anyhow::Result<PathBuf>{
        anyhow::ensure!(!self.scanning,"请等当前扫描完成后打开完整版");
        self.persist_preferences(true);
        if let Some(lib)=&mut self.library{lib.save_file()?;}
        let dir=self.library.as_ref().ok_or_else(||anyhow::anyhow!("照片库未打开"))?.directory.clone();
        self.library.take();Ok(dir)
    }
    pub fn resume(&mut self,dir:&std::path::Path,ids:&[u64]) -> anyhow::Result<()> {
        let lib=Library::open(dir)?;self.replace_library(lib,ids);Ok(())
    }
    pub fn replace_library(&mut self,lib:Library,ids:&[u64]) {
        self.persist_preferences(true);
        self.library=Some(lib);
        self.generation=self.generation.wrapping_add(1);
        self.filter=Filter::default();self.sort=Sort{key:SortKey::FileName,ascending:true,..Default::default()};self.view=BrowserView::Grid;
        self.auto_advance=false;self.thumb_size=190.0;self.recursive=true;self.restore_removed=true;
        self.album_target=None;self.compare_pair=None;self.pending_remove=None;self.pending_disk=None;self.album_dialog=None;self.delete_album=None;self.keyword_for=None;self.folders=Default::default();
        self.saved_view.clear();self.view_changed=None;self.prefs_disabled=false;
        self.load_preferences();
        self.selected=ids.iter().copied().map(PhotoId).filter(|id|self.library.as_ref().is_some_and(|l|l.catalog.photo(*id).is_some_and(|p|!p.deleted))).collect();self.active=self.selected.iter().next().copied();self.scroll_to=self.active;
        self.order.clear();self.preview_order.clear();
        self.textures.clear();self.previews.clear();self.failed.clear();self.pending.clear();self.preview_pending.clear();self.undo.clear();self.redo.clear();self.query_revision=u64::MAX;
        if !self.prefs_disabled{self.message="照片库已打开。".into();}
    }
    pub fn new(ctx: &egui::Context) -> Self {
        let opened = library::default_directory().and_then(|dir| {
            anyhow::ensure!(!luminacraft::bridge::live_handoff(&dir)?,"另一个窗口正在交接照片库，请等它返回后重试。");Library::open(&dir)
        });
        Self::from_opened(ctx, opened)
    }
    pub fn unopened(ctx:&egui::Context)->Self {Self::from_opened(ctx,Err(anyhow::anyhow!("正在打开指定目录")))}

    fn from_opened(ctx: &egui::Context, opened: anyhow::Result<Library>) -> Self {
        let (library, message) = match opened {
            Ok(lib) => { let msg = lib.warning.clone().unwrap_or_else(|| "添加照片文件夹，开始浏览和选片。".into()); (Some(lib), msg) },
            Err(e) => (None, format!("照片库未打开：{e:#}")),
        };
        let (thumb_tx, requests) = mpsc::sync_channel::<ThumbRequest>(16);
        let (results, thumb_rx) = mpsc::channel();
        let context = ctx.clone();
        std::thread::spawn(move || {
            while let Ok(req) = requests.recv() {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<image::RgbaImage> {
                    let cached = req.cache.join(format!("{}.png", req.key));
                    if let Ok(reader) = image::ImageReader::open(&cached) {
                        if let Ok(image) = reader.decode() { return Ok(image.to_rgba8()); }
                    }
                    let pixels = library::browsing_preview(&req.path,req.edge)?;
                    // Cache failure never prevents browsing; all writes stay inside the library.
                    if std::fs::create_dir_all(&req.cache).is_ok() {
                        let temp = cached.with_extension("tmp");
                        if pixels.save_with_format(&temp, image::ImageFormat::Png).is_ok() {
                            let _ = std::fs::rename(&temp, &cached);
                        }
                    }
                    Ok(pixels)
                })).map_err(|_| "缩略图解析异常，原照片保持原样。".to_string()).and_then(|r|r.map_err(|e| format!("{e:#}")));
                if results.send(ThumbResult { id: req.id, image: result,edge:req.edge,generation:req.generation }).is_err() { break; }
                context.request_repaint();
            }
        });
        let mut browser=Self {folders:Default::default(),details_open:true,details_compact:false,generation:0,view:BrowserView::Grid,auto_advance:false,thumb_size:190.0,scroll_to:None,grid_row_height:190.0,compare_pair:None,
            previews:HashMap::new(),preview_order:VecDeque::new(),preview_pending:HashSet::new(),
            undo:vec![],redo:vec![],pending_remove:None,pending_disk:None,album_dialog:None,delete_album:None,keyword_for:None,keyword_input:String::new(),saved_view:vec![],view_changed:None,prefs_disabled:false,
            library, message, scanning: false, filter: Filter::default(), sort: Sort {key:SortKey::FileName, ascending:true, ..Default::default()},
            selected:BTreeSet::new(), active:None, album_name:String::new(), album_target:None, recursive:true,restore_removed:true,restored:0,
            errors:vec![], added:0, seen:0, scan_rx:None, cancel:Arc::new(AtomicBool::new(false)), thumb_tx,thumb_rx,
            pending:HashSet::new(),textures:HashMap::new(),order:VecDeque::new(),failed:HashMap::new(),
            query_revision:u64::MAX,query_filter:Filter::default(),query_sort:Sort::default(),visible:vec![],
            #[cfg(test)] card_rects:HashMap::new(),
            #[cfg(test)] menu_action_rect:None,
            #[cfg(test)] confirm_rect:None,
            #[cfg(test)] album_rects:HashMap::new() };
        browser.load_preferences();browser
    }

    pub fn scan(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if self.scanning { return; }
        let Some(lib) = &self.library else { return; };
        let skip = lib.directory.clone();
        let paths = if self.recursive { paths } else {
            let mut files = Vec::new();
            for path in paths {
                if path.is_dir() {
                    match std::fs::read_dir(&path) {
                        Ok(entries) => files.extend(entries.filter_map(Result::ok).map(|e|e.path()).filter(|p|p.is_file())),
                        Err(e) => { self.message=format!("无法读取文件夹：{e}"); return; }
                    }
                } else { files.push(path); }
            }
            files
        };
        let (tx, rx) = mpsc::sync_channel(64);
        self.cancel=Arc::new(AtomicBool::new(false));
        let cancel=self.cancel.clone();
        let context=ctx.clone();
        self.scan_rx=Some(rx); self.scanning=true; self.errors.clear();self.added=0;self.restored=0;self.seen=0;
        self.message="扫描照片文件夹…".into();
        std::thread::spawn(move || library::scan(paths,skip,cancel,|event| {
            let result=tx.send(event).is_ok();context.request_repaint();result
        }));
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        self.poll_folder_task(ctx);
        self.persist_preferences(false);
        if self.view_changed.is_some(){ctx.request_repaint_after(std::time::Duration::from_millis(750));}
        let mut photos=Vec::new(); let mut done=None;
        if let Some(rx)=&self.scan_rx {
            for _ in 0..32 {
                match rx.try_recv() {
                    Ok(ScanEvent::Photo(p))=>{photos.push(*p);self.seen+=1;},
                    Ok(ScanEvent::Error(e))=>self.errors.push(e),
                    Ok(ScanEvent::Done{cancelled})=>{done=Some(cancelled);break;},
                    Err(mpsc::TryRecvError::Disconnected)=>{self.errors.push("扫描线程已停止。".into());done=Some(true);break;},
                    Err(mpsc::TryRecvError::Empty)=>break,
                }
            }
        }
        if !photos.is_empty() {
            if let Some(lib)=&mut self.library {
                match lib.add_with_restore(photos,self.restore_removed) {
                    Ok((count,restored))=>{self.added+=count;self.restored+=restored;},
                    Err(e)=>{self.errors.push(format!("照片库保存失败：{e:#}"));self.cancel.store(true,Ordering::Relaxed);},
                }
            }
        }
        if let Some(cancelled)=done {
            self.scanning=false;self.scan_rx=None;
            self.message=format!("{}新增 {} · 恢复 {} · 跳过 {} · {} 项错误",if cancelled{"已停止 · "}else{""},self.added,self.restored,self.seen.saturating_sub(self.added+self.restored),self.errors.len());
        } else if self.scanning && self.folders.task.is_none() {
            self.message=format!("正在扫描 · 新增 {} 张 · {} 项错误",self.added,self.errors.len());
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        while let Ok(result)=self.thumb_rx.try_recv() {
            if result.generation!=self.generation{continue;}
            if result.edge>320{
                self.preview_pending.remove(&result.id);
                match result.image{
                    Ok(image)=>{
                        while self.previews.len()>=12{if let Some(id)=self.preview_order.pop_front(){self.previews.remove(&id);}else{break;}}
                        let pixels=egui::ColorImage::from_rgba_unmultiplied([image.width() as usize,image.height() as usize],image.as_raw());
                        self.previews.insert(result.id,ctx.load_texture(format!("browse-preview-{}",result.id.0),pixels,egui::TextureOptions::LINEAR));self.preview_order.push_back(result.id);
                    },
                    Err(e)=>{self.failed.insert(result.id,e);},
                }
                continue;
            }
            self.pending.remove(&result.id);
            match result.image {
                Ok(image)=>{
                    while self.textures.len()>=256 {
                        if let Some(id)=self.order.pop_front(){self.textures.remove(&id);}else{break;}
                    }
                    let pixels=egui::ColorImage::from_rgba_unmultiplied([image.width() as usize,image.height() as usize],image.as_raw());
                    self.textures.insert(result.id,ctx.load_texture(format!("library-{}",result.id.0),pixels,egui::TextureOptions::LINEAR));
                    self.order.push_back(result.id);
                },
                Err(e)=>{self.failed.insert(result.id,e);},
            }
        }
    }

    pub fn screenshot_ready(&self) -> bool { !self.scanning && self.pending.is_empty()&&self.preview_pending.is_empty() }

    fn apply(&mut self, ops: Vec<Op>) {
        self.apply_named(ops,"照片库操作");
    }

    pub(crate) fn selected_paths(&self) -> Vec<PathBuf> {
        self.library.as_ref().map(|lib|self.targets().iter().filter_map(|id|lib.catalog.photo(*id))
            .filter_map(|p|library::photo_path(p).map(PathBuf::from)).collect()).unwrap_or_default()
    }

    pub(crate) fn edit_focus_id(&self)->Option<u64>{
        let ids=self.targets();self.active.filter(|id|ids.contains(id)).or_else(||ids.first().copied()).map(|id|id.0)
    }
    pub(crate) fn selection_identity(&self)->Option<(PathBuf,Vec<u64>,Option<u64>)>{
        self.library.as_ref().map(|lib|(lib.directory.clone(),self.selected_ids(),self.edit_focus_id()))
    }

    fn request_thumb(&mut self, id: PhotoId) {
        self.request_image(id,320);
    }
    fn request_image(&mut self,id:PhotoId,edge:usize){
        if self.failed.contains_key(&id){return;}
        if edge>320{if self.previews.contains_key(&id)||self.preview_pending.contains(&id){return;}}
        else if self.textures.contains_key(&id)||self.pending.contains(&id){return;}
        let Some(lib)=&self.library else{return;};
        let Some(photo)=lib.catalog.photo(id) else{return;};
        let Some(path)=library::photo_path(photo) else{return;};
        use sha2::Digest;
        let fingerprint=std::fs::metadata(path).ok().map(|m|format!("{}:{:?}",m.len(),m.modified().ok())).unwrap_or_default();
        let key=format!("{:x}",sha2::Sha256::digest(format!("library-v2:{edge}:{path}:{fingerprint}")));
        if self.thumb_tx.try_send(ThumbRequest{id,path:path.into(),key,cache:lib.directory.join("thumbs"),edge,generation:self.generation}).is_ok(){if edge>320{self.preview_pending.insert(id);}else{self.pending.insert(id);}}
    }

    pub fn show(&mut self, ui: &mut egui::Ui, editor_busy: bool) -> Option<Vec<PathBuf>> {
        let mut open=None;
        let compact=ui.available_width()<1160.0;
        if compact!=self.details_compact{self.details_compact=compact;self.details_open=!compact;}
        self.refresh_query();
        if self.view!=BrowserView::Grid{self.filmstrip(ui,editor_busy,&mut open);}
        egui::Panel::left("catalog-folders").default_size(215.0).size_range(196.0..=320.0).show_separator_line(false).frame(chrome::surface_frame(12)).show(ui,|ui|{
            egui::ScrollArea::vertical().id_salt("catalog-sidebar-all").show(ui,|ui|{
            ui.spacing_mut().item_spacing.y=4.0;
            ui.add_space(4.0);ui.label(RichText::new("照片库").font(crate::typography::semibold(14.0)));ui.add_space(10.0);
            ui.add_enabled_ui(self.library.is_some()&&!self.scanning,|ui|{
                let add=chrome::text_button(ui,"luminacraft.library.add","添加照片…",ui.available_width(),false,ui.is_enabled());
                egui::Popup::menu(&add).show(|ui|{
                    if ui.button("添加文件夹…").clicked(){ui.close();if let Some(dir)=rfd::FileDialog::new().pick_folder(){self.scan(vec![dir],ui.ctx());}}
                    if ui.button("选择照片…").clicked(){ui.close();if let Some(paths)=rfd::FileDialog::new().add_filter("照片",&["arw","jpg","jpeg","png","tif","tiff","dng","nef","cr2","cr3","raf","rw2","orf","pef"]).pick_files(){self.scan(paths,ui.ctx());}}
                    ui.separator();ui.checkbox(&mut self.recursive,"包含子文件夹");ui.checkbox(&mut self.restore_removed,"恢复已移出的照片");
                });
            });
            ui.add_space(16.0);
            self.basic_sources_ui(ui);
            let albums:Vec<_>=self.library.as_ref().map(|l|l.catalog.albums().map(|a|(a.clone(),l.catalog.album_count(a.id))).collect()).unwrap_or_default();
            ui.scope(|ui|{
                let quick=albums.iter().find(|(a,_)|a.quick);
                if ui.selectable_label(quick.is_some_and(|(a,_)|self.filter.album==Some(a.id)),format!("快捷收藏  {}",quick.map(|(_,n)|*n).unwrap_or(0))).clicked(){
                    let id=quick.map(|(a,_)|a.id).or_else(||{
                        let mut allocation=self.library.as_ref()?.catalog.clone();let id=allocation.alloc_album_id();let mut album=Album::new(id,"快捷收藏");album.quick=true;
                        self.apply_named(vec![Op::AddAlbum{album}],"创建快捷收藏").then_some(id)
                    });
                    if let Some(id)=id{self.select_album_source(id);}
                }
                self.folder_sections_ui(ui,editor_busy);
                ui.add_space(16.0);ui.label(RichText::new("相册").color(crate::chrome::MUTED));
                for (album,count) in albums.iter().filter(|(a,_)|!a.quick){
                    let title=format!("{}{}{}  {}",if album.is_smart(){"◇ "}else{""},album.name,if self.album_target==Some(album.id){" +"}else{""},count);
                    let response=chrome::side_tab(ui,&format!("main.album.{}",album.id.0),&title,self.filter.album==Some(album.id));
                    #[cfg(test)] {self.album_rects.insert(album.id,response.rect);}
                    if response.clicked(){self.select_album_source(album.id);}
                    if !album.is_smart()&&!album.folder{
                        if response.dnd_hover_payload::<Vec<PhotoId>>().is_some(){ui.painter().rect_stroke(response.rect,6,egui::Stroke::new(2.0,crate::chrome::ACCENT),egui::StrokeKind::Inside);}
                        if let Some(ids)=response.dnd_release_payload::<Vec<PhotoId>>(){
                            let mut photos=album.photos.clone();for id in ids.iter(){if !photos.contains(id){photos.push(*id);}}
                            self.apply_named(vec![Op::SetAlbumPhotos{id:album.id,photos}],"加入相册");
                        }
                    }
                    response.context_menu(|ui|{
                        if ui.button("重命名相册…").clicked(){self.album_dialog=Some(AlbumDialog{id:Some(album.id),name:album.name.clone(),photos:vec![],include:false,smart:album.is_smart()});ui.close();}
                        if !album.is_smart()&&!album.folder&&ui.button("设为目标相册").clicked(){self.album_target=Some(album.id);ui.close();}
                        if album.is_smart()&&ui.button("用当前筛选更新规则").clicked(){let mut rules=self.filter.clone();rules.album=None;rules.deleted=false;self.apply_named(vec![Op::SetAlbumRules{id:album.id,rules:Box::new(rules)}],"更新智能相册");ui.close();}
                        ui.add_space(16.0);if ui.button("删除相册…").clicked(){self.delete_album=Some(album.id);ui.close();}
                    });
                }
            });
            ui.add_space(16.0);ui.add(egui::TextEdit::singleline(&mut self.album_name).hint_text("新相册名称").desired_width(170.0));
            if ui.add_enabled(self.library.is_some()&&!self.album_name.trim().is_empty(),egui::Button::new("创建相册")).clicked(){
                if let Some(lib)=&mut self.library{
                    let id=lib.catalog.alloc_album_id();let album=Album::new(id,self.album_name.trim());
                    self.apply(vec![Op::AddAlbum{album}]);
                    if self.library.as_ref().is_some_and(|l|l.catalog.album(id).is_some()){self.album_target=Some(id);self.album_name.clear();}
                }
            }
            if ui.add_enabled(self.library.is_some(),egui::Button::new("从筛选创建智能相册…")).clicked(){self.create_album_dialog(true);}
            });
        });

        egui::Panel::right("catalog-details").exact_size(if self.details_open{264.0}else{64.0}).resizable(false).show_separator_line(false).frame(chrome::surface_frame(if self.details_open{12}else{8})).show(ui,|ui|{
            if !self.details_open{
                let r=chrome::icon_button(ui,"main.library.details",lightcraft_ui_egui::icons::Icon::Sidebar,"展开选片栏",false,true);
                if r.clicked(){self.details_open=true;}return;
            }
            chrome::centered_row(ui,34.0,|ui|{
                ui.label(RichText::new("选片").size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                    if chrome::icon_button(ui,"main.library.details",lightcraft_ui_egui::icons::Icon::Sidebar,"收起选片栏",false,true).clicked(){self.details_open=false;}
                });
            });

            egui::ScrollArea::vertical().id_salt("catalog-inspector").show(ui,|ui|{
            ui.label(format!("已选 {} 张",self.selected.len()));
            if self.view!=BrowserView::Grid{ui.small("评级与标记作用于活动照片");}
            let edit=chrome::text_button(ui,"luminacraft.library.edit","进入修图",ui.available_width(),true,!editor_busy&&!self.selected.is_empty());
            if edit.clicked(){open=Some(self.selected_paths());}
            ui.add_space(16.0);
            let ids=self.targets();
            ui.add_enabled_ui(!ids.is_empty(),|ui|{
                self.selection_controls(ui,&ids);
                ui.add_space(16.0);
                egui::ComboBox::from_id_salt("album-target").selected_text(self.album_target.and_then(|id|self.library.as_ref()?.catalog.album(id).map(|a|a.name.as_str())).unwrap_or("选择相册")).show_ui(ui,|ui|{if let Some(lib)=&self.library{for album in lib.catalog.albums().filter(|a|!a.folder&&!a.is_smart()){ui.selectable_value(&mut self.album_target,Some(album.id),&album.name);}}});
                if ui.add_enabled(self.album_target.is_some()&&!self.filter.deleted,egui::Button::new("加入相册")).clicked(){if let Some(id)=self.album_target{self.action(LibraryAction::AddToAlbum(id),false);}}
                let can_remove=self.filter.album.and_then(|id|self.library.as_ref()?.catalog.album(id)).is_some_and(|a|!a.is_smart()&&!a.folder);
                if ui.add_enabled(can_remove,egui::Button::new("从当前相册移除…")).clicked(){self.request_remove(false);}
                ui.add_space(16.0);
                if ui.button(if self.filter.deleted{"恢复到照片库"}else{"移出照片库"}).on_hover_text("仅改变照片库记录，原照片和 XMP 保留在原位置。").clicked(){
                    self.request_remove(true);
                }
            });
            if let Some(photo)=self.active.and_then(|id|self.library.as_ref()?.catalog.photo(id).cloned()){
                ui.add_space(16.0);ui.label(&photo.file_name);ui.small(format!("{} × {} · {} · {:.1} MB",photo.width,photo.height,photo.format,photo.file_size as f64/1_000_000.0));
                ui.small(format!("{}★ · {} · {}",photo.rating,flag_name(Some(photo.flag)),label_name(photo.label)));
                if !photo.meta.camera.is_empty(){ui.small(&photo.meta.camera);}
                if let Some(date)=&photo.captured{ui.small(date);}
                ui.add_space(16.0);ui.label("关键词");
                if self.keyword_for!=Some(photo.id){self.keyword_for=Some(photo.id);self.keyword_input=photo.meta.keywords.join(", ");}
                ui.add(egui::TextEdit::singleline(&mut self.keyword_input).hint_text("逗号分隔").desired_width(200.0).char_limit(4096));
                ui.horizontal(|ui|{
                    if ui.button("添加").clicked(){self.set_keywords(false);}
                    if ui.button("替换").clicked(){self.set_keywords(true);}
                });
                if let Some(path)=library::photo_path(&photo){ui.add(egui::Label::new(path).wrap());if !std::path::Path::new(path).is_file(){ui.colored_label(crate::chrome::ERROR,"原照片离线或已移动");}}
                if photo.kind==lightcraft_catalog::MediaKind::Raw{ui.small("浏览可使用相机内嵌预览；编辑使用传感器显影。");}
                if let Some(error)=self.failed.get(&photo.id){ui.colored_label(crate::chrome::ERROR,error);}
            }
            });
        });

        egui::CentralPanel::default().frame(chrome::surface_frame(16)).show(ui,|ui|{
            let content_width=ui.available_width();
            self.toolbar(ui);
            let total=self.library.as_ref().map(|l|l.catalog.photos().filter(|p|p.deleted==self.filter.deleted).count()).unwrap_or(0);
            if self.scanning || !self.errors.is_empty(){ui.horizontal(|ui|{if self.scanning{if ui.style().animation_time>0.0{ui.spinner();}if ui.button("停止扫描").clicked(){self.cancel.store(true,Ordering::Relaxed);self.cancel_folder_task();}}ui.add(egui::Label::new(&self.message).truncate());});}
            if !self.errors.is_empty(){egui::CollapsingHeader::new(format!("{} 项扫描错误",self.errors.len())).show(ui,|ui|{egui::ScrollArea::vertical().max_height(120.0).show(ui,|ui|{for e in &self.errors{ui.label(e);}});});}
            ui.add_space(16.0);
            if self.visible.is_empty(){ui.vertical_centered(|ui|{ui.add_space(100.0);ui.heading(if total==0{"添加你的照片文件夹"}else{"没有符合筛选的照片"});ui.label(if total==0{"原照片保留在原位置，选片信息自动保存。"}else{"调整搜索或筛选条件。"});});return;}
            ui.set_max_width(content_width);
            let columns=(content_width/self.thumb_size).floor().max(1.0) as usize;
            self.keyboard(ui,columns,editor_busy,&mut open);
            if self.view!=BrowserView::Grid{self.browse_views(ui,editor_busy,&mut open);return;}
            let width=(content_width-(columns.saturating_sub(1) as f32)*10.0)/columns as f32;
            let image_height=((width-16.0)*0.66).clamp(90.0,190.0);
            let rows=self.visible.len().div_ceil(columns);
            let mut scroll=egui::ScrollArea::vertical().id_salt("catalog-grid");
            if let Some(id)=self.scroll_to.take(){if let Some(index)=self.visible.iter().position(|p|*p==id){scroll=scroll.vertical_scroll_offset((index/columns) as f32*(self.grid_row_height+ui.spacing().item_spacing.y));}}
            let mut measured=None;
            scroll.show_rows(ui,self.grid_row_height,rows,|ui,range|{
                for row in range{ui.horizontal(|ui|{
                    for col in 0..columns{
                        let Some(id)=self.visible.get(row*columns+col).copied() else{break;};
                        self.request_thumb(id);
                        let Some(photo)=self.library.as_ref().and_then(|l|l.catalog.photo(id)).cloned() else{continue;};
                        let selected=self.selected.contains(&id);
                        let fill=if selected{crate::chrome::SELECTED}else if let Some(label)=photo.label{label_colors().into_iter().find(|(l,_)|*l==label).map(|(_,c)|Color32::from(egui::lerp(egui::Rgba::from(chrome::PANEL)..=egui::Rgba::from(c),0.18))).unwrap_or(chrome::SURFACE)}else{chrome::SURFACE};
                        let card=egui::Frame::new().fill(fill).corner_radius(chrome::CARD_RADIUS).inner_margin(8).show(ui,|ui|{ui.vertical(|ui|{
                            ui.set_width((width-16.0).max(80.0));
                            let (rect,response)=ui.allocate_exact_size(Vec2::new((width-16.0).max(80.0),image_height),egui::Sense::click());
                            if let Some(texture)=self.textures.get(&id){
                                let size=texture.size_vec2();let scale=(rect.width()/size.x).min(rect.height()/size.y);
                                let image_rect=egui::Rect::from_center_size(rect.center(),size*scale);
                                ui.painter().image(texture.id(),image_rect,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),Color32::WHITE);
                            }else{ui.painter().text(rect.center(),egui::Align2::CENTER_CENTER,if self.failed.contains_key(&id){"无法预览"}else{"加载中…"},egui::FontId::proportional(13.0),crate::chrome::MUTED);}
                            if photo.flag==Flag::Reject{ui.painter().rect_filled(rect,0,Color32::from_black_alpha(120));}
                            ui.add(egui::Label::new(&photo.file_name).truncate());
                            ui.horizontal(|ui|{ui.small(format!("{}★  {}",photo.rating,flag_name(Some(photo.flag))));if let Some(label)=photo.label{if let Some((_,color))=label_colors().into_iter().find(|(l,_)|*l==label){ui.colored_label(color,"●");}}});
                            response
                        }).inner});
                        measured=Some(card.response.rect.height());
                        if self.active==Some(id){ui.painter().rect_stroke(card.response.rect,chrome::CARD_RADIUS,egui::Stroke::new(2.0,crate::chrome::ACCENT),egui::StrokeKind::Inside);}
                        let response=ui.interact(card.response.rect,ui.id().with(("photo-card",id.0)),egui::Sense::click_and_drag());
                        lightcraft_ui_egui::widgets::register(ui.ctx(),format!("luminacraft.library.photo.{}",id.0),card.response.rect);
                        if response.drag_started(){if !self.selected.contains(&id){self.select_photo(id,Default::default());}}
                        response.dnd_set_drag_payload(self.targets());
                        #[cfg(test)] self.card_rects.insert(id, card.response.rect);
                        if response.clicked(){
                            self.select_photo(id,ui.input(|i|i.modifiers));
                        }
                        if response.double_clicked(){
                            self.select_photo(id,Default::default());self.set_view(BrowserView::Loupe);
                        }
                        if response.secondary_clicked(){
                            if !self.selected.contains(&id){self.selected.clear();self.selected.insert(id);}
                            self.active=Some(id);
                        }
                        response.context_menu(|ui|self.context_menu(ui,id,editor_busy,&mut open));
                    }
                });}
            });
            if let Some(height)=measured{if (height-self.grid_row_height).abs()>0.5{self.grid_row_height=height;ui.ctx().request_repaint();}}
        });
        self.dialogs(ui.ctx());
        self.folder_dialogs(ui.ctx());
        open
    }

    fn refresh_query(&mut self){
        if let Some(lib)=&self.library{
            if self.query_revision!=lib.catalog.revision||self.query_filter!=self.filter||self.query_sort!=self.sort{
                self.visible=lib.catalog.query(&self.filter,&self.sort);self.query_revision=lib.catalog.revision;
                self.query_filter=self.filter.clone();self.query_sort=self.sort;
                let visible:HashSet<_>=self.visible.iter().copied().collect();
                self.selected.retain(|id|visible.contains(id));
                if self.active.is_some_and(|id|!visible.contains(&id)||!self.selected.contains(&id)){self.active=self.selected.iter().next().copied();}
                if let Some((a,b))=self.compare_pair{if !visible.contains(&a)||!visible.contains(&b){self.compare_pair=None;}}
            }
        }
    }
}

impl Drop for LibraryUi { fn drop(&mut self){self.cancel.store(true,Ordering::Relaxed);self.persist_preferences(true);} }
fn flag_name(flag:Option<Flag>)->&'static str{match flag{None=>"全部标记",Some(Flag::None)=>"未标记",Some(Flag::Pick)=>"入选",Some(Flag::Reject)=>"排除"}}
fn label_name(label:Option<ColorLabel>)->&'static str{match label{None=>"全部颜色",Some(ColorLabel::Red)=>"红色",Some(ColorLabel::Yellow)=>"黄色",Some(ColorLabel::Green)=>"绿色",Some(ColorLabel::Blue)=>"蓝色",Some(ColorLabel::Purple)=>"紫色"}}
fn label_colors()->[(ColorLabel,Color32);5]{[(ColorLabel::Red,Color32::from_rgb(180,80,80)),(ColorLabel::Yellow,Color32::from_rgb(180,155,70)),(ColorLabel::Green,Color32::from_rgb(80,145,105)),(ColorLabel::Blue,Color32::from_rgb(80,125,175)),(ColorLabel::Purple,Color32::from_rgb(145,100,175))]}
fn sort_name(key:SortKey)->&'static str{match key{SortKey::FileName=>"文件名",SortKey::CaptureDate=>"拍摄时间",SortKey::ImportDate=>"入库时间",SortKey::Rating=>"星级",SortKey::FileSize=>"文件大小",_=>"排序"}}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(browser:&mut LibraryUi,ctx:&egui::Context,mut events:Vec<egui::Event>)->Option<Vec<PathBuf>>{
        let mut open=None;
        let modifiers=events.iter().rev().find_map(|e|match e{egui::Event::Key{modifiers,..}|egui::Event::PointerButton{modifiers,..}=>Some(*modifiers),_=>None}).unwrap_or_default();
        events.insert(0,egui::Event::ModifiersChanged(modifiers));
        let output=ctx.run_ui(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,Vec2::new(1380.0,900.0))),events,..Default::default()},|ui|{
            open=browser.show(ui,false);
        });output.drop_without_applying_deltas();open
    }
    #[test]
    fn grid_click_keyboard_rating_and_enter_edit(){
        let tmp=tempfile::tempdir().unwrap();let ctx=egui::Context::default();crate::typography::install(&ctx);
        let mut browser=LibraryUi::from_opened(&ctx,Library::open(&tmp.path().join("catalog")));
        let mut photos=Vec::new();for n in 0..3{
            let path=tmp.path().join(format!("{n}.png"));image::RgbaImage::from_pixel(24,16,image::Rgba([30,80,120,255])).save(&path).unwrap();
            photos.push(library::index_photo(&path).unwrap());
        }
        browser.library.as_mut().unwrap().add(photos).unwrap();
        frame(&mut browser,&ctx,vec![]);frame(&mut browser,&ctx,vec![]);
        let id=browser.visible[1];let pos=browser.card_rects[&id].center();
        frame(&mut browser,&ctx,vec![egui::Event::PointerMoved(pos)]);
        frame(&mut browser,&ctx,vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()}]);
        frame(&mut browser,&ctx,vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers:Default::default()}]);
        assert_eq!(browser.selected,BTreeSet::from([id]));
        let key=|key|egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers:Default::default()};
        frame(&mut browser,&ctx,vec![key(egui::Key::Num4)]);
        assert_eq!(browser.library.as_ref().unwrap().catalog.photo(id).unwrap().rating,4);
        assert!(frame(&mut browser,&ctx,vec![key(egui::Key::Enter)]).is_none());assert_eq!(browser.view,BrowserView::Loupe);
        let open=frame(&mut browser,&ctx,vec![key(egui::Key::D)]).unwrap();assert_eq!(open.len(),1);
        assert_eq!(open[0].file_name().unwrap(),"1.png");
    }
    fn click(browser:&mut LibraryUi,ctx:&egui::Context,pos:egui::Pos2,button:egui::PointerButton){
        frame(browser,ctx,vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true,false]{frame(browser,ctx,vec![egui::Event::PointerButton{pos,button,pressed,modifiers:Default::default()}]);}
    }
    #[test]
    fn right_click_removes_selection_and_restores_without_deleting_files(){
        let tmp=tempfile::tempdir().unwrap();let ctx=egui::Context::default();crate::typography::install(&ctx);
        let mut browser=LibraryUi::from_opened(&ctx,Library::open(&tmp.path().join("catalog")));
        let mut paths=Vec::new();let mut photos=Vec::new();
        for n in 0..3{
            let path=tmp.path().join(format!("{n}.png"));image::RgbaImage::from_pixel(24,16,image::Rgba([30,80,120,255])).save(&path).unwrap();
            photos.push(library::index_photo(&path).unwrap());paths.push(path);
        }
        browser.library.as_mut().unwrap().add(photos).unwrap();frame(&mut browser,&ctx,vec![]);frame(&mut browser,&ctx,vec![]);
        let ids=browser.visible.clone();browser.selected=BTreeSet::from([ids[0],ids[1]]);
        let pos=browser.card_rects[&ids[1]].center();click(&mut browser,&ctx,pos,egui::PointerButton::Secondary);
        assert_eq!(browser.selected.len(),2);
        frame(&mut browser,&ctx,vec![]);
        let action=browser.menu_action_rect.expect("right-click menu must expose remove").center();click(&mut browser,&ctx,action,egui::PointerButton::Primary);
        assert!(browser.pending_remove.is_some());assert!(!browser.library.as_ref().unwrap().catalog.photo(ids[0]).unwrap().deleted);
        frame(&mut browser,&ctx,vec![]);
        let confirm=browser.confirm_rect.expect("catalog removal must ask to confirm its frozen selection").center();click(&mut browser,&ctx,confirm,egui::PointerButton::Primary);
        let catalog=&browser.library.as_ref().unwrap().catalog;
        assert!(catalog.photo(ids[0]).unwrap().deleted);assert!(catalog.photo(ids[1]).unwrap().deleted);assert!(!catalog.photo(ids[2]).unwrap().deleted);
        assert!(paths.iter().all(|p|p.is_file()));
        browser.filter=Filter{deleted:true,..Default::default()};frame(&mut browser,&ctx,vec![]);frame(&mut browser,&ctx,vec![]);
        browser.selected=BTreeSet::from([ids[1]]);
        let pos=browser.card_rects[&ids[0]].center();click(&mut browser,&ctx,pos,egui::PointerButton::Secondary);
        assert_eq!(browser.selected,BTreeSet::from([ids[0]]));
        frame(&mut browser,&ctx,vec![]);
        let action=browser.menu_action_rect.expect("removed photo must expose restore").center();click(&mut browser,&ctx,action,egui::PointerButton::Primary);
        assert!(!browser.library.as_ref().unwrap().catalog.photo(ids[0]).unwrap().deleted);
        assert!(paths.iter().all(|p|p.is_file()));
    }
    fn setup(count:usize)->(tempfile::TempDir,egui::Context,LibraryUi,Vec<PhotoId>){
        let tmp=tempfile::tempdir().unwrap();let ctx=egui::Context::default();crate::typography::install(&ctx);
        let mut browser=LibraryUi::from_opened(&ctx,Library::open(&tmp.path().join("catalog")));
        let photos=(0..count).map(|n|{let path=tmp.path().join(format!("{n}.png"));image::RgbaImage::from_pixel(24,16,image::Rgba([30,80,120,255])).save(&path).unwrap();library::index_photo(&path).unwrap()}).collect();
        browser.library.as_mut().unwrap().add(photos).unwrap();browser.refresh_query();let ids=browser.visible.clone();
        frame(&mut browser,&ctx,vec![]);frame(&mut browser,&ctx,vec![]);(tmp,ctx,browser,ids)
    }
    #[test]
    fn resume_another_library_resets_filters_and_discards_late_old_thumbnails(){
        let (tmp,ctx,mut browser,ids)=setup(1);
        let old_generation=browser.generation;
        browser.filter.text="no-match-in-next-library".into();browser.view=BrowserView::Loupe;
        browser.suspend().unwrap();
        let b=tmp.path().join("other-catalog");let mut library=Library::open(&b).unwrap();
        library.add(vec![library::index_photo(&tmp.path().join("0.png")).unwrap()]).unwrap();drop(library);
        browser.resume(&b,&[ids[0].0]).unwrap();browser.refresh_query();
        assert_eq!(browser.visible.len(),1);assert_eq!(browser.view,BrowserView::Grid);
        let (tx,rx)=mpsc::channel();browser.thumb_rx=rx;browser.pending.insert(ids[0]);
        tx.send(ThumbResult{id:ids[0],image:Ok(image::RgbaImage::from_pixel(2,2,image::Rgba([255,0,0,255]))),edge:320,generation:old_generation}).unwrap();
        browser.poll(&ctx);assert!(browser.textures.is_empty());assert!(browser.pending.contains(&ids[0]));
        tx.send(ThumbResult{id:ids[0],image:Ok(image::RgbaImage::from_pixel(2,2,image::Rgba([0,255,0,255]))),edge:320,generation:browser.generation}).unwrap();
        browser.poll(&ctx);assert!(browser.textures.contains_key(&ids[0]));assert!(!browser.pending.contains(&ids[0]));
    }
    #[test]
    fn shift_range_ctrl_toggle_and_active_scope_match_lightcraft(){
        let (_tmp,_ctx,mut browser,ids)=setup(5);
        browser.select_photo(ids[0],Default::default());browser.select_photo(ids[3],egui::Modifiers{shift:true,..Default::default()});
        assert_eq!(browser.selected,ids[..4].iter().copied().collect());assert_eq!(browser.active,Some(ids[3]));
        browser.select_photo(ids[1],egui::Modifiers{command:true,..Default::default()});assert!(!browser.selected.contains(&ids[1]));
        browser.action(LibraryAction::Rate(3),false);
        assert_eq!(browser.library.as_ref().unwrap().catalog.photo(ids[1]).unwrap().rating,0);
        assert_eq!(browser.library.as_ref().unwrap().catalog.photo(ids[0]).unwrap().rating,3);
        browser.set_view(BrowserView::Loupe);let active=browser.active.unwrap();browser.action(LibraryAction::Rate(5),false);
        assert_eq!(browser.library.as_ref().unwrap().catalog.photo(active).unwrap().rating,5);
        assert_eq!(browser.library.as_ref().unwrap().catalog.photos().filter(|p|p.rating==5).count(),1);
        browser.undo_catalog(false);assert_eq!(browser.library.as_ref().unwrap().catalog.photo(active).unwrap().rating,3);
        browser.undo_catalog(true);assert_eq!(browser.library.as_ref().unwrap().catalog.photo(active).unwrap().rating,5);
    }
    #[test]
    fn delete_in_album_keeps_library_and_other_albums_confirmation_is_frozen(){
        let (tmp,_ctx,mut browser,ids)=setup(3);
        for (id,name) in [(AlbumId(1),"one"),(AlbumId(2),"two")]{let mut album=Album::new(id,name);album.photos=ids.clone();browser.apply(vec![Op::AddAlbum{album}]);}
        browser.filter.album=Some(AlbumId(1));browser.refresh_query();browser.select_photo(ids[0],Default::default());browser.request_remove(false);
        assert_eq!(browser.pending_remove.as_ref().unwrap().album,Some(AlbumId(1)));
        browser.selected=BTreeSet::from([ids[1]]);browser.confirm_remove();
        let catalog=&browser.library.as_ref().unwrap().catalog;
        assert!(!catalog.album(AlbumId(1)).unwrap().photos.contains(&ids[0]));assert!(catalog.album(AlbumId(1)).unwrap().photos.contains(&ids[1]));
        assert!(catalog.album(AlbumId(2)).unwrap().photos.contains(&ids[0]));assert!(!catalog.photo(ids[0]).unwrap().deleted);
        browser.undo_catalog(false);assert!(browser.library.as_ref().unwrap().catalog.album(AlbumId(1)).unwrap().photos.contains(&ids[0]));
        assert!(tmp.path().join("0.png").is_file());
    }
    #[test]
    fn quick_collection_smart_album_and_keywords_preserve_photo_metadata(){
        let (_tmp,_ctx,mut browser,ids)=setup(3);browser.selected=ids.iter().copied().collect();browser.active=Some(ids[0]);
        browser.action(LibraryAction::QuickToggle,false);let quick=browser.library.as_ref().unwrap().catalog.quick_collection().unwrap();
        assert_eq!(browser.library.as_ref().unwrap().catalog.album_count(quick),3);
        browser.action(LibraryAction::QuickToggle,false);assert_eq!(browser.library.as_ref().unwrap().catalog.album_count(quick),0);
        browser.keyword_input="海岸, Travel, travel".into();browser.set_keywords(false);
        assert_eq!(browser.library.as_ref().unwrap().catalog.photo(ids[0]).unwrap().meta.keywords,vec!["海岸","Travel"]);
        browser.undo_catalog(false);assert!(browser.library.as_ref().unwrap().catalog.photo(ids[0]).unwrap().meta.keywords.is_empty());
        browser.filter.rating=4;browser.create_album_dialog(true);browser.album_dialog.as_mut().unwrap().name="精选".into();browser.finish_album_dialog();
        let smart=browser.library.as_ref().unwrap().catalog.albums().find(|a|a.is_smart()).unwrap().id;assert_eq!(browser.library.as_ref().unwrap().catalog.album_count(smart),0);
        assert_ne!(browser.album_target,Some(smart));
        browser.library.as_mut().unwrap().apply(vec![Op::SetRating{id:ids[1],rating:5}]).unwrap();
        assert_eq!(browser.library.as_ref().unwrap().catalog.album_photos(smart),vec![ids[1]]);
    }
    #[test]
    fn keyboard_auto_advance_color_keys_and_delete_restore_are_distinct(){
        let (_tmp,ctx,mut browser,ids)=setup(3);browser.select_photo(ids[0],Default::default());
        let key=|key,shift|egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers{shift,..Default::default()}};
        frame(&mut browser,&ctx,vec![key(egui::Key::Num6,true)]);
        assert_eq!(browser.library.as_ref().unwrap().catalog.photo(ids[0]).unwrap().label,Some(ColorLabel::Red));assert_eq!(browser.active,Some(ids[1]));
        frame(&mut browser,&ctx,vec![key(egui::Key::X,false)]);assert_eq!(browser.library.as_ref().unwrap().catalog.photo(ids[1]).unwrap().flag,Flag::Reject);
        assert!(!browser.library.as_ref().unwrap().catalog.photo(ids[1]).unwrap().deleted);
        frame(&mut browser,&ctx,vec![key(egui::Key::Delete,false)]);assert!(browser.pending_remove.is_some());
        browser.pending_remove=None;assert!(!browser.library.as_ref().unwrap().catalog.photo(ids[1]).unwrap().deleted);
    }
    #[test]
    fn dragging_selected_photos_into_album_changes_membership_only(){
        let (tmp,ctx,mut browser,ids)=setup(3);browser.apply(vec![Op::AddAlbum{album:Album::new(AlbumId(1),"选片")}]);
        browser.selected=BTreeSet::from([ids[0],ids[1]]);browser.active=Some(ids[0]);frame(&mut browser,&ctx,vec![]);frame(&mut browser,&ctx,vec![]);
        let start=browser.card_rects[&ids[0]].center();let end=browser.album_rects[&AlbumId(1)].center();
        frame(&mut browser,&ctx,vec![egui::Event::PointerMoved(start)]);
        frame(&mut browser,&ctx,vec![egui::Event::PointerButton{pos:start,button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()}]);
        frame(&mut browser,&ctx,vec![egui::Event::PointerMoved(start+Vec2::new(20.0,0.0))]);
        frame(&mut browser,&ctx,vec![egui::Event::PointerMoved(end)]);
        frame(&mut browser,&ctx,vec![egui::Event::PointerButton{pos:end,button:egui::PointerButton::Primary,pressed:false,modifiers:Default::default()}]);
        assert_eq!(browser.library.as_ref().unwrap().catalog.album(AlbumId(1)).unwrap().photos,ids[..2]);
        assert!(tmp.path().join("0.png").is_file());assert_eq!(browser.library.as_ref().unwrap().catalog.photos().filter(|p|p.deleted).count(),0);
    }
    #[test]
    fn compare_candidate_survey_and_target_album_use_active_photo_scope(){
        let (_tmp,ctx,mut browser,ids)=setup(4);browser.select_photo(ids[0],Default::default());browser.set_view(BrowserView::Compare);
        assert_eq!(browser.compare_pair,Some((ids[0],ids[1])));assert_eq!(browser.active,Some(ids[1]));
        browser.action(LibraryAction::Flag(Flag::Pick),false);assert_eq!(browser.library.as_ref().unwrap().catalog.photo(ids[0]).unwrap().flag,Flag::None);
        browser.navigate(1,false);assert_eq!(browser.compare_pair,Some((ids[0],ids[2])));frame(&mut browser,&ctx,vec![]);
        browser.set_view(BrowserView::Survey);assert_eq!(browser.selected.len(),2);browser.action(LibraryAction::Rate(5),false);
        assert_eq!(browser.library.as_ref().unwrap().catalog.photo(ids[2]).unwrap().rating,5);assert_eq!(browser.library.as_ref().unwrap().catalog.photo(ids[0]).unwrap().rating,0);
        browser.apply(vec![Op::AddAlbum{album:Album::new(AlbumId(1),"target")}]);browser.album_target=Some(AlbumId(1));browser.action(LibraryAction::TargetToggle,false);
        assert_eq!(browser.library.as_ref().unwrap().catalog.album_photos(AlbumId(1)),vec![ids[2]]);
    }
    #[test]
    fn view_source_filter_and_selection_restore_after_restart(){
        let (tmp,_ctx,mut browser,ids)=setup(3);browser.select_photo(ids[1],Default::default());browser.view=BrowserView::Loupe;browser.filter.text="1.png".into();browser.auto_advance=true;
        browser.persist_preferences(true);drop(browser);
        let ctx=egui::Context::default();crate::typography::install(&ctx);let reopened=LibraryUi::from_opened(&ctx,Library::open(&tmp.path().join("catalog")));
        assert_eq!(reopened.view,BrowserView::Loupe);assert_eq!(reopened.visible,vec![ids[1]]);assert_eq!(reopened.active,Some(ids[1]));assert!(reopened.auto_advance);
    }
}

