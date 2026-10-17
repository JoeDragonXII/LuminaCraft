//! Host shortcut policy; Ctrl+Q belongs to the user's screenshot application.
use eframe::egui;

pub(crate) fn configure(ctx:&egui::Context){
    #[cfg(target_os="windows")]
    ctx.options_mut(|o|o.quit_shortcuts.retain(|sc|!capture_key(sc.modifiers,sc.logical_key)));
    #[cfg(not(target_os="windows"))]
    let _=ctx;
}

pub(crate) fn configure_advanced(app:&mut lightcraft_ui_egui::LightcraftApp){
    #[cfg(target_os="windows")]
    if lightcraft_ui_egui::shortcuts::shortcut_of(&app.ui.settings.keymap,"app.quit")
        .and_then(lightcraft_ui_egui::shortcuts::parse).is_some_and(|(m,k)|capture_key(m,k)){
        // Also remove the in-window menu accelerator, after loading preferences.
        app.ui.settings.keymap.insert("app.quit".into(),String::new());
    }
    #[cfg(not(target_os="windows"))]
    let _=app;
}

#[cfg(target_os="windows")]
fn capture_key(m:egui::Modifiers,k:egui::Key)->bool{
    k==egui::Key::Q&&(m==egui::Modifiers::COMMAND||m==egui::Modifiers::CTRL)
}

#[cfg(all(test,target_os="windows"))]
mod tests{
    use super::*;
    #[test]fn screenshot_key_does_not_quit_either_host(){
        let ctx=egui::Context::default();configure(&ctx);
        let mut app=lightcraft_ui_egui::LightcraftApp::new(lightcraft_engine::Session::new(),Default::default());
        configure_advanced(&mut app);
        assert!(lightcraft_ui_egui::shortcuts::shortcut_of(&app.ui.settings.keymap,"app.quit").is_none());
        let m=egui::Modifiers{ctrl:true,command:true,..Default::default()};
        let input=egui::RawInput{events:vec![egui::Event::ModifiersChanged(m),egui::Event::Key{key:egui::Key::Q,physical_key:Some(egui::Key::Q),pressed:true,repeat:false,modifiers:m}],..Default::default()};
        let output=ctx.run_ui(input,|ui|lightcraft_ui_egui::shortcuts::handle(&mut app,ui.ctx()));
        assert!(!app.ui.quit);
        assert!(!output.viewport_output.values().flat_map(|v|&v.commands).any(|c|matches!(c,egui::ViewportCommand::Close)));
        output.drop_without_applying_deltas();
        let output=ctx.run_ui(Default::default(),|ui|ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close));
        assert!(output.viewport_output.values().flat_map(|v|&v.commands).any(|c|matches!(c,egui::ViewportCommand::Close)));
        output.drop_without_applying_deltas();
    }
}
