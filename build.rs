//! Builds the small native helpers MacroFlow uses on macOS:
//!   helpers/ocr.swift    -> macroflow-ocr     (Read text steps)
//!   helpers/select.swift -> macroflow-select  (the drag-a-box area selector)
//!   helpers/notify.swift -> macroflow-notify  (Pop-up steps)
//! If one can't be built, MacroFlow still builds; only that feature reports an error.
//! (On Windows the helpers are PowerShell scripts embedded into the app instead.)

use std::path::{Path, PathBuf};
use std::process::Command;

fn build_swift(src: &str, out_dir: &Path, name: &str, env_var: &str, what: &str) {
    println!("cargo:rerun-if-changed={src}");
    let out = out_dir.join(name);
    let result = Command::new("xcrun")
        .args(["swiftc", "-O", src, "-o"])
        .arg(&out)
        .output();
    match result {
        Ok(o) if o.status.success() && out.exists() => {
            println!("cargo:rustc-env={env_var}={}", out.display());
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            for line in err.lines().filter(|l| l.contains("error")).take(8) {
                println!("cargo:warning={name}: {line}");
            }
            println!("cargo:warning=Couldn't build {name}, so {what} won't work. Run `xcode-select --install` and rebuild.");
            println!("cargo:rustc-env={env_var}=");
        }
        Err(e) => {
            println!("cargo:warning=Couldn't run swiftc ({e}), so {what} won't work. Run `xcode-select --install` and rebuild.");
            println!("cargo:rustc-env={env_var}=");
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os != "macos" {
        println!("cargo:rustc-env=MACROFLOW_OCR_HELPER=");
        println!("cargo:rustc-env=MACROFLOW_SELECT_HELPER=");
        println!("cargo:rustc-env=MACROFLOW_NOTIFY_HELPER=");
        return;
    }
    build_swift("helpers/ocr.swift", &out_dir, "macroflow-ocr", "MACROFLOW_OCR_HELPER", "Read text steps");
    build_swift(
        "helpers/notify.swift",
        &out_dir,
        "macroflow-notify",
        "MACROFLOW_NOTIFY_HELPER",
        "pop-up steps",
    );
    build_swift(
        "helpers/select.swift",
        &out_dir,
        "macroflow-select",
        "MACROFLOW_SELECT_HELPER",
        "selecting an area on screen",
    );
}
