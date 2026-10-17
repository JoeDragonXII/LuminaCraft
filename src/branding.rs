//! One embedded artwork for the header, primary window and advanced window.
pub fn icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/app-icon/icon-256.png"))
        .expect("bundled LuminaCraft icon must be a valid PNG")
        .into_rgba8();
    egui::IconData { width: image.width(), height: image.height(), rgba: image.into_raw() }
}

pub fn texture(ctx: &egui::Context) -> egui::TextureHandle {
    let icon = icon();
    ctx.load_texture("luminacraft-brand", egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize], &icon.rgba), egui::TextureOptions::LINEAR)
}
