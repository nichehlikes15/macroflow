//! The macro document: a graph of steps ("nodes") joined by connections ("edges").

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub const NODE_W: f32 = 214.0;
pub const GROUP_COLOR: u32 = 0x6c8cff;
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
    /// A random spot inside an area you drag a box around.
    Area,
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

/// An area of the screen, in the same coordinates the mouse uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The look of a pop-up notification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeStyle {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl NoticeStyle {
    pub const ALL: [NoticeStyle; 4] = [
        NoticeStyle::Info,
        NoticeStyle::Success,
        NoticeStyle::Warning,
        NoticeStyle::Error,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NoticeStyle::Info => "Info",
            NoticeStyle::Success => "Success",
            NoticeStyle::Warning => "Warning",
            NoticeStyle::Error => "Error",
        }
    }

    /// The name the pop-up helpers understand.
    pub fn key(self) -> &'static str {
        match self {
            NoticeStyle::Info => "info",
            NoticeStyle::Success => "success",
            NoticeStyle::Warning => "warning",
            NoticeStyle::Error => "error",
        }
    }

    pub fn color(self) -> u32 {
        match self {
            NoticeStyle::Info => 0x7c5cff,
            NoticeStyle::Success => 0x34c77b,
            NoticeStyle::Warning => 0xf5b84a,
            NoticeStyle::Error => 0xef5b5b,
        }
    }
}

/// Where a pop-up notification appears.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticePos {
    #[default]
    TopCenter,
    TopRight,
}

impl NoticePos {
    pub fn key(self) -> &'static str {
        match self {
            NoticePos::TopCenter => "top-center",
            NoticePos::TopRight => "top-right",
        }
    }
}

fn default_notice_ms() -> u64 {
    3000
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
        /// The area for `Target::Area`. Optional, so older macros still open.
        #[serde(default)]
        area: Option<ScreenRect>,
        /// Move up to this many pixels in a random direction each time (0 = exact).
        #[serde(default)]
        jitter_px: i32,
    },
    MoveMouse {
        target: Target,
        x: i32,
        y: i32,
        offset_x: i32,
        offset_y: i32,
        #[serde(default)]
        area: Option<ScreenRect>,
        #[serde(default)]
        jitter_px: i32,
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
        /// Only look inside this area (much faster). None = every screen.
        /// Optional, so macros saved before this existed still open.
        #[serde(default)]
        search_area: Option<ScreenRect>,
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
        /// Read the area as a number even for text comparisons (fixes misread digits).
        /// Number comparisons always do this. Optional, so older macros still open.
        #[serde(default)]
        numbers_only: bool,
    },
    End,
    /// A collapsed run of steps. Its steps live in `graph`, between a
    /// GroupIn step (where the group starts) and a GroupOut step (where it hands back).
    Group {
        #[serde(default)]
        name: String,
        #[serde(default)]
        graph: Graph,
    },
    /// The start point inside a group.
    GroupIn,
    /// Show a pop-up card at the top of the screen.
    /// Every field is optional so the step can grow without breaking saved macros.
    Notify {
        #[serde(default)]
        title: String,
        #[serde(default)]
        message: String,
        #[serde(default)]
        style: NoticeStyle,
        #[serde(default)]
        position: NoticePos,
        #[serde(default = "default_notice_ms")]
        duration_ms: u64,
        /// Pause the macro until the pop-up is clicked.
        #[serde(default)]
        wait_for_click: bool,
        #[serde(default)]
        sound: bool,
    },
    /// Leaving a group: playback carries on after the Group step.
    GroupOut,
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
                area: None,
                jitter_px: 0,
            },
            NodeKind::MoveMouse {
                target: Target::Fixed,
                x: 500,
                y: 400,
                offset_x: 0,
                offset_y: 0,
                area: None,
                jitter_px: 0,
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
            NodeKind::Notify {
                title: "Macro update".into(),
                message: "Step reached".into(),
                style: NoticeStyle::Info,
                position: NoticePos::TopCenter,
                duration_ms: default_notice_ms(),
                wait_for_click: false,
                sound: false,
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
                search_area: None,
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
                numbers_only: false,
            },
            NodeKind::Loop { count: 3 },
            NodeKind::Group {
                name: String::new(),
                graph: Graph::default(),
            },
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
            NodeKind::Group { .. } => "Group",
            NodeKind::GroupIn => "Group start",
            NodeKind::Notify { .. } => "Pop-up",
            NodeKind::GroupOut => "Group end",
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
            NodeKind::Group { .. } => "▤",
            NodeKind::GroupIn => "▷",
            NodeKind::Notify { .. } => "✦",
            NodeKind::GroupOut => "➜",
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
            NodeKind::Group { .. } | NodeKind::GroupIn | NodeKind::GroupOut => GROUP_COLOR,
            NodeKind::Notify { style, .. } => style.color(),
        }
    }

    /// Names of this step's outputs. Output 0 is the normal "next" path.
    pub fn outputs(&self) -> &'static [&'static str] {
        match self {
            NodeKind::End | NodeKind::GroupOut => &[],
            NodeKind::FindImage { .. } => &["found", "not found"],
            NodeKind::Loop { .. } => &["repeat", "done"],
            NodeKind::WindowCheck { .. } => &["yes", "no"],
            NodeKind::ReadText { .. } => &["match", "no match"],
            _ => &["next"],
        }
    }

    pub fn has_input(&self) -> bool {
        !matches!(self, NodeKind::Start | NodeKind::GroupIn)
    }

    /// Steps that are part of the macro's structure and can't be deleted, copied or grouped.
    pub fn is_fixed(&self) -> bool {
        matches!(self, NodeKind::Start | NodeKind::GroupIn | NodeKind::GroupOut)
    }

    pub fn is_group(&self) -> bool {
        matches!(self, NodeKind::Group { .. })
    }

    pub fn summary(&self) -> String {
        fn place(target: &Target, x: i32, y: i32, ox: i32, oy: i32, area: &Option<ScreenRect>, jitter: i32) -> String {
            let at = match target {
                Target::Fixed => format!("at ({x}, {y})"),
                Target::FoundImage if ox == 0 && oy == 0 => "on found image".into(),
                Target::FoundImage => format!("on found image {ox:+}, {oy:+}"),
                Target::Current => "at current position".into(),
                Target::Area => match area {
                    Some(r) => format!("randomly in {}×{}", r.w, r.h),
                    None => "randomly in an area (not set)".into(),
                },
            };
            if jitter > 0 {
                format!("{at} ±{jitter}px")
            } else {
                at
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
                area,
                jitter_px,
            } => {
                let times = if *clicks > 1 {
                    format!(" ×{clicks}")
                } else {
                    String::new()
                };
                format!(
                    "{}{times} {}",
                    button.label(),
                    place(target, *x, *y, *offset_x, *offset_y, area, *jitter_px)
                )
            }
            NodeKind::MoveMouse {
                target,
                x,
                y,
                offset_x,
                offset_y,
                area,
                jitter_px,
            } => format!("Move {}", place(target, *x, *y, *offset_x, *offset_y, area, *jitter_px)),
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
                search_area,
                ..
            } => {
                let name = template
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "no image yet".into());
                let area = if search_area.is_some() { " · in area" } else { "" };
                format!("{name} · ≥{:.0}% · {timeout_ms} ms{area}", threshold * 100.0)
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
            NodeKind::Group { name, graph } => {
                let n = graph.step_count();
                let label = if name.trim().is_empty() { "Group" } else { name.trim() };
                format!("{label} · {n} step{} · double-click to open", if n == 1 { "" } else { "s" })
            }
            NodeKind::GroupIn => "The group starts here".into(),
            NodeKind::Notify {
                title,
                message,
                style,
                duration_ms,
                wait_for_click,
                ..
            } => {
                let text = if !title.trim().is_empty() {
                    title.trim().to_string()
                } else if !message.trim().is_empty() {
                    message.trim().to_string()
                } else {
                    "(empty pop-up)".to_string()
                };
                let short: String = text.chars().take(24).collect();
                let more = if text.chars().count() > 24 { "…" } else { "" };
                let how = if *wait_for_click {
                    "waits for a click".to_string()
                } else {
                    format!("{:.1}s", *duration_ms as f32 / 1000.0)
                };
                format!("{} · “{short}{more}” · {how}", style.label())
            }
            NodeKind::GroupOut => "Leave the group and carry on".into(),
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

/// A set of steps and the connections between them. The whole macro is one,
/// and each Group step holds another inside it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

impl Graph {
    /// Find a step anywhere in this graph or the groups inside it.
    pub fn node(&self, id: u64) -> Option<&Node> {
        for n in &self.nodes {
            if n.id == id {
                return Some(n);
            }
            if let NodeKind::Group { graph, .. } = &n.kind {
                if let Some(x) = graph.node(id) {
                    return Some(x);
                }
            }
        }
        None
    }

    pub fn node_mut(&mut self, id: u64) -> Option<&mut Node> {
        for n in self.nodes.iter_mut() {
            if n.id == id {
                return Some(n);
            }
            if let NodeKind::Group { graph, .. } = &mut n.kind {
                if let Some(x) = graph.node_mut(id) {
                    return Some(x);
                }
            }
        }
        None
    }

    pub fn contains(&self, id: u64) -> bool {
        self.node(id).is_some()
    }

    /// The graph (this one or a group inside it) whose own steps include `id`.
    pub fn owner(&self, id: u64) -> Option<&Graph> {
        if self.nodes.iter().any(|n| n.id == id) {
            return Some(self);
        }
        for n in &self.nodes {
            if let NodeKind::Group { graph, .. } = &n.kind {
                if let Some(g) = graph.owner(id) {
                    return Some(g);
                }
            }
        }
        None
    }

    pub fn owner_mut(&mut self, id: u64) -> Option<&mut Graph> {
        if self.nodes.iter().any(|n| n.id == id) {
            return Some(self);
        }
        for n in self.nodes.iter_mut() {
            if let NodeKind::Group { graph, .. } = &mut n.kind {
                if let Some(g) = graph.owner_mut(id) {
                    return Some(g);
                }
            }
        }
        None
    }

    /// The graph inside the groups listed in `path` (outermost first).
    pub fn at(&self, path: &[u64]) -> Option<&Graph> {
        let Some((first, rest)) = path.split_first() else {
            return Some(self);
        };
        match &self.nodes.iter().find(|n| n.id == *first)?.kind {
            NodeKind::Group { graph, .. } => graph.at(rest),
            _ => None,
        }
    }

    pub fn at_mut(&mut self, path: &[u64]) -> Option<&mut Graph> {
        let Some((first, rest)) = path.split_first() else {
            return Some(self);
        };
        match &mut self.nodes.iter_mut().find(|n| n.id == *first)?.kind {
            NodeKind::Group { graph, .. } => graph.at_mut(rest),
            _ => None,
        }
    }

    /// The groups you'd open, outermost first, to see the step `id`.
    pub fn path_to(&self, id: u64) -> Option<Vec<u64>> {
        if self.nodes.iter().any(|n| n.id == id) {
            return Some(Vec::new());
        }
        for n in &self.nodes {
            if let NodeKind::Group { graph, .. } = &n.kind {
                if let Some(mut p) = graph.path_to(id) {
                    p.insert(0, n.id);
                    return Some(p);
                }
            }
        }
        None
    }

    fn next_local(&self, from: u64, port: usize) -> Option<u64> {
        self.edges
            .iter()
            .find(|e| e.from == from && e.port == port)
            .map(|e| e.to)
    }

    fn incoming(&self, to: u64) -> usize {
        self.edges.iter().filter(|e| e.to == to).count()
    }

    /// Run `f` on every step, including the ones inside groups.
    pub fn for_each_mut(&mut self, f: &mut dyn FnMut(&mut Node)) {
        for n in self.nodes.iter_mut() {
            f(n);
            if let NodeKind::Group { graph, .. } = &mut n.kind {
                graph.for_each_mut(f);
            }
        }
    }

    /// Steps directly in this graph, not counting the group start/end markers.
    pub fn step_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| !matches!(n.kind, NodeKind::GroupIn | NodeKind::GroupOut))
            .count()
    }

    pub fn max_id(&self) -> u64 {
        let mut max = 0;
        for n in &self.nodes {
            max = max.max(n.id);
            if let NodeKind::Group { graph, .. } = &n.kind {
                max = max.max(graph.max_id());
            }
        }
        max
    }

    /// Steps in running order, following the main output from `start`.
    pub fn chain_from(&self, start: u64) -> Vec<u64> {
        let mut out = vec![start];
        let mut cur = start;
        while let Some(next) = self.next_local(cur, 0) {
            if out.contains(&next) {
                break;
            }
            out.push(next);
            cur = next;
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "MacroFile", into = "MacroFile")]
pub struct Macro {
    pub name: String,
    /// The top level of the macro.
    pub root: Graph,
    pub next_id: u64,
    /// Wait after pressing Run, so you can switch to the target app.
    pub start_delay_ms: u64,
    /// Wait between every step.
    pub step_delay_ms: u64,
    /// Playback speed in percent (200 = twice as fast). Scales every delay.
    pub speed_pct: u32,
    /// How many times to run the whole macro (0 = until stopped).
    pub repeat: u32,
}

/// The macro exactly as it's written to disk. This is the same layout older
/// versions of MacroFlow use, so their files still open.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MacroFile {
    name: String,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    next_id: u64,
    #[serde(default = "default_start_delay")]
    start_delay_ms: u64,
    #[serde(default = "default_step_delay")]
    step_delay_ms: u64,
    #[serde(default = "default_speed")]
    speed_pct: u32,
    #[serde(default = "default_repeat")]
    repeat: u32,
}

impl From<MacroFile> for Macro {
    fn from(f: MacroFile) -> Self {
        let root = Graph {
            nodes: f.nodes,
            edges: f.edges,
        };
        // Never hand out an id that's already used (e.g. a hand-edited file).
        let next_id = f.next_id.max(root.max_id() + 1);
        Macro {
            name: f.name,
            root,
            next_id,
            start_delay_ms: f.start_delay_ms,
            step_delay_ms: f.step_delay_ms,
            speed_pct: f.speed_pct,
            repeat: f.repeat,
        }
    }
}

impl From<Macro> for MacroFile {
    fn from(m: Macro) -> Self {
        MacroFile {
            name: m.name,
            nodes: m.root.nodes,
            edges: m.root.edges,
            next_id: m.next_id,
            start_delay_ms: m.start_delay_ms,
            step_delay_ms: m.step_delay_ms,
            speed_pct: m.speed_pct,
            repeat: m.repeat,
        }
    }
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
            root: Graph::default(),
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

fn blank_node(id: u64, kind: NodeKind, x: f32, y: f32) -> Node {
    Node {
        id,
        kind,
        x,
        y,
        delay_ms: 0,
        note: String::new(),
    }
}

impl Macro {
    fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Find a step anywhere in the macro, including inside groups.
    pub fn node(&self, id: u64) -> Option<&Node> {
        self.root.node(id)
    }

    pub fn node_mut(&mut self, id: u64) -> Option<&mut Node> {
        self.root.node_mut(id)
    }

    /// Add a step to the top level of the macro.
    pub fn add_node(&mut self, kind: NodeKind, x: f32, y: f32) -> u64 {
        self.add_node_in(&[], kind, x, y)
    }

    /// Add a step inside the groups listed in `path` (or the top level if it's empty).
    pub fn add_node_in(&mut self, path: &[u64], kind: NodeKind, x: f32, y: f32) -> u64 {
        let id = self.alloc();
        let node = blank_node(id, kind, x, y);
        match self.root.at_mut(path) {
            Some(g) => g.nodes.push(node),
            None => self.root.nodes.push(node),
        }
        id
    }

    pub fn remove_node(&mut self, id: u64) {
        if let Some(g) = self.root.owner_mut(id) {
            g.nodes.retain(|n| n.id != id);
            g.edges.retain(|e| e.from != id && e.to != id);
        }
    }

    /// Connect an output to a step. Each output has at most one connection.
    /// Both steps must be in the same graph (the same group, or both at the top level).
    pub fn connect(&mut self, from: u64, port: usize, to: u64) {
        if let Some(g) = self.root.owner_mut(from) {
            if !g.nodes.iter().any(|n| n.id == to) {
                return;
            }
            g.edges.retain(|e| !(e.from == from && e.port == port));
            g.edges.push(Edge { from, port, to });
        }
    }

    pub fn disconnect(&mut self, from: u64, port: usize) {
        if let Some(g) = self.root.owner_mut(from) {
            g.edges.retain(|e| !(e.from == from && e.port == port));
        }
    }

    pub fn next(&self, from: u64, port: usize) -> Option<u64> {
        self.root.owner(from)?.next_local(from, port)
    }

    pub fn start_id(&self) -> Option<u64> {
        self.root
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Start))
            .map(|n| n.id)
    }

    /// Multiply every delay in the macro (step delays, waits and recordings), including inside groups.
    pub fn scale_delays(&mut self, factor: f32) {
        let f = |v: u64| (v as f32 * factor).round() as u64;
        self.root.for_each_mut(&mut |n: &mut Node| {
            n.delay_ms = f(n.delay_ms);
            if let NodeKind::Wait { ms, jitter_ms } = &mut n.kind {
                *ms = f(*ms);
                *jitter_ms = f(*jitter_ms);
            }
            n.kind.scale_recording(factor);
        });
        self.step_delay_ms = f(self.step_delay_ms);
    }

    /// Set every recorded delay and every per-step delay to one value, including inside groups.
    /// Wait steps are left alone, since those are usually deliberate.
    pub fn set_all_delays(&mut self, ms: u64) {
        self.root.for_each_mut(&mut |n: &mut Node| {
            n.delay_ms = ms;
            if let NodeKind::Recording { events, .. } = &mut n.kind {
                for e in events.iter_mut() {
                    e.delay_ms = ms;
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // Groups
    // -----------------------------------------------------------------------

    fn group_count(&self) -> usize {
        fn count(g: &Graph) -> usize {
            g.nodes
                .iter()
                .map(|n| match &n.kind {
                    NodeKind::Group { graph, .. } => 1 + count(graph),
                    _ => 0,
                })
                .sum()
        }
        count(&self.root)
    }

    /// A new, empty group (just its start and end) ready to add to the macro.
    pub fn new_group_kind(&mut self) -> NodeKind {
        let (in_id, out_id) = (self.alloc(), self.alloc());
        let name = format!("Group {}", self.group_count() + 1);
        NodeKind::Group {
            name,
            graph: Graph {
                nodes: vec![
                    blank_node(in_id, NodeKind::GroupIn, 60.0, 180.0),
                    blank_node(out_id, NodeKind::GroupOut, 60.0 + NODE_W + 120.0, 180.0),
                ],
                edges: vec![Edge {
                    from: in_id,
                    port: 0,
                    to: out_id,
                }],
            },
        }
    }

    /// Give every step inside a (copied) group a new id, so a copy doesn't clash with the original.
    pub fn refresh_ids(&mut self, kind: &mut NodeKind) {
        if let NodeKind::Group { graph, .. } = kind {
            let mut map = HashMap::new();
            for n in graph.nodes.iter_mut() {
                let new_id = self.alloc();
                map.insert(n.id, new_id);
                n.id = new_id;
                self.refresh_ids(&mut n.kind);
            }
            graph.edges.retain(|e| map.contains_key(&e.from) && map.contains_key(&e.to));
            for e in graph.edges.iter_mut() {
                e.from = map[&e.from];
                e.to = map[&e.to];
            }
        }
    }

    /// The longest straight line of steps starting at `start` that can be grouped.
    pub fn straight_run(&self, start: u64) -> Vec<u64> {
        let Some(g) = self.root.owner(start) else {
            return vec![];
        };
        let groupable = |id: u64| {
            g.nodes
                .iter()
                .find(|n| n.id == id)
                .map_or(false, |n| !n.kind.is_fixed() && !matches!(n.kind, NodeKind::End))
        };
        if !groupable(start) {
            return vec![];
        }
        let mut run = vec![start];
        let mut cur = start;
        loop {
            let single_output = g
                .nodes
                .iter()
                .find(|n| n.id == cur)
                .map_or(false, |n| n.kind.outputs().len() == 1);
            if !single_output {
                break;
            }
            let Some(next) = g.next_local(cur, 0) else { break };
            if run.contains(&next) || !groupable(next) || g.incoming(next) != 1 {
                break;
            }
            run.push(next);
            cur = next;
        }
        run
    }

    /// Turn a straight line of steps into one Group step. Returns the new group's id.
    pub fn group_steps(&mut self, ids: &[u64]) -> Result<u64, String> {
        if ids.len() < 2 {
            return Err("Pick at least 2 steps in a straight line to group.".into());
        }
        let (in_id, out_id, group_id) = (self.alloc(), self.alloc(), self.alloc());
        let name = format!("Group {}", self.group_count() + 1);
        let g = self
            .root
            .owner_mut(ids[0])
            .ok_or_else(|| "Those steps no longer exist.".to_string())?;
        let set: Vec<u64> = ids.to_vec();
        if !set.iter().all(|id| g.nodes.iter().any(|n| n.id == *id)) {
            return Err("All the steps to group must be in the same place (not some inside another group).".into());
        }
        let kind_of = |g: &Graph, id: u64| g.nodes.iter().find(|n| n.id == id).map(|n| n.kind.clone());
        for id in &set {
            match kind_of(g, *id) {
                Some(k) if k.is_fixed() => {
                    return Err(format!("The {} step can't go inside a group.", k.title()));
                }
                _ => {}
            }
        }

        // The first step is the only one nothing else in the selection points to.
        let heads: Vec<u64> = set
            .iter()
            .copied()
            .filter(|id| !g.edges.iter().any(|e| e.to == *id && set.contains(&e.from)))
            .collect();
        if heads.len() != 1 {
            return Err("The steps must be one straight line, each connected to the next.".into());
        }
        let mut order = vec![heads[0]];
        while order.len() < set.len() {
            let cur = *order.last().unwrap();
            let outs = kind_of(g, cur).map_or(0, |k| k.outputs().len());
            let nexts: Vec<&Edge> = g
                .edges
                .iter()
                .filter(|e| e.from == cur && set.contains(&e.to))
                .collect();
            if outs != 1 || nexts.len() != 1 || nexts[0].port != 0 || order.contains(&nexts[0].to) {
                return Err("The steps must be one straight line, each connected to the next. Steps that branch (like Find image) can only be the last one.".into());
            }
            order.push(nexts[0].to);
        }
        for id in &order[1..] {
            if g.incoming(*id) != 1 {
                return Err("A step in the middle has other connections coming into it, so the line can't be grouped.".into());
            }
        }
        let last = *order.last().unwrap();
        let last_outs = kind_of(g, last).map_or(0, |k| k.outputs().len());
        if last_outs > 1 {
            return Err("The last step branches (e.g. found / not found), so the group couldn't carry on from it. Leave that step out of the group.".into());
        }

        // Pull the steps out, keeping their layout.
        let head = g.nodes.iter().find(|n| n.id == order[0]).cloned().unwrap();
        let (gx, gy) = (head.x, head.y);
        let (dx, dy) = (60.0 + NODE_W + 80.0 - gx, 180.0 - gy);
        let mut inner_nodes: Vec<Node> = Vec::new();
        g.nodes.retain(|n| {
            if set.contains(&n.id) {
                let mut moved = n.clone();
                moved.x += dx;
                moved.y += dy;
                inner_nodes.push(moved);
                false
            } else {
                true
            }
        });
        let mut inner_edges: Vec<Edge> = g
            .edges
            .iter()
            .copied()
            .filter(|e| set.contains(&e.from) && set.contains(&e.to))
            .collect();
        let after = g.next_local(last, 0).filter(|t| !set.contains(t));
        g.edges.retain(|e| !(set.contains(&e.from) && set.contains(&e.to)));
        g.edges.retain(|e| !(e.from == last && e.port == 0));
        for e in g.edges.iter_mut() {
            if e.to == order[0] {
                e.to = group_id;
            }
        }
        if let Some(t) = after {
            g.edges.push(Edge {
                from: group_id,
                port: 0,
                to: t,
            });
        }

        let last_node = inner_nodes.iter().find(|n| n.id == last).cloned().unwrap();
        inner_nodes.insert(0, blank_node(in_id, NodeKind::GroupIn, 60.0, 180.0));
        inner_nodes.push(blank_node(
            out_id,
            NodeKind::GroupOut,
            last_node.x + NODE_W + 80.0,
            last_node.y,
        ));
        inner_edges.push(Edge {
            from: in_id,
            port: 0,
            to: order[0],
        });
        if last_outs == 1 {
            inner_edges.push(Edge {
                from: last,
                port: 0,
                to: out_id,
            });
        }
        g.nodes.push(blank_node(
            group_id,
            NodeKind::Group {
                name,
                graph: Graph {
                    nodes: inner_nodes,
                    edges: inner_edges,
                },
            },
            gx,
            gy,
        ));
        Ok(group_id)
    }

    /// Put a group's steps back where the group was. Returns the ids of the steps.
    pub fn ungroup(&mut self, group_id: u64) -> Result<Vec<u64>, String> {
        let g = self
            .root
            .owner_mut(group_id)
            .ok_or_else(|| "That group no longer exists.".to_string())?;
        let idx = g
            .nodes
            .iter()
            .position(|n| n.id == group_id)
            .ok_or_else(|| "That group no longer exists.".to_string())?;
        let (gx, gy) = (g.nodes[idx].x, g.nodes[idx].y);
        let NodeKind::Group { graph: inner, .. } = g.nodes[idx].kind.clone() else {
            return Err("That step isn't a group.".into());
        };
        let in_id = inner.nodes.iter().find(|n| matches!(n.kind, NodeKind::GroupIn)).map(|n| n.id);
        let out_ids: Vec<u64> = inner
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::GroupOut))
            .map(|n| n.id)
            .collect();
        let first = in_id.and_then(|i| inner.next_local(i, 0)).filter(|f| !out_ids.contains(f));
        let after = g.next_local(group_id, 0);

        let anchor = first
            .and_then(|f| inner.nodes.iter().find(|n| n.id == f))
            .map(|n| (n.x, n.y))
            .unwrap_or((gx, gy));
        let (dx, dy) = (gx - anchor.0, gy - anchor.1);

        let moved: Vec<Node> = inner
            .nodes
            .iter()
            .filter(|n| Some(n.id) != in_id && !out_ids.contains(&n.id))
            .map(|n| {
                let mut c = n.clone();
                c.x += dx;
                c.y += dy;
                c
            })
            .collect();
        let moved_ids: Vec<u64> = moved.iter().map(|n| n.id).collect();

        g.nodes.remove(idx);
        let incoming: Vec<Edge> = g.edges.iter().copied().filter(|e| e.to == group_id).collect();
        g.edges.retain(|e| e.from != group_id && e.to != group_id);
        g.nodes.extend(moved);
        for e in &inner.edges {
            let touches_marker = Some(e.from) == in_id
                || Some(e.to) == in_id
                || out_ids.contains(&e.from)
                || out_ids.contains(&e.to);
            if !touches_marker {
                g.edges.push(*e);
            }
        }
        // What pointed at the group now points at its first step (or straight past an empty group).
        for e in incoming {
            if let Some(to) = first.or(after) {
                g.edges.push(Edge { to, ..e });
            }
        }
        // Whatever led to the group's end now leads to what came after the group.
        if let Some(t) = after {
            for e in inner.edges.iter().filter(|e| out_ids.contains(&e.to)) {
                if Some(e.from) != in_id {
                    g.edges.push(Edge {
                        from: e.from,
                        port: e.port,
                        to: t,
                    });
                }
            }
        }
        Ok(moved_ids)
    }
}

/// Where MacroFlow keeps macros and snipped images by default.
/// (Not "Documents/MacroFlow": macOS and Windows ignore letter case in folder names,
/// so that would be the same folder as a "Documents/macroflow" source checkout.)
pub fn app_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Documents").join("MacroFlow Macros")
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

/// Characters OCR often reads instead of digits, and the digit they usually are.
fn confusable_digit(c: char) -> Option<char> {
    Some(match c {
        '0'..='9' => c,
        'O' | 'o' | 'D' | 'Q' | 'U' => '0',
        'I' | 'l' | '|' | 'i' | '!' | 'j' | 'J' => '1',
        'Z' | 'z' => '2',
        'S' | 's' => '5',
        'B' => '8',
        'G' | 'b' => '6',
        'g' | 'q' => '9',
        _ => return None,
    })
}

/// Look-alikes that are only trusted in the middle of a number, never at its ends
/// (so "15s" stays 15 and "Gold" doesn't become a number).
fn weak_lookalike(c: char) -> bool {
    matches!(c, 'D' | 'Q' | 'U' | 'i' | '!' | 'j' | 'J' | 'z' | 's' | 'G' | 'b' | 'g' | 'q')
}

/// Turn whatever OCR read into a clean number like "-12345.5", or None if there's no number.
///
/// It fixes letters that are really digits ("1O0" → "100", "l5" → "15"), joins digits that
/// were split by spaces ("12 345" → "12345") and handles thousands separators. If several
/// numbers are found, the longest one wins, because a stray icon read as "1" is shorter
/// than the real value.
pub fn clean_number(raw: &str) -> Option<String> {
    let chars: Vec<char> = raw
        .chars()
        .map(|c| match c {
            '−' | '–' | '—' => '-',
            '\n' | '\r' | '\t' => ' ',
            c => c,
        })
        .collect();

    struct Run {
        start: usize,
        end: usize,
        text: String,
        real: usize,
        fake: usize,
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if confusable_digit(chars[i]).is_none() {
            i += 1;
            continue;
        }
        // Grab a run of digit-like characters (with . or , between digits).
        let mut j = i;
        while j < chars.len() {
            let c = chars[j];
            let sep_ok = (c == '.' || c == ',')
                && j > i
                && j + 1 < chars.len()
                && confusable_digit(chars[j + 1]).is_some();
            if confusable_digit(c).is_some() || sep_ok {
                j += 1;
            } else {
                break;
            }
        }
        let (mut s, mut e) = (i, j);
        while s < e && (weak_lookalike(chars[s]) || matches!(chars[s], '.' | ',')) {
            s += 1;
        }
        while e > s && (weak_lookalike(chars[e - 1]) || matches!(chars[e - 1], '.' | ',')) {
            e -= 1;
        }
        if s < e {
            let mut text = String::new();
            let (mut real, mut fake) = (0, 0);
            for &c in &chars[s..e] {
                match confusable_digit(c) {
                    Some(d) => {
                        if c.is_ascii_digit() {
                            real += 1;
                        } else {
                            fake += 1;
                        }
                        text.push(d);
                    }
                    None => text.push(c),
                }
            }
            runs.push(Run { start: s, end: e, text, real, fake });
        }
        i = j;
    }

    let is_letter = |k: Option<&char>| k.map_or(false, |c| c.is_alphabetic());
    let all_digits = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit());
    let three_digits = |r: &Run| r.text.len() == 3 && all_digits(&r.text);
    let spaces_between = |a: usize, b: usize| a < b && chars[a..b].iter().all(|c| c.is_whitespace());
    // A lone look-alike ("l", "I", "|") right before a 3-digit group is the leading digit
    // of a spaced number: "l 892 039" is 1 892 039, not 892 039.
    let lone_lead = |idx: usize| -> bool {
        let r = &runs[idx];
        if r.real != 0 || r.text.len() > 3 || !all_digits(&r.text) {
            return false;
        }
        if is_letter(r.start.checked_sub(1).and_then(|p| chars.get(p))) || is_letter(chars.get(r.end)) {
            return false;
        }
        runs.get(idx + 1)
            .map_or(false, |n| n.real >= 2 && three_digits(n) && spaces_between(r.end, n.start))
    };
    // Mostly real digits (or a leading look-alike as above)...
    let strict: Vec<&Run> = runs
        .iter()
        .enumerate()
        .filter(|(i, r)| (r.real > 0 && r.fake <= r.real + 1) || lone_lead(*i))
        .map(|(_, r)| r)
        .collect();
    // ...or, if there are none, a whole word made only of look-alikes (e.g. "lOO").
    let loose: Vec<&Run> = runs
        .iter()
        .filter(|r| {
            r.real == 0
                && r.fake >= 2
                && !is_letter(r.start.checked_sub(1).and_then(|p| chars.get(p)))
                && !is_letter(chars.get(r.end))
        })
        .collect();
    let keep = if strict.is_empty() { loose } else { strict };
    if keep.is_empty() {
        return None;
    }

    // Is the whole read just digits and spaces? Then it's one number split up.
    let only_number = chars.iter().all(|c| {
        c.is_whitespace() || confusable_digit(*c).is_some() || matches!(c, '.' | ',' | '-' | '+')
    });

    // Join neighbouring runs separated only by spaces when they form one number:
    // "1 892 039" (a 1-3 digit lead, then groups of exactly 3) or a read that's only digits.
    // (start, text, still a valid "spaced thousands" chain)
    let mut groups: Vec<(usize, String, bool)> = Vec::new();
    let mut prev_end: Option<usize> = None;
    for r in keep {
        let gap_is_space = prev_end.map_or(false, |e| spaces_between(e, r.start));
        if let Some((_, last, chain)) = groups.last_mut() {
            if gap_is_space && (only_number || (three_digits(r) && *chain)) {
                last.push_str(&r.text);
                *chain = *chain && three_digits(r);
                prev_end = Some(r.end);
                continue;
            }
        }
        let chain = all_digits(&r.text) && r.text.len() <= 3;
        groups.push((r.start, r.text.clone(), chain));
        prev_end = Some(r.end);
    }
    let groups: Vec<(usize, String)> = groups.into_iter().map(|(s, t, _)| (s, t)).collect();

    // Longest number wins; on a tie, the first one.
    let mut best: Option<(usize, String)> = None;
    for (start, text) in groups {
        let len = text.chars().filter(|c| c.is_ascii_digit()).count();
        let better = best
            .as_ref()
            .map_or(true, |(_, b)| len > b.chars().filter(|c| c.is_ascii_digit()).count());
        if better {
            best = Some((start, text));
        }
    }
    let (start, text) = best?;
    let negative = chars[..start]
        .iter()
        .rev()
        .find(|c| !c.is_whitespace())
        .map_or(false, |c| *c == '-');
    canonical_number(&text).map(|n| if negative { format!("-{n}") } else { n })
}

/// "12,345.50" → "12345.50", "12,5" → "12.5", "1.234.567" → "1234567".
fn canonical_number(s: &str) -> Option<String> {
    let parts: Vec<(char, &str)> = {
        let mut v = Vec::new();
        let mut last = 0;
        let mut sep = ' ';
        for (i, c) in s.char_indices() {
            if c == '.' || c == ',' {
                v.push((sep, &s[last..i]));
                sep = c;
                last = i + 1;
            }
        }
        v.push((sep, &s[last..]));
        v
    };
    if parts.iter().any(|(_, p)| p.is_empty()) {
        let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        return (!digits.is_empty()).then_some(digits);
    }
    let mut out = parts[0].1.to_string();
    let rest = &parts[1..];
    for (idx, (sep, p)) in rest.iter().enumerate() {
        let is_last = idx == rest.len() - 1;
        // A group of exactly 3 digits after a separator is thousands grouping,
        // unless it's the last group after a "." (then it's a decimal, e.g. 1.250).
        let thousands = p.len() == 3 && !(is_last && *sep == '.' && rest.len() == 1);
        if thousands {
            out.push_str(p);
        } else if is_last {
            out.push('.');
            out.push_str(p);
        } else {
            out.push_str(p);
        }
    }
    Some(out)
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
    fn cleans_ocr_numbers() {
        assert_eq!(clean_number("1O0").as_deref(), Some("100"));
        assert_eq!(clean_number("lOO").as_deref(), Some("100"));
        assert_eq!(clean_number("12 345").as_deref(), Some("12345"));
        assert_eq!(clean_number("Gold: 1,500").as_deref(), Some("1500"));
        assert_eq!(clean_number("Score 12,345.5").as_deref(), Some("12345.5"));
        assert_eq!(clean_number("-42").as_deref(), Some("-42"));
        assert_eq!(clean_number("1500pts").as_deref(), Some("1500"));
        assert_eq!(clean_number("Level 3   98765").as_deref(), Some("98765"));
        assert_eq!(clean_number("no digits here"), None);
        // Numbers spaced into thousands, including a leading 1 misread as l / I / |.
        assert_eq!(clean_number("1 892 039").as_deref(), Some("1892039"));
        assert_eq!(clean_number("l 892 039").as_deref(), Some("1892039"));
        assert_eq!(clean_number("I 892 039").as_deref(), Some("1892039"));
        assert_eq!(clean_number("Gold: | 892 039").as_deref(), Some("1892039"));
        assert_eq!(clean_number("892 390").as_deref(), Some("892390"));
        assert_eq!(clean_number("Gold: 892 390").as_deref(), Some("892390"));
        assert_eq!(clean_number("12 345 678").as_deref(), Some("12345678"));
        assert_eq!(clean_number("Level 3   98765").as_deref(), Some("98765"));
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

    #[test]
    fn old_files_still_open() {
        let old = include_str!("../examples/hello.json");
        let m: Macro = serde_json::from_str(old).expect("old macro file should load");
        assert_eq!(m.root.nodes.len(), 6);
        assert!(m.next_id > m.root.max_id());
        // Saving keeps the same top-level layout.
        let v: serde_json::Value = serde_json::to_value(&m).unwrap();
        assert!(v.get("nodes").is_some() && v.get("edges").is_some() && v.get("root").is_none());
    }

    #[test]
    fn old_find_image_steps_still_open() {
        let old = r#"{"type":"FindImage","template":null,"threshold":0.85,"timeout_ms":5000,"interval_ms":250,"click_when_found":false}"#;
        let k: NodeKind = serde_json::from_str(old).expect("old Find image step should load");
        assert!(matches!(k, NodeKind::FindImage { search_area: None, .. }));
    }

    #[test]
    fn old_click_steps_still_open() {
        let old = r#"{"type":"Click","target":"Fixed","x":10,"y":20,"offset_x":0,"offset_y":0,"button":"Left","clicks":1}"#;
        let k: NodeKind = serde_json::from_str(old).expect("old Click step should load");
        assert!(matches!(k, NodeKind::Click { area: None, jitter_px: 0, .. }));
    }

    #[test]
    fn notify_steps_load_with_missing_fields() {
        let k: NodeKind = serde_json::from_str(r#"{"type":"Notify","title":"Hi"}"#).unwrap();
        assert!(matches!(
            k,
            NodeKind::Notify { duration_ms: 3000, wait_for_click: false, style: NoticeStyle::Info, .. }
        ));
    }

    #[test]
    fn group_and_ungroup() {
        let mut m = Macro::default();
        let start = m.start_id().unwrap();
        let a = m.add_node(NodeKind::Wait { ms: 1, jitter_ms: 0 }, 300.0, 180.0);
        let b = m.add_node(NodeKind::Wait { ms: 2, jitter_ms: 0 }, 600.0, 180.0);
        let c = m.add_node(NodeKind::End, 900.0, 180.0);
        m.connect(start, 0, a);
        m.connect(a, 0, b);
        m.connect(b, 0, c);
        assert_eq!(m.straight_run(a), vec![a, b]);
        let g = m.group_steps(&[a, b]).unwrap();
        assert_eq!(m.next(start, 0), Some(g));
        assert_eq!(m.next(g, 0), Some(c));
        assert!(m.node(a).is_some(), "grouped steps are still found");
        let json = serde_json::to_string(&m).unwrap();
        let back: Macro = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
        m.ungroup(g).unwrap();
        assert_eq!(m.next(start, 0), Some(a));
        assert_eq!(m.next(a, 0), Some(b));
        assert_eq!(m.next(b, 0), Some(c));
    }
}
