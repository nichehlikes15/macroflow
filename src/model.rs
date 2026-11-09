//! The macro document: a graph of steps ("nodes") joined by connections ("edges").

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const NODE_W: f32 = 214.0;
pub const NODE_H: f32 = 78.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseBtn {
    #[default]
    Left,
    Right,
    Middle,
}

impl MouseBtn {
    pub fn label(self) -> &'static str {
        match self {
            MouseBtn::Left => "Left",
            MouseBtn::Right => "Right",
            MouseBtn::Middle => "Middle",
        }
    }
}

/// Where a mouse step acts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// A fixed screen position.
    #[default]
    Fixed,
    /// Wherever the most recent "Find image" step found its image (plus an offset).
    FoundImage,
    /// Wherever the mouse already is.
    Current,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mods {
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub ctrl: bool,
}

impl Mods {
    #[cfg(target_os = "macos")]
    pub fn symbols(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push('⌃');
        }
        if self.alt {
            s.push('⌥');
        }
        if self.shift {
            s.push('⇧');
        }
        if self.cmd {
            s.push('⌘');
        }
        s
    }

    #[cfg(not(target_os = "macos"))]
    pub fn symbols(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.cmd {
            parts.push("Win");
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{}+", parts.join("+"))
        }
    }
}

/// How a "Read text" step compares what it reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextCompare {
    #[default]
    Contains,
    Equals,
    StartsWith,
    Regex,
    NotEmpty,
    Changed,
    NumEq,
    NumNe,
    NumGt,
    NumGe,
    NumLt,
    NumLe,
}

impl TextCompare {
    pub const TEXT: [TextCompare; 6] = [
        TextCompare::Contains,
        TextCompare::Equals,
        TextCompare::StartsWith,
        TextCompare::Regex,
        TextCompare::NotEmpty,
        TextCompare::Changed,
    ];
    pub const NUMBER: [TextCompare; 6] = [
        TextCompare::NumEq,
        TextCompare::NumNe,
        TextCompare::NumGt,
        TextCompare::NumGe,
        TextCompare::NumLt,
        TextCompare::NumLe,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TextCompare::Contains => "contains",
            TextCompare::Equals => "is exactly",
            TextCompare::StartsWith => "starts with",
            TextCompare::Regex => "matches regex",
            TextCompare::NotEmpty => "has any text",
            TextCompare::Changed => "has changed",
            TextCompare::NumEq => "=",
            TextCompare::NumNe => "≠",
            TextCompare::NumGt => ">",
            TextCompare::NumGe => "≥",
            TextCompare::NumLt => "<",
            TextCompare::NumLe => "≤",
        }
    }

    pub fn is_number(self) -> bool {
        TextCompare::NUMBER.contains(&self)
    }

    pub fn needs_value(self) -> bool {
        !matches!(self, TextCompare::NotEmpty | TextCompare::Changed)
    }
}

/// Which window(s) a "Window check" step looks at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinScope {
    /// Only the window you're currently using (the front window of the active app).
    #[default]
    Frontmost,
    /// Any window that's open on screen.
    AnyOpen,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinField {
    #[default]
    Title,
    App,
    Either,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinMatch {
    #[default]
    Contains,
    Equals,
    StartsWith,
}

impl WinMatch {
    pub fn label(self) -> &'static str {
        match self {
            WinMatch::Contains => "contains",
            WinMatch::Equals => "is exactly",
            WinMatch::StartsWith => "starts with",
        }
    }
}

/// One raw input event inside a recording.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RecAction {
    Move { x: i32, y: i32 },
    Down { button: MouseBtn, x: i32, y: i32 },
    Up { button: MouseBtn, x: i32, y: i32 },
    KeyDown { key: String },
    KeyUp { key: String },
}

impl RecAction {
    pub fn describe(&self) -> String {
        match self {
            RecAction::Move { x, y } => format!("Move to ({x}, {y})"),
            RecAction::Down { button, x, y } => format!("{} press at ({x}, {y})", button.label()),
            RecAction::Up { button, x, y } => format!("{} release at ({x}, {y})", button.label()),
            RecAction::KeyDown { key } => format!("Key down {}", crate::keys::label(key)),
            RecAction::KeyUp { key } => format!("Key up {}", crate::keys::label(key)),
        }
    }

    pub fn is_move(&self) -> bool {
        matches!(self, RecAction::Move { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecEvent {
    /// Time to wait before this event, in milliseconds.
    pub delay_ms: u64,
    pub action: RecAction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum NodeKind {
    Start,
    Click {
        target: Target,
        x: i32,
        y: i32,
        offset_x: i32,
        offset_y: i32,
        button: MouseBtn,
        clicks: u32,
    },
    MoveMouse {
        target: Target,
        x: i32,
        y: i32,
        offset_x: i32,
        offset_y: i32,
    },
    KeyPress {
        key: String,
        mods: Mods,
    },
    TypeText {
        text: String,
        press_enter: bool,
    },
    Wait {
        ms: u64,
        jitter_ms: u64,
    },
    Scroll {
        amount: i32,
        horizontal: bool,
    },
    Recording {
        events: Vec<RecEvent>,
        speed: f32,
    },
    FindImage {
        template: Option<PathBuf>,
        threshold: f32,
        timeout_ms: u64,
        interval_ms: u64,
        click_when_found: bool,
    },
    Loop {
        count: u32,
    },
    WindowCheck {
        scope: WinScope,
        field: WinField,
        mode: WinMatch,
        text: String,
        case_sensitive: bool,
        timeout_ms: u64,
        interval_ms: u64,
        bring_to_front: bool,
    },
    ReadText {
        /// Screen area in global points.
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        compare: TextCompare,
        value: String,
        case_sensitive: bool,
        timeout_ms: u64,
        interval_ms: u64,
        #[serde(default)]
        preview: Option<PathBuf>,
    },
    End,
}

impl NodeKind {
    pub fn palette() -> Vec<NodeKind> {
        vec![
            NodeKind::Click {
                target: Target::Fixed,
                x: 500,
                y: 400,
                offset_x: 0,
                offset_y: 0,
                button: MouseBtn::Left,
                clicks: 1,
            },
            NodeKind::MoveMouse {
                target: Target::Fixed,
                x: 500,
                y: 400,
                offset_x: 0,
                offset_y: 0,
            },
            NodeKind::KeyPress {
                key: "enter".into(),
                mods: Mods::default(),
            },
            NodeKind::TypeText {
                text: "Hello!".into(),
                press_enter: false,
            },
            NodeKind::Wait {
                ms: 500,
                jitter_ms: 0,
            },
            NodeKind::Scroll {
                amount: 3,
                horizontal: false,
            },
            NodeKind::FindImage {
                template: None,
                threshold: 0.85,
                timeout_ms: 5000,
                interval_ms: 250,
                click_when_found: false,
            },
            NodeKind::WindowCheck {
                scope: WinScope::Frontmost,
                field: WinField::Title,
                mode: WinMatch::Contains,
                text: String::new(),
                case_sensitive: false,
                timeout_ms: 0,
                interval_ms: 250,
                bring_to_front: false,
            },
            NodeKind::ReadText {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                compare: TextCompare::Contains,
                value: String::new(),
                case_sensitive: false,
                timeout_ms: 0,
                interval_ms: 500,
                preview: None,
            },
            NodeKind::Loop { count: 3 },
            NodeKind::Recording {
                events: vec![],
                speed: 1.0,
            },
            NodeKind::End,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            NodeKind::Start => "Start",
            NodeKind::Click { .. } => "Click",
            NodeKind::MoveMouse { .. } => "Move mouse",
            NodeKind::KeyPress { .. } => "Key press",
            NodeKind::TypeText { .. } => "Type text",
            NodeKind::Wait { .. } => "Wait",
            NodeKind::Scroll { .. } => "Scroll",
            NodeKind::Recording { .. } => "Recording",
            NodeKind::FindImage { .. } => "Find image",
            NodeKind::Loop { .. } => "Loop",
            NodeKind::WindowCheck { .. } => "Window check",
            NodeKind::ReadText { .. } => "Read text",
            NodeKind::End => "End",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            NodeKind::Start => "▶",
            NodeKind::Click { .. } => "◉",
            NodeKind::MoveMouse { .. } => "➜",
            NodeKind::KeyPress { .. } => "⌘",
            NodeKind::TypeText { .. } => "T",
            NodeKind::Wait { .. } => "◷",
            NodeKind::Scroll { .. } => "⇵",
            NodeKind::Recording { .. } => "●",
            NodeKind::FindImage { .. } => "◎",
            NodeKind::Loop { .. } => "↻",
            NodeKind::WindowCheck { .. } => "▣",
            NodeKind::ReadText { .. } => "Aa",
            NodeKind::End => "■",
        }
    }

    pub fn color(&self) -> u32 {
        match self {
            NodeKind::Start => 0x34c77b,
            NodeKind::Click { .. } => 0x4c8dff,
            NodeKind::MoveMouse { .. } => 0x2bb3c0,
            NodeKind::KeyPress { .. } => 0xff9f43,
            NodeKind::TypeText { .. } => 0xf368e0,
            NodeKind::Wait { .. } => 0x9aa0b4,
            NodeKind::Scroll { .. } => 0x54a0ff,
            NodeKind::Recording { .. } => 0xff5e57,
            NodeKind::FindImage { .. } => 0xa55eea,
            NodeKind::Loop { .. } => 0xfeca57,
            NodeKind::WindowCheck { .. } => 0x1dd1a1,
            NodeKind::ReadText { .. } => 0xff7eb6,
            NodeKind::End => 0x7f8796,
        }
    }

    /// Names of this step's outputs. Output 0 is the normal "next" path.
    pub fn outputs(&self) -> &'static [&'static str] {
        match self {
            NodeKind::End => &[],
            NodeKind::FindImage { .. } => &["found", "not found"],
            NodeKind::Loop { .. } => &["repeat", "done"],
            NodeKind::WindowCheck { .. } => &["yes", "no"],
            NodeKind::ReadText { .. } => &["match", "no match"],
            _ => &["next"],
        }
    }

    pub fn has_input(&self) -> bool {
        !matches!(self, NodeKind::Start)
    }

    pub fn summary(&self) -> String {
        fn place(target: &Target, x: i32, y: i32, ox: i32, oy: i32) -> String {
            match target {
                Target::Fixed => format!("at ({x}, {y})"),
                Target::FoundImage if ox == 0 && oy == 0 => "on found image".into(),
                Target::FoundImage => format!("on found image {ox:+}, {oy:+}"),
                Target::Current => "at current position".into(),
            }
        }
        match self {
            NodeKind::Start => "The macro begins here".into(),
            NodeKind::Click {
                target,
                x,
                y,
                offset_x,
                offset_y,
                button,
                clicks,
            } => {
                let times = if *clicks > 1 {
                    format!(" ×{clicks}")
                } else {
                    String::new()
                };
                format!(
                    "{}{times} {}",
                    button.label(),
                    place(target, *x, *y, *offset_x, *offset_y)
                )
            }
            NodeKind::MoveMouse {
                target,
                x,
                y,
                offset_x,
                offset_y,
            } => format!("Move {}", place(target, *x, *y, *offset_x, *offset_y)),
            NodeKind::KeyPress { key, mods } => crate::keys::combo_label(key, mods),
            NodeKind::TypeText { text, press_enter } => {
                let short: String = text.chars().take(28).collect();
                let more = if text.chars().count() > 28 { "…" } else { "" };
                let enter = if *press_enter { " ↩" } else { "" };
                format!("“{short}{more}”{enter}")
            }
            NodeKind::Wait { ms, jitter_ms } => {
                if *jitter_ms > 0 {
                    format!("{ms} ms (+ up to {jitter_ms} ms random)")
                } else {
                    format!("{ms} ms")
                }
            }
            NodeKind::Scroll { amount, horizontal } => {
                let dir = if *horizontal { "sideways" } else { "vertically" };
                format!("{amount} lines {dir}")
            }
            NodeKind::Recording { events, speed } => {
                let total: u64 = events.iter().map(|e| e.delay_ms).sum();
                let secs = total as f32 / 1000.0 / speed.max(0.01);
                format!("{} events · {secs:.1}s · {speed:.2}×", events.len())
            }
            NodeKind::FindImage {
                template,
                threshold,
                timeout_ms,
                ..
            } => {
                let name = template
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "no image yet".into());
                format!("{name} · ≥{:.0}% · {timeout_ms} ms", threshold * 100.0)
            }
            NodeKind::Loop { count } => {
                if *count == 0 {
                    "Repeat forever".into()
                } else {
                    format!("Repeat {count}×, then continue")
                }
            }
            NodeKind::WindowCheck {
                scope,
                field,
                mode,
                text,
                timeout_ms,
                ..
            } => {
                let which = match scope {
                    WinScope::Frontmost => "Current window",
                    WinScope::AnyOpen => "Any window",
                };
                let what = match field {
                    WinField::Title => "title",
                    WinField::App => "app",
                    WinField::Either => "title/app",
                };
                let wait = if *timeout_ms > 0 {
                    format!(" · wait {timeout_ms} ms")
                } else {
                    String::new()
                };
                if text.is_empty() {
                    format!("{which} {what} – set the text")
                } else {
                    format!("{which} {what} {} “{text}”{wait}", mode.label())
                }
            }
            NodeKind::ReadText {
                w, h, compare, value, ..
            } => {
                if *w <= 0 || *h <= 0 {
                    return "Select an area on screen".into();
                }
                let what = if compare.is_number() { "Number" } else { "Text" };
                if compare.needs_value() {
                    format!("{what} {} “{value}”", compare.label())
                } else {
                    format!("{what} {}", compare.label())
                }
            }
            NodeKind::End => "Stop the macro".into(),
        }
    }

    pub fn scale_recording(&mut self, factor: f32) {
        if let NodeKind::Recording { events, .. } = self {
            for e in events.iter_mut() {
                e.delay_ms = (e.delay_ms as f32 * factor).round() as u64;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: u64,
    pub kind: NodeKind,
    pub x: f32,
    pub y: f32,
    /// Extra wait before this step runs.
    #[serde(default)]
    pub delay_ms: u64,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: u64,
    pub port: usize,
    pub to: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Macro {
    pub name: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub next_id: u64,
    /// Wait after pressing Run, so you can switch to the target app.
    #[serde(default = "default_start_delay")]
    pub start_delay_ms: u64,
    /// Wait between every step.
    #[serde(default = "default_step_delay")]
    pub step_delay_ms: u64,
    /// Playback speed in percent (200 = twice as fast). Scales every delay.
    #[serde(default = "default_speed")]
    pub speed_pct: u32,
    /// How many times to run the whole macro (0 = until stopped).
    #[serde(default = "default_repeat")]
    pub repeat: u32,
}

fn default_start_delay() -> u64 {
    1500
}
fn default_step_delay() -> u64 {
    40
}
fn default_speed() -> u32 {
    100
}
fn default_repeat() -> u32 {
    1
}

impl Default for Macro {
    fn default() -> Self {
        let mut m = Macro {
            name: "Untitled macro".into(),
            nodes: vec![],
            edges: vec![],
            next_id: 1,
            start_delay_ms: default_start_delay(),
            step_delay_ms: default_step_delay(),
            speed_pct: default_speed(),
            repeat: default_repeat(),
        };
        m.add_node(NodeKind::Start, 60.0, 180.0);
        m
    }
}

impl Macro {
    pub fn node(&self, id: u64) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u64) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn add_node(&mut self, kind: NodeKind, x: f32, y: f32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(Node {
            id,
            kind,
            x,
            y,
            delay_ms: 0,
            note: String::new(),
        });
        id
    }

    pub fn remove_node(&mut self, id: u64) {
        self.nodes.retain(|n| n.id != id);
        self.edges.retain(|e| e.from != id && e.to != id);
    }

    /// Connect an output to a step. Each output has at most one connection.
    pub fn connect(&mut self, from: u64, port: usize, to: u64) {
        self.disconnect(from, port);
        self.edges.push(Edge { from, port, to });
    }

    pub fn disconnect(&mut self, from: u64, port: usize) {
        self.edges.retain(|e| !(e.from == from && e.port == port));
    }

    pub fn next(&self, from: u64, port: usize) -> Option<u64> {
        self.edges
            .iter()
            .find(|e| e.from == from && e.port == port)
            .map(|e| e.to)
    }

    pub fn start_id(&self) -> Option<u64> {
        self.nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Start))
            .map(|n| n.id)
    }

    /// Multiply every delay in the macro (step delays, waits and recordings).
    pub fn scale_delays(&mut self, factor: f32) {
        let f = |v: u64| (v as f32 * factor).round() as u64;
        for n in self.nodes.iter_mut() {
            n.delay_ms = f(n.delay_ms);
            if let NodeKind::Wait { ms, jitter_ms } = &mut n.kind {
                *ms = f(*ms);
                *jitter_ms = f(*jitter_ms);
            }
            n.kind.scale_recording(factor);
        }
        self.step_delay_ms = f(self.step_delay_ms);
    }

    /// Set every recorded delay and every per-step delay to one value.
    /// Wait steps are left alone, since those are usually deliberate.
    pub fn set_all_delays(&mut self, ms: u64) {
        for n in self.nodes.iter_mut() {
            n.delay_ms = ms;
            if let NodeKind::Recording { events, .. } = &mut n.kind {
                for e in events.iter_mut() {
                    e.delay_ms = ms;
                }
            }
        }
    }
}

/// Where MacroFlow keeps macros and snipped images by default.
pub fn app_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Documents").join("MacroFlow")
}

pub fn images_dir() -> PathBuf {
    app_dir().join("images")
}

pub fn out_port_pos(n: &Node, port: usize) -> (f32, f32) {
    let count = n.kind.outputs().len().max(1);
    (n.x + NODE_W, n.y + port_offset(count, port))
}

pub fn in_port_pos(n: &Node) -> (f32, f32) {
    (n.x, n.y + NODE_H / 2.0)
}

/// Vertical position of output `i` of `count`, measured from the node's top.
pub fn port_offset(count: usize, i: usize) -> f32 {
    NODE_H * (i as f32 + 1.0) / (count as f32 + 1.0)
}

// ---------------------------------------------------------------------------
// Comparisons used by Window check and Read text
// ---------------------------------------------------------------------------

pub fn win_matches(hay: &str, needle: &str, mode: WinMatch, case_sensitive: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    let (h, n) = if case_sensitive {
        (hay.trim().to_string(), needle.trim().to_string())
    } else {
        (hay.trim().to_lowercase(), needle.trim().to_lowercase())
    };
    match mode {
        WinMatch::Contains => h.contains(&n),
        WinMatch::Equals => h == n,
        WinMatch::StartsWith => h.starts_with(&n),
    }
}

/// Pull the first number out of some text: "Score: 1,234.5 pts" -> 1234.5, "-£12" -> -12.
pub fn parse_number(text: &str) -> Option<f64> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let mut start = i;
            // Look back past a currency symbol / space for a minus sign.
            let mut j = i;
            while j > 0 && matches!(chars[j - 1], '£' | '$' | '€' | '¥' | ' ') {
                j -= 1;
            }
            let negative = j > 0 && matches!(chars[j - 1], '-' | '−');
            let mut s = String::new();
            let mut seen_dot = false;
            while start < chars.len() {
                let c = chars[start];
                if c.is_ascii_digit() {
                    s.push(c);
                } else if c == ',' && start + 1 < chars.len() && chars[start + 1].is_ascii_digit() {
                    // thousands separator – skip
                } else if c == '.' && !seen_dot && start + 1 < chars.len() && chars[start + 1].is_ascii_digit() {
                    seen_dot = true;
                    s.push('.');
                } else {
                    break;
                }
                start += 1;
            }
            return s.parse::<f64>().ok().map(|v| if negative { -v } else { v });
        }
        i += 1;
    }
    None
}

/// Does the text read from the screen pass the step's test?
pub fn text_matches(
    read: &str,
    compare: TextCompare,
    value: &str,
    case_sensitive: bool,
    previous: Option<&str>,
) -> Result<bool, String> {
    let norm = |t: &str| {
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if case_sensitive {
            t
        } else {
            t.to_lowercase()
        }
    };
    let (r, v) = (norm(read), norm(value));
    if compare.is_number() {
        let target: f64 = value
            .trim()
            .replace(',', "")
            .parse()
            .map_err(|_| format!("“{value}” isn't a number to compare with."))?;
        let Some(n) = parse_number(read) else {
            return Ok(false);
        };
        let eps = 1e-9;
        return Ok(match compare {
            TextCompare::NumEq => (n - target).abs() < eps,
            TextCompare::NumNe => (n - target).abs() >= eps,
            TextCompare::NumGt => n > target,
            TextCompare::NumGe => n >= target - eps,
            TextCompare::NumLt => n < target,
            TextCompare::NumLe => n <= target + eps,
            _ => false,
        });
    }
    Ok(match compare {
        TextCompare::Contains => !v.is_empty() && r.contains(&v),
        TextCompare::Equals => r == v,
        TextCompare::StartsWith => !v.is_empty() && r.starts_with(&v),
        TextCompare::Regex => {
            let pattern = if case_sensitive {
                value.to_string()
            } else {
                format!("(?i){value}")
            };
            let re = regex::Regex::new(&pattern).map_err(|e| format!("That regex isn't valid: {e}"))?;
            re.is_match(read)
        }
        TextCompare::NotEmpty => !r.is_empty(),
        TextCompare::Changed => match previous {
            Some(p) => norm(p) != r,
            None => false,
        },
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_number("Score: 1,234.5 pts"), Some(1234.5));
        assert_eq!(parse_number("-£12"), Some(-12.0));
        assert_eq!(parse_number("Level 7."), Some(7.0));
        assert_eq!(parse_number("none"), None);
    }

    #[test]
    fn compares() {
        assert!(text_matches("Gold: 1,500", TextCompare::NumGe, "1000", false, None).unwrap());
        assert!(!text_matches("Gold: 900", TextCompare::NumGe, "1000", false, None).unwrap());
        assert!(text_matches("Upload COMPLETE", TextCompare::Contains, "complete", false, None).unwrap());
        assert!(text_matches("abc", TextCompare::Changed, "", false, Some("abd")).unwrap());
        assert!(text_matches("Order #4521", TextCompare::Regex, r"#\d{4}", false, None).unwrap());
        assert!(win_matches("Inbox – Mail", "inbox", WinMatch::StartsWith, false));
    }
}
