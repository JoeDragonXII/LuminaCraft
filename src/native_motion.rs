//! Read-only native animation preference. No system settings are changed.
pub(super) fn enabled()->bool {
    if std::env::var_os("LUMINACRAFT_REDUCED_MOTION").is_some_and(|v|v=="1"){return false;}
    #[cfg(windows)]{
        #[link(name="user32")]
        unsafe extern "system" {fn SystemParametersInfoW(action:u32,param:u32,value:*mut std::ffi::c_void,flags:u32)->i32;}
        let mut enabled:i32=0;
        // SPI_GETCLIENTAREAANIMATION writes one BOOL; the stack value lives for the call.
        let ok=unsafe{SystemParametersInfoW(0x1042,0,(&mut enabled as *mut i32).cast(),0)};
        ok!=0 && enabled!=0
    }
    #[cfg(not(windows))]{true}
}
