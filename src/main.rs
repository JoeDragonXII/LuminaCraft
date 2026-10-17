#![cfg_attr(target_os="windows",windows_subsystem="windows")]

mod app;
mod chrome;
mod typography;
mod branding;
mod keyboard;
mod library_ui;
mod upstream_desktop;
fn main() {
    let args:Vec<String>=std::env::args().skip(1).collect();
    if args.first().is_some_and(|s|s=="--version"){println!("luminacraft {}",env!("CARGO_PKG_VERSION"));return;}
    if args.first().is_some_and(|s|s=="--probe") {
        let result=args.get(1).ok_or_else(||anyhow::anyhow!("需要原片路径。")).and_then(|s|luminacraft::engine::probe(std::path::Path::new(s)));
        match result{Ok(v)=>println!("{v}"),Err(e)=>{eprintln!("{e:#}");std::process::exit(1);}}return;
    }
    if args.first().is_some_and(|s|s=="--advanced") {
        // Re-exec with isolated engine settings before any GUI or worker threads start.
        if std::env::var_os("LUMINACRAFT_DESKTOP_CHILD").is_none() {
            let base=std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir).join("LuminaCraft");
            let library=std::env::var_os("LUMINACRAFT_LIBRARY").map(std::path::PathBuf::from).unwrap_or_else(||base.join("Library"));
            let config=std::env::var_os("LUMINACRAFT_CONFIG_DIR").map(std::path::PathBuf::from).unwrap_or_else(||base.join("Settings"));
            let result=std::env::current_exe().and_then(|exe| {
                let mut command=std::process::Command::new(exe);
                command.args(&args).env("LUMINACRAFT_DESKTOP_CHILD","1")
                    .env("LIGHTCRAFT_LIBRARY",library).env("LIGHTCRAFT_CONFIG_DIR",config)
                    .env("LIGHTCRAFT_LANGUAGE",std::env::var("LUMINACRAFT_LANGUAGE").unwrap_or_else(|_|"zh-hans".into()));
                if std::env::var_os("LUMINACRAFT_LIBRARY").is_none() { command.env("LUMINACRAFT_LIBRARY_DEFAULT","1"); }
                command.status()
            });
            match result { Ok(status)=>std::process::exit(status.code().unwrap_or(1)),Err(e)=>{startup_error(&e.to_string());std::process::exit(1);} }
        }
        if let Err(e)=upstream_desktop::run(){startup_error(&e.to_string());std::process::exit(1);}
        return;
    }
    let options=eframe::NativeOptions{viewport:eframe::egui::ViewportBuilder::default().with_icon(branding::icon()).with_inner_size([1380.0,900.0]).with_min_inner_size([980.0,680.0]),renderer:eframe::Renderer::Glow,..Default::default()};
    let input=if args.first().is_some_and(|s|s=="--workflow"){args.get(1)}else{args.first()}.map(std::path::PathBuf::from);
    if let Err(e)=eframe::run_native("LuminaCraft",options,Box::new(move|cc|Ok(Box::new(app::StudioApp::new(cc,input))))){startup_error(&e.to_string());std::process::exit(1);}
}

fn startup_error(message:&str){
    eprintln!("启动失败：{message}");
    rfd::MessageDialog::new().set_title("LuminaCraft 启动失败").set_description(message).set_level(rfd::MessageLevel::Error).show();
}
