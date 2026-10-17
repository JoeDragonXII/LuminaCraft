//! Compact library controls; menus retain the existing filter and selection logic.
use super::*;
use lightcraft_ui_egui::icons::Icon;

fn register(ui:&egui::Ui,id:&str,response:&egui::Response){lightcraft_ui_egui::widgets::register(ui.ctx(),id,response.rect);}

impl LibraryUi{
    pub(super) fn selection_controls(&mut self,ui:&mut egui::Ui,ids:&[PhotoId]){
        let states:Vec<_>=self.library.as_ref().map(|lib|ids.iter().filter_map(|id|lib.catalog.photo(*id)).map(|p|(p.rating,p.flag,p.label)).collect()).unwrap_or_default();
        ui.scope(|ui|{
            ui.spacing_mut().item_spacing.x=6.0;
            chrome::centered_row(ui,34.0,|ui|{
                let width=(ui.available_width()-12.0)/3.0;
                for (kind,title) in [("rating","星级"),("flag","标记"),("label","颜色")]{
                    let r=chrome::text_button(ui,&format!("luminacraft.library.assign.{kind}"),title,width,false,ui.is_enabled());
                    lightcraft_ui_egui::icons::paint(ui.painter(),egui::Rect::from_center_size(egui::pos2(r.rect.right()-10.0,r.rect.center().y),Vec2::splat(12.0)),Icon::ChevronDown,chrome::MUTED);
                    egui::Popup::menu(&r).show(|ui|{
                        ui.set_min_width(228.0);ui.spacing_mut().item_spacing.x=4.0;
                        ui.weak(match kind{"rating"=>"设置星级", "flag"=>"设置标记",_=>"设置颜色标签"});
                        chrome::centered_row(ui,34.0,|ui|{
                            match kind{
                                "rating"=>for rating in 0..=5{
                                    let selected=!states.is_empty()&&states.iter().all(|s|s.0==rating);
                                    let label=if rating==0{"清除星级 · 0".into()}else{format!("{rating} 星 · {rating}")};
                                    let r=if rating==0{chrome::icon_button(ui,"luminacraft.library.rate.0",Icon::Close,&label,selected,true)}else{chrome::text_button(ui,&format!("luminacraft.library.rate.{rating}"),&rating.to_string(),34.0,selected,true).on_hover_text(label)};
                                    if r.clicked(){self.action(LibraryAction::Rate(rating),false);ui.close();}
                                },
                                "flag"=>{let w=(ui.available_width()-8.0)/3.0;for(flag,label,id)in [(Flag::Pick,"入选","pick"),(Flag::Reject,"排除","reject"),(Flag::None,"清除","none")]{
                                    let selected=!states.is_empty()&&states.iter().all(|s|s.1==flag);
                                    if chrome::text_button(ui,&format!("luminacraft.library.flag.{id}"),label,w,selected,true).clicked(){self.action(LibraryAction::Flag(flag),false);ui.close();}
                                }},
                                _=>{
                                    if chrome::icon_button(ui,"luminacraft.library.label.clear",Icon::Close,"清除颜色标签",!states.is_empty()&&states.iter().all(|s|s.2.is_none()),true).clicked(){self.action(LibraryAction::Label(None),false);ui.close();}
                                    for(label,color)in label_colors(){
                                        let selected=!states.is_empty()&&states.iter().all(|s|s.2==Some(label));
                                        let r=chrome::icon_button(ui,&format!("luminacraft.library.label.{label:?}"),Icon::Circle,label_name(Some(label)),selected,true);
                                        ui.painter().circle_filled(r.rect.center(),7.0,color);
                                        if r.clicked(){self.action(LibraryAction::Label(Some(label)),false);ui.close();}
                                    }
                                }
                            }
                        });
                    });
                }
            });
        });
    }
    pub(super) fn toolbar(&mut self,ui:&mut egui::Ui){
        ui.scope(|ui|{
            ui.spacing_mut().interact_size.y=34.0;
            ui.spacing_mut().item_spacing=Vec2::new(8.0,8.0);
            chrome::centered_row(ui,34.0,|ui|{
                let(view_icon,view_name)=match self.view{BrowserView::Grid=>(Icon::GridPhoto,"网格"),BrowserView::Loupe=>(Icon::Single,"放大"),BrowserView::Compare=>(Icon::Compare,"对比"),BrowserView::Survey=>(Icon::Survey,"多图")};
                let view=chrome::icon_button(ui,"luminacraft.library.views",view_icon,&format!("{view_name}视图 · 切换视图"),false,true);
                egui::Popup::menu(&view).show(|ui|{
                    for(view,id,title,key)in [(BrowserView::Grid,"grid","网格","G"),(BrowserView::Loupe,"loupe","放大","E"),(BrowserView::Compare,"compare","对比","C"),(BrowserView::Survey,"survey","多图","N")]{
                        let r=ui.selectable_label(self.view==view,format!("{title}    {key}"));register(ui,&format!("luminacraft.library.view.{id}"),&r);
                        if r.clicked(){self.set_view(view);ui.close();}
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                let selection=chrome::icon_button(ui,"luminacraft.library.selection",Icon::More,&format!("选择与视图选项 · 已选 {} 张",self.selected.len()),false,true);
                egui::Popup::menu(&selection).show(|ui|{
                    ui.weak(format!("{} 张结果 · 已选 {} 张",self.visible.len(),self.selected.len()));
                    let all=ui.add_enabled(!self.visible.is_empty(),egui::Button::new("全选当前结果"));register(ui,"luminacraft.library.select-all",&all);
                    if all.clicked(){self.selected=self.visible.iter().copied().collect();ui.close();}
                    let clear=ui.add_enabled(!self.selected.is_empty(),egui::Button::new("清除选择"));register(ui,"luminacraft.library.clear-selection",&clear);
                    if clear.clicked(){self.selected.clear();ui.close();}
                    ui.checkbox(&mut self.auto_advance,"标记后自动前进");
                    if self.view==BrowserView::Grid{ui.separator();ui.label("缩略图大小");ui.add(egui::Slider::new(&mut self.thumb_size,150.0..=300.0).show_value(false));}
                });
                let sort=chrome::icon_button(ui,"luminacraft.library.sort",Icon::Sort,"排序",false,true);
                egui::Popup::menu(&sort).show(|ui|{
                    for key in [SortKey::FileName,SortKey::CaptureDate,SortKey::ImportDate,SortKey::Rating,SortKey::FileSize]{if ui.selectable_value(&mut self.sort.key,key,sort_name(key)).clicked(){ui.close();}}
                    ui.separator();ui.checkbox(&mut self.sort.ascending,"升序");
                });
                let active=usize::from(self.filter.rating>0)+usize::from(self.filter.flag.is_some())+usize::from(self.filter.label.is_some());
                let filter=chrome::icon_button(ui,"luminacraft.library.filters",Icon::Filter,&format!("筛选 · {active} 项条件"),active>0,true);
                egui::Popup::menu(&filter).show(|ui|{
                    ui.set_min_width(180.0);
                    ui.label("星级");
                    egui::ComboBox::from_id_salt("rating-filter").width(170.0).selected_text(if self.filter.rating==0{"全部星级".into()}else{format!("≥ {}★",self.filter.rating)}).show_ui(ui,|ui|{for r in 0..=5{ui.selectable_value(&mut self.filter.rating,r,if r==0{"全部星级".into()}else{format!("≥ {r}★")});}});
                    ui.label("标记");
                    egui::ComboBox::from_id_salt("flag-filter").width(170.0).selected_text(flag_name(self.filter.flag)).show_ui(ui,|ui|{for flag in [None,Some(Flag::Pick),Some(Flag::Reject),Some(Flag::None)]{ui.selectable_value(&mut self.filter.flag,flag,flag_name(flag));}});
                    ui.label("颜色");
                    egui::ComboBox::from_id_salt("color-filter").width(170.0).selected_text(label_name(self.filter.label)).show_ui(ui,|ui|{ui.selectable_value(&mut self.filter.label,None,"全部颜色");for (label,_) in label_colors(){ui.selectable_value(&mut self.filter.label,Some(label),label_name(Some(label)));}});
                    let reset=ui.button("重置筛选").on_hover_text("清除搜索、星级、标记和颜色；保留当前相册及文件夹范围");register(ui,"luminacraft.library.reset-filters",&reset);
                    if reset.clicked(){let album=self.filter.album;let folder=self.filter.folder.clone();let library_folder=self.filter.library_folder.clone();let deleted=self.filter.deleted;self.filter=Filter{album,folder,library_folder,deleted,..Default::default()};ui.close();}
                });
                if ui.available_width()>350.0{ui.label(egui::RichText::new(format!("{} 张",self.visible.len())).small().color(chrome::MUTED));}
                let search=ui.add_sized(Vec2::new(ui.available_width(),34.0),egui::TextEdit::singleline(&mut self.filter.text).hint_text("搜索照片").vertical_align(egui::Align::Center)).on_hover_text("搜索文件名、相机或关键词");
                register(ui,"luminacraft.library.search",&search);
                });
            });
            self.refresh_query();
        });
    }
}
