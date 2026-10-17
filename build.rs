use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon/app.rc");
    println!("cargo:rerun-if-changed=assets/app-icon/luminacraft.ico");
    println!("cargo:rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("app-icon.o");
    // The workspace wrapper supplies the GNU Windows toolchain on PATH.
    let rc = env::var_os("RC").unwrap_or_else(|| "windres".into());
    let status = Command::new(rc)
        .args(["-I", "assets/app-icon", "-i", "assets/app-icon/app.rc", "-O", "coff", "-o"])
        .arg(&output)
        .status()
        .expect("Windows icon compiler unavailable; build with scripts/cargo.ps1");
    assert!(status.success(), "Windows icon resource compilation failed");
    for binary in ["luminacraft", "luminacraft-launcher"] {
        println!("cargo:rustc-link-arg-bin={binary}={}", output.display());
    }
}
