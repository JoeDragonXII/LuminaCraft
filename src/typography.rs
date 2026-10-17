//! Native typography: optical-size-aware Latin, compact Chinese UI fallback,
//! real semibold headings, and tabular digits for editable values.
use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

pub(crate) const BODY: f32 = 13.0;
pub(crate) const CAPTION: f32 = 11.5;
pub(crate) fn body() -> FontId { FontId::proportional(BODY) }
pub(crate) fn semibold(size:f32) -> FontId { FontId::new(size,FontFamily::Name("UI Semibold".into())) }

fn definitions() -> FontDefinitions {
    let mut fonts=FontDefinitions::default();
    let directory=std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(||"C:/Windows".into())).join("Fonts");
    let mut load=|name:&str,file:&str,index:u32| {
        let Ok(bytes)=std::fs::read(directory.join(file))else{return false};
        let mut data=FontData::from_owned(bytes);data.index=index;
        fonts.font_data.insert(name.into(),data.into());true
    };
    let regular=load("ui-latin","SegUIVar.ttf",0);
    let semibold=load("ui-semibold","seguisb.ttf",0);
    let numbers=load("ui-numbers","segoeui.ttf",0);
    let chinese=load("ui-chinese","msyh.ttc",1);
    if regular {
        let data=std::sync::Arc::make_mut(fonts.font_data.get_mut("ui-latin").unwrap());
        data.tweak.coords=egui::epaint::text::VariationCoords::new([(b"wght",400.0),(b"opsz",13.0)]);
    }
    let defaults=fonts.families[&FontFamily::Proportional].clone();
    let family=|primary:Option<&str>,cjk:Option<&str>|primary.into_iter().chain(cjk).map(str::to_owned).chain(defaults.iter().cloned()).collect::<Vec<_>>();
    fonts.families.insert(FontFamily::Proportional,family(regular.then_some("ui-latin"),chinese.then_some("ui-chinese")));
    fonts.families.insert(FontFamily::Name("UI Semibold".into()),family(semibold.then_some("ui-semibold").or(regular.then_some("ui-latin")),chinese.then_some("ui-chinese")));
    fonts.families.insert(FontFamily::Name("UI Numbers".into()),family(numbers.then_some("ui-numbers"),chinese.then_some("ui-chinese")));
    fonts
}

pub(crate) fn install(ctx:&egui::Context) {
    static FONTS:std::sync::OnceLock<FontDefinitions>=std::sync::OnceLock::new();
    ctx.set_fonts(FONTS.get_or_init(definitions).clone());
}

pub(crate) fn apply(style:&mut egui::Style) {
    style.text_styles.insert(TextStyle::Body,body());
    style.text_styles.insert(TextStyle::Button,body());
    style.text_styles.insert(TextStyle::Small,FontId::proportional(CAPTION));
    style.text_styles.insert(TextStyle::Heading,semibold(17.0));
    style.text_styles.insert(TextStyle::Name("Value".into()),FontId::new(BODY,FontFamily::Name("UI Numbers".into())));
    style.drag_value_text_style=TextStyle::Name("Value".into());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_stacks_resolve_and_native_numeric_digits_are_tabular() {
        let ctx=egui::Context::default();install(&ctx);
        let output=ctx.run_ui(Default::default(),|ui| {
            apply(ui.style_mut());
            for (text,font) in [("LuminaCraft 照片库",body()),("修图工作台",semibold(15.0)),("6611.00",ui.style().text_styles[&TextStyle::Name("Value".into())].clone())] {
                let galley=ui.painter().layout_no_wrap(text.into(),font,egui::Color32::WHITE);
                assert!(galley.size().x>0.0 && galley.size().y<30.0,"font must fit native control rows");
            }
            if cfg!(windows) {
                let font=ui.style().text_styles[&TextStyle::Name("Value".into())].clone();
                let width=|text:&str|ui.painter().layout_no_wrap(text.into(),font.clone(),egui::Color32::WHITE).size().x;
                assert!((width("000000")-width("111111")).abs()<0.1,"parameter digits must not shift horizontally");
            }
        });output.drop_without_applying_deltas();
    }
}
