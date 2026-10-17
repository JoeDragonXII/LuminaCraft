//! Catalog-only interaction adapter. Reference: the pinned LightCraft grid, selection and
//! library command implementation; Lightroom Classic's public Library documentation.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq,serde::Serialize,serde::Deserialize)]
#[serde(rename_all="lowercase")]
pub(super) enum BrowserView { #[default] Grid, Loupe, Compare, Survey }

#[derive(serde::Serialize,serde::Deserialize)]
#[serde(default)]
struct ViewPrefs {
    view:BrowserView,filter:Filter,sort:Sort,thumb_size:f32,auto_advance:bool,
    recursive:bool,restore_removed:bool,target:Option<AlbumId>,active:Option<PhotoId>,
    selected:Vec<PhotoId>,compare:Option<(PhotoId,PhotoId)>,
    folder_parents:bool,
}
impl Default for ViewPrefs{
    fn default()->Self{Self{view:BrowserView::Grid,filter:Filter::default(),sort:Sort{key:SortKey::FileName,ascending:true,..Default::default()},thumb_size:190.0,auto_advance:false,recursive:true,restore_removed:true,target:None,active:None,selected:vec![],compare:None,folder_parents:false}}
}
#[derive(Clone)]
pub(super) struct HistoryEntry { pub ops: Vec<Op>, pub label: String }
#[derive(Clone)]
pub(super) struct PendingRemoval { pub ids: Vec<PhotoId>, pub album: Option<AlbumId> }
#[derive(Clone)]
pub(super) struct AlbumDialog { pub id: Option<AlbumId>, pub name: String, pub photos: Vec<PhotoId>, pub include: bool, pub smart: bool }
#[derive(Clone, Copy)]
pub(super) enum LibraryAction {
    Rate(u8), RelativeRating(i8), Flag(Flag), Label(Option<ColorLabel>),
    AddToAlbum(AlbumId), QuickToggle, TargetToggle, Restore,
}

impl LibraryUi {
    fn preferences(&self)->ViewPrefs{ViewPrefs{view:self.view,filter:self.filter.clone(),sort:self.sort,thumb_size:self.thumb_size,auto_advance:self.auto_advance,recursive:self.recursive,restore_removed:self.restore_removed,target:self.album_target,active:self.active,selected:self.selected.iter().copied().collect(),compare:self.compare_pair,folder_parents:self.folders.show_parents}}

    pub(super) fn load_preferences(&mut self){
        let Some(lib)=&self.library else{return;};let path=lib.directory.join("luminacraft-view.json");if !path.exists(){return;}
        let result=(||->anyhow::Result<ViewPrefs>{anyhow::ensure!(std::fs::metadata(&path)?.len()<=1024*1024,"视图配置文件过大。");Ok(serde_json::from_slice(&std::fs::read(&path)?)?)})();
        match result{
            Ok(mut prefs)=>{
                if !prefs.thumb_size.is_finite(){prefs.thumb_size=190.0;}
                if prefs.filter.album.is_some_and(|id|lib.catalog.album(id).is_none()){prefs.filter.album=None;}
                self.filter=prefs.filter;self.sort=prefs.sort;self.view=prefs.view;self.thumb_size=prefs.thumb_size.clamp(150.0,300.0);
                self.auto_advance=prefs.auto_advance;self.recursive=prefs.recursive;self.restore_removed=prefs.restore_removed;
                self.folders.show_parents=prefs.folder_parents;
                self.album_target=prefs.target.filter(|id|lib.catalog.album(*id).is_some_and(|a|!a.folder&&!a.is_smart()));
                self.selected=prefs.selected.into_iter().filter(|id|lib.catalog.photo(*id).is_some()).collect();
                self.active=prefs.active.filter(|id|lib.catalog.photo(*id).is_some());if let Some(id)=self.active{self.selected.insert(id);}
                self.compare_pair=prefs.compare.filter(|(a,b)|lib.catalog.photo(*a).is_some()&&lib.catalog.photo(*b).is_some());
                self.refresh_query();
                self.scroll_to=self.active;
                self.saved_view=serde_json::to_vec(&self.preferences()).unwrap_or_default();
            },
            Err(e)=>{self.prefs_disabled=true;self.message=format!("视图配置读取失败，文件已保留：{e:#}");}
        }
    }

    pub(super) fn persist_preferences(&mut self,force:bool){
        use lightcraft_catalog::Store;
        if self.prefs_disabled||self.library.is_none(){return;}
        let Ok(bytes)=serde_json::to_vec(&self.preferences()) else{return;};if bytes==self.saved_view{self.view_changed=None;return;}
        let changed=self.view_changed.get_or_insert_with(std::time::Instant::now);
        if !force&&changed.elapsed().as_millis()<750{return;}
        let result=self.library.as_ref().ok_or_else(||anyhow::anyhow!("照片库未打开。")).and_then(|lib|Ok(lightcraft_catalog::FsStore::open(&lib.directory)?.write_atomic("luminacraft-view.json",&bytes)?));
        match result{Ok(())=>{self.saved_view=bytes;self.view_changed=None;},Err(e)=>{self.message=format!("视图配置保存失败：{e:#}");self.view_changed=Some(std::time::Instant::now());}}
    }
    pub(super) fn targets(&self) -> Vec<PhotoId> {
        if self.view==BrowserView::Grid {
            self.visible.iter().copied().filter(|id|self.selected.contains(id)).collect()
        } else { self.active.into_iter().collect() }
    }

    pub(super) fn select_photo(&mut self,id:PhotoId,modifiers:egui::Modifiers) {
        let mut selection=lightcraft_engine::Selection { ids:self.visible.iter().copied().filter(|p|self.selected.contains(p)).collect(),active:self.active };
        if modifiers.shift { selection.extend_to(id,&self.visible); }
        else if modifiers.command { selection.toggle(id); }
        else { selection=lightcraft_engine::Selection::single(id); }
        self.selected=selection.ids.into_iter().collect();self.active=selection.active;
    }

    pub(super) fn navigate(&mut self,offset:isize,extend:bool) {
        if self.visible.is_empty(){return;}
        if self.view==BrowserView::Compare {
            if let Some((primary,candidate))=self.compare_pair{
                let order:Vec<_>=self.visible.iter().copied().filter(|id|*id!=primary).collect();
                if let Some(at)=order.iter().position(|id|*id==candidate){
                    let id=order[at.saturating_add_signed(offset).min(order.len()-1)];
                    self.compare_pair=Some((primary,id));self.selected=BTreeSet::from([primary,id]);self.active=Some(id);self.scroll_to=Some(id);
                }
            }
            return;
        }
        if self.view==BrowserView::Survey {
            let order:Vec<_>=self.visible.iter().copied().filter(|id|self.selected.contains(id)).collect();
            if !order.is_empty(){let at=self.active.and_then(|id|order.iter().position(|p|*p==id)).unwrap_or(0);self.active=Some(order[at.saturating_add_signed(offset).min(order.len()-1)]);self.scroll_to=self.active;}
            return;
        }
        let at=self.active.and_then(|id|self.visible.iter().position(|p|*p==id));
        let next=at.map(|n|n.saturating_add_signed(offset).min(self.visible.len()-1)).unwrap_or(0);
        let id=self.visible[next];
        self.select_photo(id,egui::Modifiers{shift:extend,..Default::default()});
        self.scroll_to=Some(id);
    }

    pub(super) fn apply_named(&mut self,ops:Vec<Op>,label:&str)->bool {
        if ops.is_empty(){return false;}
        let has_relinks=ops.iter().any(|op|matches!(op,Op::Relink{..}));
        let Some(lib)=&mut self.library else{return false;};
        match lib.transact(ops) {
            Ok(inverse)=>{
                self.folders.project_paths_changed|=has_relinks;
                self.undo.push(HistoryEntry{ops:inverse,label:label.into()});
                if self.undo.len()>100{self.undo.remove(0);}
                self.redo.clear();self.message=format!("{label} · 已保存");true
            },
            Err(e)=>{self.message=format!("保存失败：{e:#}");false}
        }
    }

    pub fn undo_catalog(&mut self,redo:bool){
        let entry=if redo{self.redo.last()}else{self.undo.last()}.cloned();
        let (Some(entry),Some(lib))=(entry,self.library.as_mut()) else{return;};
        match lib.transact(entry.ops.clone()) {
            Ok(inverse)=>{
                self.folders.project_paths_changed|=entry.ops.iter().any(|op|matches!(op,Op::Relink{..}));
                if redo{self.redo.pop();self.undo.push(HistoryEntry{ops:inverse,label:entry.label.clone()});}
                else{self.undo.pop();self.redo.push(HistoryEntry{ops:inverse,label:entry.label.clone()});}
                self.message=format!("{}{} · 已保存",if redo{"重做："}else{"撤销："},entry.label);
                if self.filter.album.is_some_and(|id|lib.catalog.album(id).is_none()){self.filter.album=None;}
                self.refresh_query();
            },
            Err(e)=>self.message=format!("无法撤销或重做：{e:#}"),
        }
    }

    pub fn can_undo(&self,redo:bool)->bool { if redo{!self.redo.is_empty()}else{!self.undo.is_empty()} }

    pub(super) fn action(&mut self,action:LibraryAction,advance:bool){
        let ids=self.targets();if ids.is_empty(){return;}
        let Some(lib)=&self.library else{return;};
        let (ops,label)=match action {
            LibraryAction::Rate(r)=>(ids.iter().map(|id|Op::SetRating{id:*id,rating:r}).collect(),"设置星级"),
            LibraryAction::RelativeRating(d)=>(ids.iter().filter_map(|id|lib.catalog.photo(*id).map(|p|Op::SetRating{id:*id,rating:(i16::from(p.rating)+i16::from(d)).clamp(0,5) as u8})).collect(),"调整星级"),
            LibraryAction::Flag(flag)=>(ids.iter().map(|id|Op::SetFlag{id:*id,flag}).collect(),"设置标记"),
            LibraryAction::Label(label)=>(ids.iter().map(|id|Op::SetLabel{id:*id,label}).collect(),"设置颜色标签"),
            LibraryAction::Restore=>(ids.iter().map(|id|Op::SetDeleted{id:*id,deleted:false}).collect(),"恢复照片"),
            LibraryAction::AddToAlbum(album_id)=>{
                let Some(album)=lib.catalog.album(album_id) else{return;};
                if album.folder||album.is_smart(){self.message="智能相册由筛选规则决定成员。".into();return;}
                let mut photos=album.photos.clone();
                for id in &ids{if !photos.contains(id){photos.push(*id);}}
                (vec![Op::SetAlbumPhotos{id:album_id,photos}],"加入相册")
            },
            LibraryAction::QuickToggle|LibraryAction::TargetToggle=>{
                let mut ops=vec![];
                let target=matches!(action,LibraryAction::TargetToggle).then_some(self.album_target).flatten().filter(|id|lib.catalog.album(*id).is_some_and(|a|!a.is_smart()&&!a.folder));
                let (album_id,mut photos)=if let Some(id)=target.or_else(||lib.catalog.quick_collection()){(id,lib.catalog.album(id).map(|a|a.photos.clone()).unwrap_or_default())}
                    else{
                        let mut allocation=lib.catalog.clone();let id=allocation.alloc_album_id();let mut album=Album::new(id,"快捷收藏");album.quick=true;
                        ops.push(Op::AddAlbum{album});(id,vec![])
                    };
                if ids.iter().all(|id|photos.contains(id)){photos.retain(|id|!ids.contains(id));}
                else{for id in &ids{if !photos.contains(id){photos.push(*id);}}}
                ops.push(Op::SetAlbumPhotos{id:album_id,photos});(ops,if target.is_some(){"更新目标相册"}else{"更新快捷收藏"})
            }
        };
        if self.apply_named(ops,label){
            let metadata=matches!(action,LibraryAction::Rate(_)|LibraryAction::RelativeRating(_)|LibraryAction::Flag(_)|LibraryAction::Label(_));
            if advance||(metadata&&self.auto_advance){self.navigate(1,false);}
        }
    }

    pub(super) fn request_remove(&mut self,force_catalog:bool) {
        let ids=self.targets();if ids.is_empty(){return;}
        if self.filter.deleted{self.action(LibraryAction::Restore,false);return;}
        let album=if force_catalog{None}else{self.filter.album.filter(|id|self.library.as_ref().and_then(|l|l.catalog.album(*id)).is_some_and(|a|!a.is_smart()&&!a.folder))};
        self.pending_remove=Some(PendingRemoval{ids,album});
    }

    pub(super) fn confirm_remove(&mut self) {
        let Some(pending)=self.pending_remove.clone() else{return;};
        let Some(lib)=&self.library else{return;};
        let before=self.visible.clone();let at=self.active.and_then(|id|before.iter().position(|p|*p==id)).unwrap_or(0);
        let (ops,label)=if let Some(id)=pending.album {
            let Some(album)=lib.catalog.album(id) else{self.pending_remove=None;return;};
            (vec![Op::SetAlbumPhotos{id,photos:album.photos.iter().copied().filter(|p|!pending.ids.contains(p)).collect()}],"从相册移除")
        }else{(pending.ids.iter().map(|id|Op::SetDeleted{id:*id,deleted:true}).collect(),"移出照片库")};
        if self.apply_named(ops,label){
            self.pending_remove=None;self.refresh_query();
            if self.active.is_none(){if let Some(id)=self.visible.get(at.min(self.visible.len().saturating_sub(1))).copied(){self.select_photo(id,Default::default());}}
        }
    }

    pub(super) fn set_view(&mut self,view:BrowserView) {
        self.refresh_query();
        if self.active.is_none(){if let Some(id)=self.visible.first().copied(){self.select_photo(id,Default::default());}}
        if view==BrowserView::Compare {
            let active=self.active;
            let candidate=self.visible.iter().copied().find(|id|Some(*id)!=active&&self.selected.contains(id))
                .or_else(||active.and_then(|id|self.visible.iter().position(|p|*p==id)).and_then(|i|self.visible.get(i+1).or_else(||i.checked_sub(1).and_then(|i|self.visible.get(i)))).copied());
            if let Some(id)=candidate{self.selected.insert(id);}
            if let (Some(primary),Some(candidate))=(active,candidate){self.compare_pair=Some((primary,candidate));self.active=Some(candidate);}
        }
        self.view=view;
        self.scroll_to=self.active;
    }

    pub fn open_grid(&mut self){self.set_view(BrowserView::Grid);}

    pub(super) fn set_keywords(&mut self,replace:bool){
        let mut keywords=Vec::<String>::new();
        for word in self.keyword_input.split([',','，',';','；']).map(str::trim).filter(|w|!w.is_empty()){
            if word.len()>256||keywords.len()>=200{self.message="关键词过多或过长，请缩短后重试。".into();return;}
            if !keywords.iter().any(|w|w.eq_ignore_ascii_case(word)){keywords.push(word.into());}
        }
        let ids=self.targets();let Some(lib)=&self.library else{return;};let mut ops=vec![];
        for id in ids{
            if let Some(photo)=lib.catalog.photo(id){let mut meta=photo.meta.clone();
                if replace{meta.keywords=keywords.clone();}else{for word in &keywords{if !meta.keywords.iter().any(|w|w.eq_ignore_ascii_case(word)){meta.keywords.push(word.clone());}}}
                if meta.keywords.len()>200{self.message="合并后关键词超过 200 个，未应用任何修改。".into();return;}
                ops.push(Op::SetMeta{id,meta:Box::new(meta)});
            }
        }
        if self.apply_named(ops,"更新关键词"){self.keyword_for=None;}
    }

    pub(super) fn filmstrip(&mut self,ui:&mut egui::Ui,editor_busy:bool,open:&mut Option<Vec<PathBuf>>){
        egui::Panel::bottom("catalog-filmstrip").exact_size(90.0).show_separator_line(false).frame(crate::chrome::surface_frame(8)).show(ui,|ui|{
            let mut scroll=egui::ScrollArea::horizontal().id_salt("catalog-strip");
            if let Some(id)=self.scroll_to.take(){if let Some(at)=self.visible.iter().position(|p|*p==id){scroll=scroll.horizontal_scroll_offset(at as f32*100.0);}}
            scroll.show_viewport(ui,|ui,viewport|{
                ui.set_width(self.visible.len() as f32*100.0);ui.set_height(76.0);
                let start=(viewport.min.x/100.0).floor().max(0.0) as usize;
                let end=((viewport.max.x/100.0).ceil() as usize+1).min(self.visible.len());
                for index in start..end{
                    let id=self.visible[index];self.request_thumb(id);
                    let rect=egui::Rect::from_min_size(ui.max_rect().min+Vec2::new(index as f32*100.0,0.0),Vec2::new(92.0,68.0));
                    if let Some(texture)=self.textures.get(&id){let original=texture.size_vec2();let scale=(rect.width()/original.x).min(rect.height()/original.y);ui.painter().image(texture.id(),egui::Rect::from_center_size(rect.center(),original*scale),egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),Color32::WHITE);}
                    if self.selected.contains(&id){ui.painter().rect_stroke(rect,crate::chrome::CONTROL_RADIUS,egui::Stroke::new(if self.active==Some(id){2.0}else{1.0},if self.active==Some(id){crate::chrome::ACCENT}else{crate::chrome::MUTED}),egui::StrokeKind::Inside);}
                    let response=ui.interact(rect,ui.id().with(("strip",id.0)),egui::Sense::click());
                    if response.clicked(){
                        if self.view==BrowserView::Compare{if let Some((primary,_))=self.compare_pair{if id!=primary{self.compare_pair=Some((primary,id));self.selected=BTreeSet::from([primary,id]);self.active=Some(id);}}}
                        else if self.view==BrowserView::Survey{self.active=Some(id);self.selected.insert(id);}
                        else{self.select_photo(id,ui.input(|i|i.modifiers));}
                    }
                    response.context_menu(|ui|self.context_menu(ui,id,editor_busy,open));
                }
            });
        });
    }

    pub(super) fn create_album_dialog(&mut self,smart:bool){
        self.album_dialog=Some(AlbumDialog{id:None,name:String::new(),photos:self.targets(),include:true,smart});
    }

    pub(super) fn finish_album_dialog(&mut self){
        let Some(dialog)=self.album_dialog.clone() else{return;};
        let name=dialog.name.trim();if name.is_empty()||name.len()>256{return;}
        let Some(lib)=&self.library else{return;};
        let (id,ops)=if let Some(id)=dialog.id{(id,vec![Op::RenameAlbum{id,name:name.into()}])}
            else{
                let mut allocation=lib.catalog.clone();let id=allocation.alloc_album_id();let mut album=Album::new(id,name);
                if dialog.smart{let mut rules=self.filter.clone();rules.album=None;rules.deleted=false;album.smart=Some(Box::new(rules));}
                else if dialog.include{album.photos=dialog.photos;}
                (id,vec![Op::AddAlbum{album}])
            };
        if self.apply_named(ops,if dialog.id.is_some(){"重命名相册"}else{"创建相册"}){self.album_dialog=None;if dialog.id.is_none()&&!dialog.smart{self.album_target=Some(id);}}
    }

    pub(super) fn reveal(&mut self,id:PhotoId){
        let path=self.library.as_ref().and_then(|l|l.catalog.photo(id)).and_then(|p|library::photo_path(p)).map(str::to_owned);
        if let Some(path)=path {
            let path=path.strip_prefix(r"\\?\").unwrap_or(&path);
            if let Err(e)=std::process::Command::new("explorer.exe").arg("/select,").arg(path).spawn(){self.message=format!("无法打开资源管理器：{e}");}
        }
    }

    pub(super) fn context_menu(&mut self,ui:&mut egui::Ui,id:PhotoId,editor_busy:bool,open:&mut Option<Vec<PathBuf>>) {
        if !self.selected.contains(&id){self.select_photo(id,Default::default());}
        self.active=Some(id);
        ui.label(format!("{} 张照片",self.targets().len()));ui.separator();
        if ui.button("放大查看  E").clicked(){self.set_view(BrowserView::Loupe);ui.close();}
        if ui.add_enabled(!editor_busy,egui::Button::new("进入编辑  D")).clicked(){*open=Some(self.selected_paths());ui.close();}
        ui.separator();
        ui.menu_button("设置星级",|ui|{for rating in 0..=5{if ui.button(if rating==0{"无星级".into()}else{"★".repeat(rating as usize)}).clicked(){self.action(LibraryAction::Rate(rating),false);egui::Popup::close_all(ui.ctx());}}});
        ui.menu_button("设置标记",|ui|{for (flag,title) in [(Flag::Pick,"入选  P"),(Flag::Reject,"排除  X"),(Flag::None,"清除标记  U")]{if ui.button(title).clicked(){self.action(LibraryAction::Flag(flag),false);egui::Popup::close_all(ui.ctx());}}});
        ui.menu_button("设置颜色标签",|ui|{
            for (label,color) in label_colors(){if ui.button(RichText::new(label_name(Some(label))).color(color)).clicked(){self.action(LibraryAction::Label(Some(label)),false);egui::Popup::close_all(ui.ctx());}}
            if ui.button("清除颜色标签").clicked(){self.action(LibraryAction::Label(None),false);egui::Popup::close_all(ui.ctx());}
        });
        ui.separator();
        if !self.filter.deleted{
            if self.album_target.is_some()&&ui.button("加入 / 移出快捷收藏").clicked(){self.action(LibraryAction::QuickToggle,false);ui.close();}
            if ui.button(if self.album_target.is_some(){"加入 / 移出目标相册  B"}else{"加入 / 移出快捷收藏  B"}).clicked(){self.action(LibraryAction::TargetToggle,false);ui.close();}
            ui.menu_button("加入相册",|ui|{
                let albums:Vec<_>=self.library.as_ref().map(|l|l.catalog.albums().filter(|a|!a.folder&&!a.is_smart()&&!a.quick).map(|a|(a.id,a.name.clone())).collect()).unwrap_or_default();
                for (id,name) in albums{if ui.button(name).clicked(){self.action(LibraryAction::AddToAlbum(id),false);egui::Popup::close_all(ui.ctx());}}
                ui.separator();if ui.button("新建相册…").clicked(){self.create_album_dialog(false);egui::Popup::close_all(ui.ctx());}
            });
            if let Some(album)=self.filter.album.and_then(|id|self.library.as_ref()?.catalog.album(id).cloned()).filter(|a|!a.is_smart()&&!a.folder){
                if ui.button("从当前相册移除…").clicked(){self.request_remove(false);ui.close();}
                if ui.button("设为相册封面").clicked(){self.apply_named(vec![Op::SetAlbumCover{id:album.id,cover:Some(id)}],"设置相册封面");ui.close();}
            }
        }
        if ui.button("在资源管理器中显示").clicked(){self.reveal(id);ui.close();}
        ui.separator();
        let action=ui.button(if self.filter.deleted{"恢复到照片库"}else{"移出照片库…"});
        #[cfg(test)] {self.menu_action_rect=Some(action.rect);}
        if action.clicked(){if self.filter.deleted{self.action(LibraryAction::Restore,false);}else{self.request_remove(true);}ui.close();}
        if self.filter.deleted&&ui.button("从磁盘删除…").clicked(){if let Some(lib)=&self.library{match luminacraft::deletion::prepare(&lib.catalog,&self.targets()){Ok(plan)=>self.pending_disk=Some(plan),Err(e)=>self.message=format!("{e:#}")}}ui.close();}
    }

    pub(super) fn dialogs(&mut self,ctx:&egui::Context){
        if let Some(pending)=self.pending_remove.clone(){
            let mut confirm=false;let mut cancel=false;let mut disk=false;let mut catalog=false;
            let response=egui::Modal::new(egui::Id::new("remove-photos")).show(ctx,|ui|{
                ui.heading(if pending.album.is_some(){"从相册移除"}else{"移出照片库"});
                ui.label(format!("将移除 {} 张选中的照片。",pending.ids.len()));
                ui.label(if pending.album.is_some(){"照片仍保留在照片库及其他相册中。"}else{"照片可在“最近删除”中恢复。"});
                ui.small("硬盘上的原照片和 XMP 保持原样。");ui.separator();
                ui.horizontal(|ui|{
                    let button=ui.button(if pending.album.is_some(){"从相册移除"}else{"从照片库移除"});
                    #[cfg(test)] {self.confirm_rect=Some(button.rect);}
                    lightcraft_ui_egui::widgets::register(ui.ctx(),"main.delete.remove",button.rect);
                    confirm=button.clicked();if pending.album.is_some(){catalog=ui.button("从照片库移除").clicked();}let button=ui.button("从磁盘删除…");lightcraft_ui_egui::widgets::register(ui.ctx(),"main.delete.disk",button.rect);disk=button.clicked();cancel=ui.button("取消").clicked();
                });
            });
            if disk{if let Some(lib)=&self.library{match luminacraft::deletion::prepare(&lib.catalog,&pending.ids){Ok(plan)=>{self.pending_remove=None;self.pending_disk=Some(plan);},Err(e)=>self.message=format!("{e:#}")}}}
            else if catalog{if let Some(p)=&mut self.pending_remove{p.album=None;}self.confirm_remove();}
            else if confirm{self.confirm_remove();}else if cancel||response.should_close(){self.pending_remove=None;}
        }
        if let Some(plan)=self.pending_disk.clone(){let mut confirm=false;let mut cancel=false;
            let response=egui::Modal::new(egui::Id::new("delete-from-disk")).show(ctx,|ui|{
                ui.heading("从磁盘删除原片");ui.label(format!("将 {} 个原片移入 Windows 回收站，并移除 {} 条照片库记录。",plan.files.len(),plan.records));
                ui.label("同一原片的虚拟副本和其他相册中的引用也会移除。");
                egui::ScrollArea::vertical().max_height(140.0).show(ui,|ui|{for file in &plan.files{ui.small(file.path.display().to_string());}});
                ui.small("原 XMP 保留。App 撤销不能还原磁盘文件；需要时请从 Windows 回收站还原，再重新导入。");
                ui.horizontal(|ui|{let button=ui.button("移入回收站并移除记录");lightcraft_ui_egui::widgets::register(ui.ctx(),"main.delete.confirmDisk",button.rect);confirm=button.clicked();cancel=ui.button("取消").clicked();});
            });
            if confirm{if let Some(lib)=&mut self.library{match luminacraft::deletion::from_library(lib,&plan){Ok(report)=>{if report.recycled>0{self.undo.clear();self.redo.clear();}self.message=format!("已回收 {} 个原片，移除 {} 条记录。{}",report.recycled,report.removed,report.errors.join("；"));if !report.errors.is_empty(){self.message.push_str(&format!(" 恢复档案：{}",report.recovery.display()));}},Err(e)=>self.message=format!("未完成磁盘删除：{e:#}")}}self.pending_disk=None;self.refresh_query();}
            else if cancel||response.should_close(){self.pending_disk=None;}
        }
        if let Some(mut dialog)=self.album_dialog.clone(){
            let mut confirm=false;let mut cancel=false;
            let response=egui::Modal::new(egui::Id::new("album-dialog")).show(ctx,|ui|{
                ui.heading(if dialog.id.is_some(){"重命名相册"}else if dialog.smart{"从筛选创建智能相册"}else{"新建相册"});
                ui.add(egui::TextEdit::singleline(&mut dialog.name).hint_text("相册名称").char_limit(64));
                if dialog.id.is_none(){if dialog.smart{ui.label("成员随当前搜索、星级、标记和颜色条件自动更新。");}else{ui.checkbox(&mut dialog.include,format!("包含 {} 张选中的照片",dialog.photos.len()));}}
                ui.horizontal(|ui|{confirm=ui.add_enabled(!dialog.name.trim().is_empty(),egui::Button::new("保存")).clicked();cancel=ui.button("取消").clicked();});
            });
            self.album_dialog=Some(dialog);
            if confirm{self.finish_album_dialog();}else if cancel||response.should_close(){self.album_dialog=None;}
        }
        if let Some(id)=self.delete_album {
            let mut confirm=false;let mut cancel=false;
            let response=egui::Modal::new(egui::Id::new("delete-album")).show(ctx,|ui|{
                ui.heading("删除相册");ui.label("只删除相册及成员关系，照片仍保留在照片库中。");
                ui.horizontal(|ui|{confirm=ui.button("删除相册").clicked();cancel=ui.button("取消").clicked();});
            });
            if confirm&&self.apply_named(vec![Op::RemoveAlbum{id}],"删除相册"){
                self.delete_album=None;if self.filter.album==Some(id){self.filter.album=None;}
            }else if cancel||response.should_close(){self.delete_album=None;}
        }
    }

    pub(super) fn keyboard(&mut self,ui:&mut egui::Ui,columns:usize,editor_busy:bool,open:&mut Option<Vec<PathBuf>>){
        if ui.ctx().text_edit_focused()||egui::Popup::is_any_open(ui.ctx())||self.pending_remove.is_some()||self.album_dialog.is_some()||self.delete_album.is_some(){return;}
        let modifiers=ui.input(|i|i.modifiers);
        let pressed=|key|ui.input(|i|i.key_pressed(key));
        if modifiers.command {
            if pressed(egui::Key::Z){self.undo_catalog(modifiers.shift);}
            if pressed(egui::Key::Y){self.undo_catalog(true);}
            if pressed(egui::Key::A){self.selected=self.visible.iter().copied().collect();if self.active.is_none(){self.active=self.visible.first().copied();}}
            if pressed(egui::Key::D){self.selected.clear();self.active=None;}
            if pressed(egui::Key::B){
                if let Some(id)=self.library.as_ref().and_then(|l|l.catalog.quick_collection()){self.filter.album=Some(id);self.filter.library_folder=None;self.filter.deleted=false;}
            }
            return;
        }
        if modifiers.alt{return;}
        if pressed(egui::Key::G)||pressed(egui::Key::Escape){self.set_view(BrowserView::Grid);}
        if pressed(egui::Key::E)||pressed(egui::Key::Enter){self.set_view(BrowserView::Loupe);}
        if pressed(egui::Key::Space){self.set_view(if self.view==BrowserView::Grid{BrowserView::Loupe}else{BrowserView::Grid});}
        if pressed(egui::Key::C){self.set_view(BrowserView::Compare);}
        if pressed(egui::Key::N){self.set_view(BrowserView::Survey);}
        if pressed(egui::Key::D)&&!editor_busy{let paths=self.selected_paths();if !paths.is_empty(){*open=Some(paths);}}
        let vertical=if self.view==BrowserView::Grid{columns as isize}else{1};
        let direction=if pressed(egui::Key::ArrowRight){1}else if pressed(egui::Key::ArrowLeft){-1}else if pressed(egui::Key::ArrowDown){vertical}else if pressed(egui::Key::ArrowUp){-vertical}else{0};
        if direction!=0{self.navigate(direction,modifiers.shift);}
        if pressed(egui::Key::Home){self.navigate(-(self.visible.len() as isize),modifiers.shift);}
        if pressed(egui::Key::End){self.navigate(self.visible.len() as isize,modifiers.shift);}
        if pressed(egui::Key::Delete)||pressed(egui::Key::Backspace){self.request_remove(false);return;}
        for (key,rating) in [(egui::Key::Num0,0),(egui::Key::Num1,1),(egui::Key::Num2,2),(egui::Key::Num3,3),(egui::Key::Num4,4),(egui::Key::Num5,5)]{
            if pressed(key){self.action(LibraryAction::Rate(rating),modifiers.shift);return;}
        }
        for (key,label) in [(egui::Key::Num6,ColorLabel::Red),(egui::Key::Num7,ColorLabel::Yellow),(egui::Key::Num8,ColorLabel::Green),(egui::Key::Num9,ColorLabel::Blue)]{
            if pressed(key){self.action(LibraryAction::Label(Some(label)),modifiers.shift);return;}
        }
        for (key,flag) in [(egui::Key::P,Flag::Pick),(egui::Key::X,Flag::Reject),(egui::Key::U,Flag::None)]{
            if pressed(key){self.action(LibraryAction::Flag(flag),modifiers.shift);return;}
        }
        if pressed(egui::Key::OpenBracket){self.action(LibraryAction::RelativeRating(-1),modifiers.shift);}
        if pressed(egui::Key::CloseBracket){self.action(LibraryAction::RelativeRating(1),modifiers.shift);}
        if pressed(egui::Key::B){self.action(LibraryAction::TargetToggle,modifiers.shift);}
    }

    pub(super) fn browse_views(&mut self,ui:&mut egui::Ui,editor_busy:bool,open:&mut Option<Vec<PathBuf>>){
        let ids:Vec<_>=match self.view{
            BrowserView::Grid=>return,
            BrowserView::Loupe=>self.active.into_iter().collect(),
            BrowserView::Compare=>self.compare_pair.map(|(a,b)|vec![a,b]).unwrap_or_else(||self.active.into_iter().collect()),
            BrowserView::Survey=>self.visible.iter().copied().filter(|id|self.selected.contains(id)).take(12).collect(),
        };
        if ids.is_empty(){ui.centered_and_justified(|ui|{ui.label("选择照片后查看");});return;}
        if self.view==BrowserView::Compare{
            ui.horizontal(|ui|{
                if ui.button("交换参考与候选").clicked(){if let Some((a,b))=self.compare_pair{self.compare_pair=Some((b,a));self.active=Some(a);}}
                if ui.button("选定候选").clicked(){if let Some((_,b))=self.compare_pair{
                    let candidate=self.visible.iter().position(|id|*id==b).and_then(|i|self.visible.get(i+1).or_else(||i.checked_sub(1).and_then(|i|self.visible.get(i)))).copied();
                    if let Some(candidate)=candidate{self.compare_pair=Some((b,candidate));self.selected=BTreeSet::from([b,candidate]);self.active=Some(candidate);}
                }}
                if ui.button("上一张").clicked(){self.navigate(-1,false);}
                if ui.button("下一张").clicked(){self.navigate(1,false);}
            });
        }
        let columns=match self.view{BrowserView::Loupe=>1,BrowserView::Compare=>2,_=>if ids.len()<=4{2}else{3}};
        let rows=ids.len().div_ceil(columns);
        let size=Vec2::new(((ui.available_width()-(columns-1) as f32*10.0)/columns as f32).max(60.0),((ui.available_height()-rows as f32*56.0)/rows as f32).max(80.0));
        let mut leave=None;
        for row in ids.chunks(columns){ui.horizontal(|ui|{for id in row{
            self.request_image(*id,1600);
            let Some(photo)=self.library.as_ref().and_then(|l|l.catalog.photo(*id)).cloned() else{continue;};
            let texture=self.previews.get(id).or_else(||self.textures.get(id));
            let mut response=None;
            ui.vertical(|ui|{
                ui.set_width(size.x);
                let (rect,resp)=ui.allocate_exact_size(size,egui::Sense::click());
                if let Some(texture)=texture{let actual=texture.size_vec2();let scale=(rect.width()/actual.x).min(rect.height()/actual.y);ui.painter().image(texture.id(),egui::Rect::from_center_size(rect.center(),actual*scale),egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),Color32::WHITE);}
                else{ui.painter().text(rect.center(),egui::Align2::CENTER_CENTER,if self.failed.contains_key(id){"无法预览"}else{"正在加载预览…"},egui::FontId::proportional(14.0),crate::chrome::MUTED);}
                if self.active==Some(*id){ui.painter().rect_stroke(rect,crate::chrome::CONTROL_RADIUS,egui::Stroke::new(2.0,crate::chrome::ACCENT),egui::StrokeKind::Inside);}
                ui.horizontal(|ui|{ui.add(egui::Label::new(format!("{} · {}★ · {}",photo.file_name,photo.rating,flag_name(Some(photo.flag)))).truncate());if self.view==BrowserView::Survey&&ui.small_button("×").on_hover_text("仅从本次筛选视图排除").clicked(){leave=Some(*id);}});
                if self.view==BrowserView::Compare{ui.small(if self.compare_pair.is_some_and(|(a,_)|a==*id){"参考"}else{"候选"});}
                response=Some(resp);
            });
            if let Some(response)=response{
                if response.clicked(){self.active=Some(*id);}
                if response.double_clicked(){self.set_view(BrowserView::Grid);}
                response.context_menu(|ui|self.context_menu(ui,*id,editor_busy,open));
            }
        }});}
        if let Some(id)=leave{self.selected.remove(&id);if self.active==Some(id){self.active=self.selected.iter().next().copied();}}
    }
}

