//! Small local GUI entry point. Resolve paths from this exe, never the caller's cwd.
#![cfg_attr(target_os="windows",windows_subsystem="windows")]

fn launch()->std::io::Result<()>{
    let exe=std::env::current_exe()?;
    let root=exe.parent().ok_or_else(||std::io::Error::other("无法确定程序目录"))?;
    let target=[root.join("dist/LuminaCraft/LuminaCraft.exe"),root.join("target/debug/luminacraft.exe")]
        .into_iter().find(|p|p.is_file()).ok_or_else(||std::io::Error::new(std::io::ErrorKind::NotFound,"未找到 LuminaCraft 主程序，请先运行 scripts/cargo.ps1 build。"))?;
    std::process::Command::new(target).current_dir(root).args(std::env::args_os().skip(1)).spawn()?;
    Ok(())
}

fn main(){
    if let Err(error)=launch(){
        #[cfg(target_os="windows")]
        {
            #[link(name="user32")]
            unsafe extern "system"{fn MessageBoxW(window:*mut std::ffi::c_void,text:*const u16,title:*const u16,flags:u32)->i32;}
            let message:Vec<u16>=format!("启动失败：{error}").encode_utf16().chain(Some(0)).collect();
            let title:Vec<u16>="LuminaCraft".encode_utf16().chain(Some(0)).collect();
            // Native error feedback remains visible even with no console attached.
            unsafe{MessageBoxW(std::ptr::null_mut(),message.as_ptr(),title.as_ptr(),0x10);}
        }
        #[cfg(not(target_os="windows"))]
        eprintln!("启动失败：{error}");
        std::process::exit(1);
    }
}
