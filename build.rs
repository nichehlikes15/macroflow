//! Compiles the small Swift OCR helper (helpers/ocr.swift) that "Read text" steps use.
//! If it can't be built, MacroFlow still builds; only Read text steps will report an error.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=helpers/ocr.swift");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let helper = out_dir.join("macroflow-ocr");

    if target_os != "macos" {
        println!("cargo:rustc-env=MACROFLOW_OCR_HELPER=");
        return;
    }

    let result = Command::new("xcrun")
        .args(["swiftc", "-O", "helpers/ocr.swift", "-o"])
        .arg(&helper)
        .output();

    match result {
        Ok(out) if out.status.success() && helper.exists() => {
            println!("cargo:rustc-env=MACROFLOW_OCR_HELPER={}", helper.display());
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            for line in err.lines().take(6) {
                println!("cargo:warning=OCR helper: {line}");
            }
            println!("cargo:warning=Couldn't build the OCR helper, so Read text steps won't work. Run `xcode-select --install` and rebuild.");
            println!("cargo:rustc-env=MACROFLOW_OCR_HELPER=");
        }
        Err(e) => {
            println!("cargo:warning=Couldn't run swiftc ({e}). Read text steps won't work. Run `xcode-select --install` and rebuild.");
            println!("cargo:rustc-env=MACROFLOW_OCR_HELPER=");
        }
    }
}
