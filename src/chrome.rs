//! Shared native App chrome; no project or image mutations.
use eframe::egui::{self,Color32,Vec2};
// Reference palette: mint #97FE98, graphite #303531, cream #FFF2DF.
// Keep the actual photograph surround neutral to avoid a green viewing cast.
pub(crate) const SURFACE:Color32=Color32::from_rgb(35,40,37);
pub(crate) const PANEL:Color32=Color32::from_rgb(48,53,49);
pub(crate) const CANVAS:Color32=Color32::from_rgb(29,32,31);
pub(crate) const CONTROL:Color32=Color32::from_rgb(58,65,59);
pub(crate) const HOVER:Color32=Color32::from_rgb(72,82,74);
pub(crate) const SELECTED:Color32=Color32::from_rgb(61,82,65);
pub(crate) const MUTED:Color32=Color32::from_rgb(190,199,188);
pub(crate) const INK:Color32=Color32::from_rgb(255,242,223);
pub(crate) const ACCENT:Color32=Color32::from_rgb(151,254,152);
pub(crate) const ON_ACCENT:Color32=Color32::from_rgb(35,47,38);
pub(crate) const ERROR:Color32=Color32::from_rgb(255,188,180);
pub(crate) const WARNING:Color32=Color32::from_rgb(234,202,151);
// Shared radius hierarchy: shells, cards/status, and interactive controls/media.
pub(crate) const SHELL_RADIUS:u8=16;
pub(crate) const CARD_RADIUS:u8=12;
pub(crate) const CONTROL_RADIUS:u8=8;
pub(crate) const CONTROL_HEIGHT:f32=34.0;

/// Shared fixed-size controls keep icon, rating and navigation rows on one baseline.
pub(crate) fn icon_button(ui:&mut egui::Ui,id:&str,icon:lightcraft_ui_egui::icons::Icon,label:&str,selected:bool,enabled:bool)->egui::Response{
    icon_button_width(ui,id,icon,label,selected,enabled,CONTROL_HEIGHT)
}
pub(crate) fn icon_button_width(ui:&mut egui::Ui,id:&str,icon:lightcraft_ui_egui::icons::Icon,label:&str,selected:bool,enabled:bool,width:f32)->egui::Response{
    let(r,fg)=control(ui,id,label,selected,enabled,Vec2::new(width,CONTROL_HEIGHT));
    lightcraft_ui_egui::icons::paint(ui.painter(),egui::Rect::from_center_size(r.rect.center(),Vec2::splat(18.0)),icon,fg);
    r.on_hover_text(label)
}
pub(crate) fn text_button(ui:&mut egui::Ui,id:&str,label:&str,width:f32,selected:bool,enabled:bool)->egui::Response{
    let(r,fg)=control(ui,id,label,selected,enabled,Vec2::new(width,CONTROL_HEIGHT));
    ui.painter().with_clip_rect(r.rect.shrink2(Vec2::new(6.0,0.0))).text(r.rect.center(),egui::Align2::CENTER_CENTER,label,crate::typography::body(),fg);
    r.on_hover_text(label)
}
pub(crate) fn control(ui:&mut egui::Ui,id:&str,label:&str,selected:bool,enabled:bool,size:Vec2)->(egui::Response,Color32){
    let(rect,_)=ui.allocate_exact_size(size,egui::Sense::hover());
    let r=ui.interact(rect,ui.make_persistent_id(id),if enabled{egui::Sense::click()}else{egui::Sense::hover()});
    r.widget_info(||egui::WidgetInfo::selected(egui::WidgetType::Button,enabled,selected,label));
    // Selection and keyboard feedback are immediate; only pointer hover fades.
    let h=ui.ctx().animate_bool_with_time(r.id.with("hover"),enabled&&r.hovered(),ui.style().animation_time.min(0.12));
    let(fill,fg)=choice_colors(selected&&enabled,enabled,if selected&&enabled{1.0}else{0.0},h);
    ui.painter().rect_filled(rect,CONTROL_RADIUS,if r.is_pointer_button_down_on(){fill.gamma_multiply(0.9)}else{fill});
    if r.has_focus(){ui.painter().rect_stroke(rect,CONTROL_RADIUS,egui::Stroke::new(2.0,if selected&&enabled{ON_ACCENT}else{ACCENT}),egui::StrokeKind::Inside);}
    lightcraft_ui_egui::widgets::register(ui.ctx(),id,rect);
    (r,fg)
}

pub(crate) fn centered_row<R>(ui:&mut egui::Ui,height:f32,contents:impl FnOnce(&mut egui::Ui)->R)->egui::InnerResponse<R>{
    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(),height),egui::Layout::left_to_right(egui::Align::Center),contents)
}

pub(crate) fn surface_frame(margin:i8)->egui::Frame{
    egui::Frame::new().fill(PANEL)
        .corner_radius(SHELL_RADIUS).outer_margin(6).inner_margin(margin)
}


pub(crate) fn apply(ui:&mut egui::Ui,motion_enabled:bool){
        ui.ctx().style_mut_of(egui::Theme::Dark,|style|style.animation_time=if motion_enabled{0.15}else{0.0});
        let mut owned=(*ui.ctx().style_of(egui::Theme::Dark)).clone();
        let style=&mut owned;
        style.spacing.item_spacing=Vec2::new(10.0,10.0);style.spacing.button_padding=Vec2::new(12.0,7.0);
        crate::typography::apply(style);
        style.spacing.slider_width=150.0;style.animation_time=if motion_enabled{0.15}else{0.0};
        style.visuals=egui::Visuals::dark();
        style.visuals.indent_has_left_vline=false;
        style.visuals.window_corner_radius=SHELL_RADIUS.into();
        style.visuals.panel_fill=SURFACE;style.visuals.window_fill=PANEL;
        style.visuals.extreme_bg_color=SURFACE;
        style.visuals.faint_bg_color=CONTROL;
        style.visuals.hyperlink_color=ACCENT;
        style.visuals.error_fg_color=ERROR;style.visuals.warn_fg_color=WARNING;
        style.visuals.text_cursor.stroke.color=ACCENT;
        style.visuals.override_text_color=Some(INK);style.visuals.weak_text_color=Some(MUTED);
        style.visuals.selection.bg_fill=SELECTED;style.visuals.selection.stroke=egui::Stroke::new(1.0,INK);
        for w in [&mut style.visuals.widgets.noninteractive,&mut style.visuals.widgets.inactive,&mut style.visuals.widgets.hovered,&mut style.visuals.widgets.active,&mut style.visuals.widgets.open]{
            w.fg_stroke=egui::Stroke::new(1.0,INK);w.bg_stroke=egui::Stroke::NONE;
        }
        style.visuals.widgets.noninteractive.bg_fill=SURFACE;
        style.visuals.widgets.noninteractive.weak_bg_fill=SURFACE;
        style.visuals.widgets.inactive.bg_fill=CONTROL;style.visuals.widgets.inactive.weak_bg_fill=CONTROL;
        style.visuals.widgets.hovered.bg_fill=HOVER;style.visuals.widgets.hovered.weak_bg_fill=HOVER;
        style.visuals.widgets.active.bg_fill=SELECTED;style.visuals.widgets.active.weak_bg_fill=SELECTED;
        style.visuals.widgets.active.bg_stroke=egui::Stroke::new(1.0,ACCENT);
        style.visuals.widgets.open.bg_fill=SELECTED;style.visuals.widgets.open.weak_bg_fill=SELECTED;
        for w in [&mut style.visuals.widgets.noninteractive,&mut style.visuals.widgets.inactive,&mut style.visuals.widgets.hovered,&mut style.visuals.widgets.active,&mut style.visuals.widgets.open]{w.corner_radius=CONTROL_RADIUS.into();}
        style.visuals.window_shadow=egui::epaint::Shadow{offset:[0,4],blur:18,spread:0,color:Color32::from_black_alpha(70)};
        ui.ctx().set_style_of(egui::Theme::Dark,owned.clone());
        *ui.style_mut()=owned;
        ui.painter().rect_filled(ui.max_rect(),0,SURFACE);
}

/// Stable identity keeps hover/focus feedback attached to the action across pages.
pub(crate) fn tab(ui:&mut egui::Ui,id:&str,text:&str,selected:bool,enabled:bool)->egui::Response{choice(ui,id,text,selected,enabled,false)}
pub(crate) fn side_tab(ui:&mut egui::Ui,id:&str,text:&str,selected:bool)->egui::Response{choice(ui,id,text,selected,ui.is_enabled(),true)}
fn choice(ui:&mut egui::Ui,id:&str,text:&str,selected:bool,enabled:bool,full_width:bool)->egui::Response{
    let galley=ui.painter().layout_no_wrap(text.into(),crate::typography::body(),INK);
    let (rect,_)=ui.allocate_exact_size(Vec2::new(if full_width{ui.available_width()}else{galley.size().x+24.0},if full_width{36.0}else{34.0}),egui::Sense::hover());
    let r=ui.interact(rect,ui.make_persistent_id(id),if enabled{egui::Sense::click()}else{egui::Sense::hover()});
    r.widget_info(||egui::WidgetInfo::selected(egui::WidgetType::Button,enabled,selected,text));
    let t=ui.ctx().animate_bool_with_time(r.id.with("selected"),selected,ui.style().animation_time);
    let h=ui.ctx().animate_bool_with_time(r.id.with("hover"),enabled&&r.hovered(),ui.style().animation_time);
    let (fill,fg)=choice_colors(selected,enabled,t,h);
    ui.painter().rect_filled(rect,CONTROL_RADIUS,if r.is_pointer_button_down_on(){fill.gamma_multiply(0.88)}else{fill});
    ui.painter().with_clip_rect(rect.shrink2(Vec2::new(10.0,0.0))).text(if full_width{egui::pos2(rect.left()+12.0,rect.center().y)}else{rect.center()},if full_width{egui::Align2::LEFT_CENTER}else{egui::Align2::CENTER_CENTER},text,crate::typography::body(),fg);
    if r.has_focus(){ui.painter().rect_stroke(rect,CONTROL_RADIUS,egui::Stroke::new(2.0,if selected&&enabled{ON_ACCENT}else{ACCENT}),egui::StrokeKind::Inside);}
    lightcraft_ui_egui::widgets::register(ui.ctx(),id,rect);
    r.on_hover_text(text)
}

/// Animate within each contrast-safe palette, never through grey-on-grey inversions.
pub(crate) fn choice_colors(selected:bool,enabled:bool,selection:f32,hover:f32)->(Color32,Color32){
    if !enabled{return (CONTROL,MUTED);}
    let (from,to,t)=if selected{(Color32::from_rgb(177,227,174),ACCENT,selection)}else{(CONTROL,HOVER,hover.max(selection))};
    let fill=Color32::from(egui::lerp(egui::Rgba::from(from)..=egui::Rgba::from(to),t));
    (fill,if !enabled{MUTED}else if selected{ON_ACCENT}else{INK})
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn compact_controls_activate_from_the_keyboard(){
        let ctx=egui::Context::default();
        let output=ctx.run_ui(Default::default(),|ui|{let r=icon_button(ui,"test.control",lightcraft_ui_egui::icons::Icon::Plus,"放大",false,true);ui.memory_mut(|m|m.request_focus(r.id));});output.drop_without_applying_deltas();
        let mut clicked=false;
        let output=ctx.run_ui(egui::RawInput{events:vec![egui::Event::Key{key:egui::Key::Enter,physical_key:None,pressed:true,repeat:false,modifiers:Default::default()}],..Default::default()},|ui|{clicked=icon_button(ui,"test.control",lightcraft_ui_egui::icons::Icon::Plus,"放大",false,true).clicked();});output.drop_without_applying_deltas();
        assert!(clicked,"focused icon controls must respond to Enter");
    }
    fn luminance(c:Color32)->f64{let rgb=[c.r(),c.g(),c.b()].map(|v|{let s=v as f64/255.0;if s<=0.04045{s/12.92}else{((s+0.055)/1.055).powf(2.4)}});rgb[0]*0.2126+rgb[1]*0.7152+rgb[2]*0.0722}
    #[test]fn navigation_text_remains_readable_during_selection_and_hover(){
        for enabled in [false,true]{for selected in [false,true]{for step in 0..=20{let t=step as f32/20.0;let(bg,fg)=choice_colors(selected,enabled,t,1.0-t);let a=luminance(bg);let b=luminance(fg);assert!((a.max(b)+0.05)/(a.min(b)+0.05)>=4.5);}}}
        for bg in [SURFACE,PANEL,CONTROL,HOVER,SELECTED]{for fg in [INK,MUTED,ERROR,WARNING,ACCENT]{let a=luminance(bg);let b=luminance(fg);assert!((a.max(b)+0.05)/(a.min(b)+0.05)>=4.5,"semantic color is unreadable on {bg:?}");}}
    }
}
