//! Pictures built into MacroFlow: the step icons (drawn as SVG so they look the
//! same everywhere and say what the step does) and the app icon.

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

const ICONS: &[(&str, &[u8])] = icons![
    "bell", "click", "disabled", "drag", "end", "find", "gone", "group", "group_in", "group_out", "if",
    "info", "key", "loop", "move", "pixel", "read", "record", "scroll", "start", "trigger", "type",
    "variable", "wait", "webhook", "window",
];

/// The app icon (512 × 512 PNG), for the Dock on macOS. Windows gets the .ico
/// built into the .exe instead (see build.rs).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().find(|(p, _)| *p == path).map(|(_, data)| Cow::Borrowed(*data)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

/// Show MacroFlow's icon in the Dock (a plain binary has no app bundle to take it from).
#[cfg(target_os = "macos")]
// objc 0.2's macros check a `cargo-clippy` feature that newer Rust warns about.
#[allow(unexpected_cfgs)]
pub fn set_dock_icon() {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: plain AppKit calls on the main thread (GPUI's run callback), with
    // objects that are either autoreleased or kept by NSApplication.
    unsafe {
        let data: *mut Object = msg_send![class!(NSData), dataWithBytes: APP_ICON_PNG.as_ptr() length: APP_ICON_PNG.len()];
        if data.is_null() {
            return;
        }
        let image: *mut Object = msg_send![class!(NSImage), alloc];
        let image: *mut Object = msg_send![image, initWithData: data];
        if image.is_null() {
            return;
        }
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setApplicationIconImage: image];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_icon_exists() {
        use crate::model::NodeKind;
        for k in NodeKind::palette().iter().chain([NodeKind::Start, NodeKind::GroupIn, NodeKind::GroupOut].iter()) {
            assert!(Assets.load(k.icon()).unwrap().is_some(), "missing {}", k.icon());
        }
        assert!(Assets.load("icons/disabled.svg").unwrap().is_some());
        assert!(Assets.load("icons/info.svg").unwrap().is_some());
    }
}
