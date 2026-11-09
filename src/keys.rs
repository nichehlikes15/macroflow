//! Key naming shared by the recorder, the editor and playback (macOS and Windows).
//!
//! Keys are stored in macros as short lowercase names ("a", "enter", "cmd", "f5", "num1", ...)
//! so saved macro files stay readable and work on both platforms.
//! "cmd" means ⌘ Command on macOS and the ⊞ Windows key on Windows;
//! "alt" means ⌥ Option on macOS and Alt on Windows.

use crate::model::Mods;
use device_query::Keycode;
use enigo::Key;

/// The key to send during playback for a key name.
pub fn to_enigo(name: &str) -> Option<Key> {
    // Single printable characters (letters, digits, punctuation) are sent as characters,
    // which works with the current keyboard layout on both platforms.
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        if !c.is_control() && c != ' ' {
            return Some(Key::Unicode(c));
        }
    }
    Some(match name {
        "enter" | "numenter" => Key::Return,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "escape" => Key::Escape,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "capslock" => Key::CapsLock,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        "f13" => Key::F13,
        "f14" => Key::F14,
        "f15" => Key::F15,
        "f16" => Key::F16,
        "f17" => Key::F17,
        "f18" => Key::F18,
        "f19" => Key::F19,
        "f20" => Key::F20,
        "num0" => Key::Numpad0,
        "num1" => Key::Numpad1,
        "num2" => Key::Numpad2,
        "num3" => Key::Numpad3,
        "num4" => Key::Numpad4,
        "num5" => Key::Numpad5,
        "num6" => Key::Numpad6,
        "num7" => Key::Numpad7,
        "num8" => Key::Numpad8,
        "num9" => Key::Numpad9,
        "num+" => Key::Add,
        "num-" => Key::Subtract,
        "num*" => Key::Multiply,
        "num/" => Key::Divide,
        "num." => Key::Decimal,
        "num=" => Key::Unicode('='),
        "shift" => Key::LShift,
        "rshift" => Key::RShift,
        "ctrl" => Key::LControl,
        "rctrl" => Key::RControl,
        "alt" => Key::Alt,
        "cmd" => Key::Meta,
        #[cfg(target_os = "macos")]
        "rcmd" => Key::RCommand,
        #[cfg(target_os = "macos")]
        "ralt" => Key::ROption,
        #[cfg(target_os = "macos")]
        "fn" => Key::Function,
        #[cfg(target_os = "macos")]
        "help" | "insert" => Key::Help,
        #[cfg(target_os = "windows")]
        "rcmd" => Key::RWin,
        #[cfg(target_os = "windows")]
        "ralt" => Key::RMenu,
        #[cfg(target_os = "windows")]
        "help" | "insert" => Key::Insert,
        _ => return None,
    })
}

/// Name for a key reported by the recorder.
pub fn from_device(k: Keycode) -> String {
    use Keycode as K;
    #[allow(unreachable_patterns)]
    let name = match k {
        K::Key0 => "0",
        K::Key1 => "1",
        K::Key2 => "2",
        K::Key3 => "3",
        K::Key4 => "4",
        K::Key5 => "5",
        K::Key6 => "6",
        K::Key7 => "7",
        K::Key8 => "8",
        K::Key9 => "9",
        K::A => "a",
        K::B => "b",
        K::C => "c",
        K::D => "d",
        K::E => "e",
        K::F => "f",
        K::G => "g",
        K::H => "h",
        K::I => "i",
        K::J => "j",
        K::K => "k",
        K::L => "l",
        K::M => "m",
        K::N => "n",
        K::O => "o",
        K::P => "p",
        K::Q => "q",
        K::R => "r",
        K::S => "s",
        K::T => "t",
        K::U => "u",
        K::V => "v",
        K::W => "w",
        K::X => "x",
        K::Y => "y",
        K::Z => "z",
        K::F1 => "f1",
        K::F2 => "f2",
        K::F3 => "f3",
        K::F4 => "f4",
        K::F5 => "f5",
        K::F6 => "f6",
        K::F7 => "f7",
        K::F8 => "f8",
        K::F9 => "f9",
        K::F10 => "f10",
        K::F11 => "f11",
        K::F12 => "f12",
        K::F13 => "f13",
        K::F14 => "f14",
        K::F15 => "f15",
        K::F16 => "f16",
        K::F17 => "f17",
        K::F18 => "f18",
        K::F19 => "f19",
        K::F20 => "f20",
        K::Escape => "escape",
        K::Space => "space",
        K::LControl => "ctrl",
        K::RControl => "rctrl",
        K::LShift => "shift",
        K::RShift => "rshift",
        K::LAlt => "alt",
        K::RAlt => "ralt",
        K::Command => "cmd",
        K::RCommand => "rcmd",
        K::LOption => "alt",
        K::ROption => "ralt",
        K::LMeta => "cmd",
        K::RMeta => "rcmd",
        K::Enter => "enter",
        K::Up => "up",
        K::Down => "down",
        K::Left => "left",
        K::Right => "right",
        K::Backspace => "backspace",
        K::CapsLock => "capslock",
        K::Tab => "tab",
        K::Home => "home",
        K::End => "end",
        K::PageUp => "pageup",
        K::PageDown => "pagedown",
        K::Insert => "insert",
        K::Delete => "delete",
        K::Numpad0 => "num0",
        K::Numpad1 => "num1",
        K::Numpad2 => "num2",
        K::Numpad3 => "num3",
        K::Numpad4 => "num4",
        K::Numpad5 => "num5",
        K::Numpad6 => "num6",
        K::Numpad7 => "num7",
        K::Numpad8 => "num8",
        K::Numpad9 => "num9",
        K::NumpadSubtract => "num-",
        K::NumpadAdd => "num+",
        K::NumpadDivide => "num/",
        K::NumpadMultiply => "num*",
        K::NumpadEquals => "num=",
        K::NumpadEnter => "numenter",
        K::NumpadDecimal => "num.",
        K::Grave => "`",
        K::Minus => "-",
        K::Equal => "=",
        K::LeftBracket => "[",
        K::RightBracket => "]",
        K::BackSlash => "\\",
        K::Semicolon => ";",
        K::Apostrophe => "'",
        K::Comma => ",",
        K::Dot => ".",
        K::Slash => "/",
        _ => return format!("{k:?}").to_lowercase(),
    };
    name.to_string()
}

/// Name for a key reported by GPUI when the user presses it inside the MacroFlow window.
pub fn from_gpui(key: &str) -> Option<String> {
    let lower = key.to_lowercase();
    let mapped = match lower.as_str() {
        "return" => "enter",
        "esc" => "escape",
        "pgup" => "pageup",
        "pgdown" => "pagedown",
        "!" => "1",
        "@" => "2",
        "#" => "3",
        "$" => "4",
        "%" => "5",
        "^" => "6",
        "&" => "7",
        "*" => "8",
        "(" => "9",
        ")" => "0",
        "_" => "-",
        "+" => "=",
        "{" => "[",
        "}" => "]",
        "|" => "\\",
        ":" => ";",
        "\"" => "'",
        "<" => ",",
        ">" => ".",
        "?" => "/",
        "~" => "`",
        "control" => "ctrl",
        "option" => "alt",
        "command" | "platform" | "super" | "win" | "windows" => "cmd",
        other => other,
    };
    to_enigo(mapped).map(|_| mapped.to_string())
}

pub fn is_modifier(name: &str) -> bool {
    matches!(
        name,
        "cmd" | "rcmd" | "shift" | "rshift" | "alt" | "ralt" | "ctrl" | "rctrl" | "fn"
    )
}

#[cfg(target_os = "macos")]
const NAMES: &[(&str, &str)] = &[
    ("cmd", "⌘ Command"),
    ("rcmd", "⌘ Right Command"),
    ("alt", "⌥ Option"),
    ("ralt", "⌥ Right Option"),
    ("ctrl", "⌃ Control"),
    ("rctrl", "⌃ Right Control"),
    ("shift", "⇧ Shift"),
    ("rshift", "⇧ Right Shift"),
    ("backspace", "⌫ Delete"),
    ("delete", "⌦ Forward Delete"),
    ("enter", "↩ Return"),
    ("insert", "Help"),
    ("help", "Help"),
];

#[cfg(not(target_os = "macos"))]
const NAMES: &[(&str, &str)] = &[
    ("cmd", "⊞ Win"),
    ("rcmd", "⊞ Right Win"),
    ("alt", "Alt"),
    ("ralt", "Right Alt"),
    ("ctrl", "Ctrl"),
    ("rctrl", "Right Ctrl"),
    ("shift", "Shift"),
    ("rshift", "Right Shift"),
    ("backspace", "Backspace"),
    ("delete", "Delete"),
    ("enter", "Enter"),
    ("insert", "Insert"),
    ("help", "Insert"),
];

/// Human friendly label for a key name.
pub fn label(name: &str) -> String {
    if let Some((_, l)) = NAMES.iter().find(|(n, _)| *n == name) {
        return l.to_string();
    }
    let fixed = match name {
        "fn" => "fn",
        "numenter" => "Keypad Enter",
        "tab" => "⇥ Tab",
        "space" => "Space",
        "escape" => "Esc",
        "up" => "↑ Up",
        "down" => "↓ Down",
        "left" => "← Left",
        "right" => "→ Right",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page Up",
        "pagedown" => "Page Down",
        "capslock" => "Caps Lock",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    if let Some(rest) = name.strip_prefix("num") {
        return format!("Keypad {rest}");
    }
    name.to_uppercase()
}

pub fn combo_label(key: &str, mods: &Mods) -> String {
    let m = mods.symbols();
    if m.is_empty() {
        label(key)
    } else if cfg!(target_os = "macos") {
        format!("{m} {}", label(key))
    } else {
        // Windows style: "Ctrl+Shift+S"
        format!("{m}{}", label(key))
    }
}

/// Keys for the modifiers in a combo, in press order.
pub fn modifier_keys(mods: &Mods) -> Vec<Key> {
    let mut v = Vec::new();
    if mods.ctrl {
        v.push(Key::LControl);
    }
    if mods.alt {
        v.push(Key::Alt);
    }
    if mods.shift {
        v.push(Key::LShift);
    }
    if mods.cmd {
        v.push(Key::Meta);
    }
    v
}
