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

impl ScreenRect {
    #[allow(dead_code)]
    pub fn shifted(self, dx: i32, dy: i32) -> ScreenRect {
        ScreenRect {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }
}

/// Converts positions from one screen area to another, scaling them.
/// "Positions saved for this screen / window" → "the same spot on that one".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping {
    pub from: ScreenRect,
    pub to: ScreenRect,
}

impl Mapping {
    pub fn identity() -> Self {
        let unit = ScreenRect { x: 0, y: 0, w: 1, h: 1 };
        Mapping { from: unit, to: unit }
    }

    /// Just move positions by (dx, dy), no scaling.
    pub fn translate(dx: i32, dy: i32) -> Self {
        Mapping {
            from: ScreenRect { x: 0, y: 0, w: 1, h: 1 },
            to: ScreenRect { x: dx, y: dy, w: 1, h: 1 },
        }
    }

    pub fn inverse(&self) -> Self {
        Mapping { from: self.to, to: self.from }
    }

    fn sx(&self) -> f64 {
        self.to.w.max(1) as f64 / self.from.w.max(1) as f64
    }

    fn sy(&self) -> f64 {
        self.to.h.max(1) as f64 / self.from.h.max(1) as f64
    }

    /// How much bigger (> 1) or smaller (< 1) things are on the target.
    pub fn scale(&self) -> f64 {
        (self.sx() + self.sy()) / 2.0
    }

    pub fn is_identity(&self) -> bool {
        self.from.x == self.to.x && self.from.y == self.to.y && (self.sx() - 1.0).abs() < 1e-9 && (self.sy() - 1.0).abs() < 1e-9
    }

    pub fn point(&self, x: i32, y: i32) -> (i32, i32) {
        (
            (self.to.x as f64 + (x - self.from.x) as f64 * self.sx()).round() as i32,
            (self.to.y as f64 + (y - self.from.y) as f64 * self.sy()).round() as i32,
        )
    }

    pub fn size(&self, w: i32, h: i32) -> (i32, i32) {
        (
            ((w as f64 * self.sx()).round() as i32).max(1),
            ((h as f64 * self.sy()).round() as i32).max(1),
        )
    }

    pub fn rect(&self, r: ScreenRect) -> ScreenRect {
        let (x, y) = self.point(r.x, r.y);
        let (w, h) = self.size(r.w, r.h);
        ScreenRect { x, y, w, h }
    }
}

/// The window a macro's screen positions are relative to. `x`/`y` is where that
/// window's top-left corner was when the positions were captured.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowAnchor {
    pub app: String,
    #[serde(default)]
    pub title: String,
    pub x: i32,
    pub y: i32,
}

/// How an If step compares its two sides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CmpOp {
    #[default]
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
    NotContains,
}

impl CmpOp {
    pub const ALL: [CmpOp; 8] = [
        CmpOp::Eq,
        CmpOp::Ne,
        CmpOp::Gt,
        CmpOp::Ge,
        CmpOp::Lt,
        CmpOp::Le,
        CmpOp::Contains,
        CmpOp::NotContains,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "≠",
            CmpOp::Gt => ">",
            CmpOp::Ge => "≥",
            CmpOp::Lt => "<",
            CmpOp::Le => "≤",
            CmpOp::Contains => "contains",
            CmpOp::NotContains => "doesn't contain",
        }
    }
}

fn default_one() -> u32 {
    1
}

fn is_one(v: &u32) -> bool {
    *v == 1
}

fn is_true(v: &bool) -> bool {
    *v
}

fn default_true() -> bool {
    true
}

fn default_pixel_tolerance() -> u32 {
    24
}

fn default_pixel_interval() -> u64 {
    100
}

fn default_drag_ms() -> u64 {
    400
}

fn default_mouse_speed() -> u32 {
    100
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
        /// Wheel clicks (positive = down / right).
        amount: i32,
        horizontal: bool,
        /// How far each wheel click scrolls (1 = one notch, as before).
        /// Optional, so older macros still open.
        #[serde(default = "default_one", skip_serializing_if = "is_one")]
        strength: u32,
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
        /// Also store what was read in this variable (empty = don't).
        #[serde(default)]
        save_to: String,
    },
    /// Hold a mouse button at one spot, move to another, and let go.
    Drag {
        #[serde(default)]
        from_x: i32,
        #[serde(default)]
        from_y: i32,
        #[serde(default)]
        to_x: i32,
        #[serde(default)]
        to_y: i32,
        #[serde(default)]
        button: MouseBtn,
        /// How long the move between the two spots takes.
        #[serde(default = "default_drag_ms")]
        duration_ms: u64,
    },
    /// Store a value (text, a number, or a sum like `gold + 1000`) in a variable.
    SetVar {
        #[serde(default)]
        name: String,
        #[serde(default)]
        expr: String,
    },
    /// Compare two values (variables, numbers, text) and branch on the result.
    IfCompare {
        #[serde(default)]
        left: String,
        #[serde(default)]
        op: CmpOp,
        #[serde(default)]
        right: String,
    },
    /// Check the colour of one spot on the screen.
    PixelColor {
        #[serde(default)]
        x: i32,
        #[serde(default)]
        y: i32,
        /// 0xRRGGBB
        #[serde(default)]
        color: u32,
        /// How far each of red / green / blue may be off (0 = exact, 255 = anything).
        #[serde(default = "default_pixel_tolerance")]
        tolerance: u32,
        #[serde(default)]
        timeout_ms: u64,
        #[serde(default = "default_pixel_interval")]
        interval_ms: u64,
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
            NodeKind::Drag {
                from_x: 400,
                from_y: 400,
                to_x: 700,
                to_y: 400,
                button: MouseBtn::Left,
                duration_ms: default_drag_ms(),
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
                strength: 1,
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
                save_to: String::new(),
            },
            NodeKind::PixelColor {
                x: 0,
                y: 0,
                color: 0x34c77b,
                tolerance: default_pixel_tolerance(),
                timeout_ms: 0,
                interval_ms: default_pixel_interval(),
            },
            NodeKind::SetVar {
                name: "count".into(),
                expr: "count + 1".into(),
            },
            NodeKind::IfCompare {
                left: "count".into(),
                op: CmpOp::Ge,
                right: "10".into(),
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
            NodeKind::Drag { .. } => "Drag",
            NodeKind::KeyPress { .. } => "Key press",
            NodeKind::TypeText { .. } => "Type text",
            NodeKind::Wait { .. } => "Wait",
            NodeKind::Scroll { .. } => "Scroll",
            NodeKind::Recording { .. } => "Recording",
            NodeKind::FindImage { .. } => "Find image",
            NodeKind::Loop { .. } => "Loop",
            NodeKind::WindowCheck { .. } => "Window check",
            NodeKind::ReadText { .. } => "Read text",
            NodeKind::SetVar { .. } => "Set variable",
            NodeKind::IfCompare { .. } => "If",
            NodeKind::PixelColor { .. } => "Pixel colour",
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
            NodeKind::Drag { .. } => "⇢",
            NodeKind::KeyPress { .. } => "⌘",
            NodeKind::TypeText { .. } => "T",
            NodeKind::Wait { .. } => "◷",
            NodeKind::Scroll { .. } => "⇵",
            NodeKind::Recording { .. } => "●",
            NodeKind::FindImage { .. } => "◎",
            NodeKind::Loop { .. } => "↻",
            NodeKind::WindowCheck { .. } => "▣",
            NodeKind::ReadText { .. } => "Aa",
            NodeKind::SetVar { .. } => "x=",
            NodeKind::IfCompare { .. } => "?",
            NodeKind::PixelColor { .. } => "◈",
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
            NodeKind::Drag { .. } => 0x0abde3,
            NodeKind::KeyPress { .. } => 0xff9f43,
            NodeKind::TypeText { .. } => 0xf368e0,
            NodeKind::Wait { .. } => 0x9aa0b4,
            NodeKind::Scroll { .. } => 0x54a0ff,
            NodeKind::Recording { .. } => 0xff5e57,
            NodeKind::FindImage { .. } => 0xa55eea,
            NodeKind::Loop { .. } => 0xfeca57,
            NodeKind::WindowCheck { .. } => 0x1dd1a1,
            NodeKind::ReadText { .. } => 0xff7eb6,
            NodeKind::SetVar { .. } => 0x48dbfb,
            NodeKind::IfCompare { .. } => 0xc8d6e5,
            NodeKind::PixelColor { color, .. } => {
                if *color == 0 {
                    0x9aa0b4
                } else {
                    *color
                }
            }
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
            NodeKind::IfCompare { .. } => &["yes", "no"],
            NodeKind::PixelColor { .. } => &["match", "no match"],
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
            NodeKind::Scroll { amount, horizontal, strength } => {
                let dir = match (*horizontal, *amount < 0) {
                    (false, false) => "down",
                    (false, true) => "up",
                    (true, false) => "right",
                    (true, true) => "left",
                };
                let n = amount.unsigned_abs();
                let s = if n == 1 { "" } else { "s" };
                let x = if *strength > 1 { format!(" · ×{strength}") } else { String::new() };
                format!("{n} wheel click{s} {dir}{x}")
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
            NodeKind::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
                duration_ms,
            } => format!(
                "{} ({from_x}, {from_y}) → ({to_x}, {to_y}) · {duration_ms} ms",
                button.label()
            ),
            NodeKind::SetVar { name, expr } => {
                let n = if name.trim().is_empty() { "?" } else { name.trim() };
                format!("{n} = {}", expr.trim())
            }
            NodeKind::IfCompare { left, op, right } => {
                format!("{} {} {}", left.trim(), op.label(), right.trim())
            }
            NodeKind::PixelColor { x, y, color, timeout_ms, .. } => {
                let wait = if *timeout_ms > 0 {
                    format!(" · wait {timeout_ms} ms")
                } else {
                    String::new()
                };
                format!("#{color:06X} at ({x}, {y}){wait}")
            }
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
    /// Your own name for the step (shown instead of its type). Optional, so older
    /// macros still open.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Pause here when the macro reaches this step (debugging).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub breakpoint: bool,
}

impl Node {
    /// What to call this step: its own name, a group's name, or its type.
    pub fn label(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().to_string();
        }
        match &self.kind {
            NodeKind::Group { name, .. } if !name.trim().is_empty() => name.trim().to_string(),
            k => k.title().to_string(),
        }
    }
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
    /// Screen positions are relative to this window (None = plain screen positions).
    pub anchor: Option<WindowAnchor>,
    /// Show a red pop-up when a run ends unexpectedly.
    pub alert_on_stop: bool,
    /// When set, positions are saved for this screen (or followed window) and scaled
    /// to whatever screen / window size is used when the macro runs. None = exact
    /// screen positions (how macros worked before this existed).
    pub reference: Option<ScreenRect>,
    /// Move the mouse along curved, varying-speed paths instead of jumping.
    pub human_mouse: bool,
    /// How fast human-like moves are, in percent (200 = twice as fast).
    pub mouse_speed: u32,
    /// Stop the macro if the mouse is pushed into a corner of the screen.
    pub failsafe: bool,
    /// Show a green pop-up when the macro finishes (an End step, or every repeat done).
    pub alert_on_finish: bool,
    /// Show a pop-up when the macro pauses at a breakpoint.
    pub alert_on_breakpoint: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anchor: Option<WindowAnchor>,
    #[serde(default = "default_true")]
    alert_on_stop: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<ScreenRect>,
    #[serde(default)]
    human_mouse: bool,
    #[serde(default = "default_mouse_speed")]
    mouse_speed: u32,
    #[serde(default = "default_true")]
    failsafe: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    alert_on_finish: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    alert_on_breakpoint: bool,
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
            anchor: f.anchor,
            alert_on_stop: f.alert_on_stop,
            reference: f.reference,
            human_mouse: f.human_mouse,
            mouse_speed: f.mouse_speed,
            failsafe: f.failsafe,
            alert_on_finish: f.alert_on_finish,
            alert_on_breakpoint: f.alert_on_breakpoint,
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
            anchor: m.anchor,
            alert_on_stop: m.alert_on_stop,
            reference: m.reference,
            human_mouse: m.human_mouse,
            mouse_speed: m.mouse_speed,
            failsafe: m.failsafe,
            alert_on_finish: m.alert_on_finish,
            alert_on_breakpoint: m.alert_on_breakpoint,
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
            anchor: None,
            alert_on_stop: true,
            reference: None,
            human_mouse: false,
            mouse_speed: default_mouse_speed(),
            failsafe: true,
            alert_on_finish: false,
            alert_on_breakpoint: true,
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
        name: String::new(),
        breakpoint: false,
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

// ---------------------------------------------------------------------------
// Copy and paste, and moving every position at once
// ---------------------------------------------------------------------------

/// Steps copied to the clipboard (as JSON), so they can be pasted into any macro.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepClip {
    /// Marks the clipboard text as MacroFlow steps.
    pub macroflow_steps: u32,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Macro {
    /// Copy steps (and the connections between them). Start / group start / group end
    /// are left out. All the steps must be in the same place (same group, or top level).
    pub fn copy_steps(&self, ids: &[u64]) -> Option<StepClip> {
        let first = *ids.first()?;
        let g = self.root.owner(first)?;
        let nodes: Vec<Node> = g
            .nodes
            .iter()
            .filter(|n| ids.contains(&n.id) && !n.kind.is_fixed())
            .cloned()
            .collect();
        if nodes.is_empty() {
            return None;
        }
        let keep: Vec<u64> = nodes.iter().map(|n| n.id).collect();
        let edges = g
            .edges
            .iter()
            .copied()
            .filter(|e| keep.contains(&e.from) && keep.contains(&e.to))
            .collect();
        Some(StepClip {
            macroflow_steps: 1,
            nodes,
            edges,
        })
    }

    /// Paste copied steps into the groups listed in `path`, moved by (dx, dy), with new ids.
    /// Returns the new steps' ids.
    pub fn paste_steps(&mut self, path: &[u64], clip: &StepClip, dx: f32, dy: f32) -> Vec<u64> {
        let mut map: HashMap<u64, u64> = HashMap::new();
        let mut new_nodes = Vec::new();
        for n in &clip.nodes {
            if n.kind.is_fixed() {
                continue;
            }
            let mut c = n.clone();
            let id = self.alloc();
            map.insert(n.id, id);
            c.id = id;
            c.x += dx;
            c.y += dy;
            self.refresh_ids(&mut c.kind);
            new_nodes.push(c);
        }
        let new_edges: Vec<Edge> = clip
            .edges
            .iter()
            .filter_map(|e| {
                Some(Edge {
                    from: *map.get(&e.from)?,
                    port: e.port,
                    to: *map.get(&e.to)?,
                })
            })
            .collect();
        let ids: Vec<u64> = new_nodes.iter().map(|n| n.id).collect();
        let path: &[u64] = if self.root.at(path).is_some() { path } else { &[] };
        let g = self.root.at_mut(path).expect("the top level always exists");
        g.nodes.extend(new_nodes);
        g.edges.extend(new_edges);
        ids
    }

    /// Move every saved screen position (clicks, areas, recordings…) by (dx, dy).
    #[allow(dead_code)]
    pub fn shift_positions(&mut self, dx: i32, dy: i32) {
        self.map_positions(&Mapping::translate(dx, dy));
    }

    /// Convert every saved screen position (and area size) with `mapping`.
    pub fn map_positions(&mut self, mapping: &Mapping) {
        if mapping.is_identity() {
            return;
        }
        self.root.for_each_mut(&mut |n: &mut Node| map_kind(&mut n.kind, mapping));
    }
}

/// Move a step's screen positions by (dx, dy).
#[allow(dead_code)]
pub fn shift_kind(kind: &mut NodeKind, dx: i32, dy: i32) {
    map_kind(kind, &Mapping::translate(dx, dy));
}

/// Convert a step's screen positions (and area sizes) with `m`.
pub fn map_kind(kind: &mut NodeKind, m: &Mapping) {
    if m.is_identity() {
        return;
    }
    match kind {
        NodeKind::Click { x, y, area, .. } | NodeKind::MoveMouse { x, y, area, .. } => {
            (*x, *y) = m.point(*x, *y);
            if let Some(r) = area {
                *r = m.rect(*r);
            }
        }
        NodeKind::FindImage { search_area, .. } => {
            if let Some(r) = search_area {
                *r = m.rect(*r);
            }
        }
        NodeKind::ReadText { x, y, w, h, .. } => {
            (*x, *y) = m.point(*x, *y);
            if *w > 0 && *h > 0 {
                (*w, *h) = m.size(*w, *h);
            }
        }
        NodeKind::PixelColor { x, y, .. } => {
            (*x, *y) = m.point(*x, *y);
        }
        NodeKind::Drag { from_x, from_y, to_x, to_y, .. } => {
            (*from_x, *from_y) = m.point(*from_x, *from_y);
            (*to_x, *to_y) = m.point(*to_x, *to_y);
        }
        NodeKind::Recording { events, .. } => {
            for e in events.iter_mut() {
                match &mut e.action {
                    RecAction::Move { x, y } | RecAction::Down { x, y, .. } | RecAction::Up { x, y, .. } => {
                        (*x, *y) = m.point(*x, *y);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Does this step use saved screen positions (so it moves with a followed window)?
pub fn uses_screen_positions(kind: &NodeKind) -> bool {
    match kind {
        NodeKind::Click { target, .. } | NodeKind::MoveMouse { target, .. } => {
            matches!(target, Target::Fixed | Target::Area)
        }
        NodeKind::FindImage { search_area, .. } => search_area.is_some(),
        NodeKind::ReadText { .. } | NodeKind::PixelColor { .. } | NodeKind::Drag { .. } => true,
        NodeKind::Recording { events, .. } => events.iter().any(|e| !matches!(e.action, RecAction::KeyDown { .. } | RecAction::KeyUp { .. })),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Variables and maths
// ---------------------------------------------------------------------------

/// A value worked out by a Set variable / If step.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f64),
    Text(String),
}

impl Value {
    pub fn as_text(&self) -> String {
        match self {
            Value::Num(n) => format_number(*n),
            Value::Text(t) => t.clone(),
        }
    }

    pub fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Text(t) => text_to_number(t),
        }
    }
}

/// 1500.0 → "1500", 2.5 → "2.5", 1/3 → "0.333333".
pub fn format_number(n: f64) -> String {
    if !n.is_finite() {
        return n.to_string();
    }
    if (n - n.round()).abs() < 1e-9 && n.abs() < 1e15 {
        return format!("{}", n.round() as i64);
    }
    let s = format!("{n:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// "1500", " 1,500 ", "1 892 039", "-2.5" → numbers; anything else isn't one.
pub fn text_to_number(t: &str) -> Option<f64> {
    let s: String = t.trim().chars().filter(|c| *c != ',' && *c != ' ').collect();
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

pub fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(f) if f.is_alphabetic() || f == '_') && c.all(|ch| ch.is_alphanumeric() || ch == '_')
}

/// Replace every `{name}` with that variable's value (unknown names become empty).
/// Braces around anything that isn't a name are left alone.
pub fn fill_vars(s: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = after[..close].trim();
                if is_ident(name) {
                    out.push_str(vars.get(name).map(String::as_str).unwrap_or(""));
                } else {
                    out.push('{');
                    out.push_str(&after[..close]);
                    out.push('}');
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
    LParen,
    RParen,
}

fn tokenize(src: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).map_or(false, |d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push(Tok::Num(s.parse().ok()?));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        if c == '{' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '}' {
                j += 1;
            }
            if j >= chars.len() {
                return None;
            }
            let name: String = chars[i + 1..j].iter().collect::<String>().trim().to_string();
            if !is_ident(&name) {
                return None;
            }
            out.push(Tok::Ident(name));
            i = j + 1;
            continue;
        }
        out.push(match c {
            '+' | '-' | '*' | '/' | '%' => Tok::Op(c),
            '×' => Tok::Op('*'),
            '÷' => Tok::Op('/'),
            '−' => Tok::Op('-'),
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            _ => return None,
        });
        i += 1;
    }
    Some(out)
}

enum EvalErr {
    /// Not a sum at all: treat it as text.
    Syntax,
    /// A sum that can't be worked out (e.g. dividing by zero).
    Math(String),
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
    vars: &'a HashMap<String, String>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn expr(&mut self) -> Result<f64, EvalErr> {
        let mut v = self.term()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(op)) if *op == '+' || *op == '-' => *op,
                _ => return Ok(v),
            };
            self.pos += 1;
            let r = self.term()?;
            v = if op == '+' { v + r } else { v - r };
        }
    }

    fn term(&mut self) -> Result<f64, EvalErr> {
        let mut v = self.factor()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(op)) if *op == '*' || *op == '/' || *op == '%' => *op,
                _ => return Ok(v),
            };
            self.pos += 1;
            let r = self.factor()?;
            v = match op {
                '*' => v * r,
                _ if r == 0.0 => return Err(EvalErr::Math("can't divide by zero".into())),
                '/' => v / r,
                _ => v % r,
            };
        }
    }

    fn factor(&mut self) -> Result<f64, EvalErr> {
        let tok = self.peek().cloned().ok_or(EvalErr::Syntax)?;
        self.pos += 1;
        match tok {
            Tok::Op('-') => Ok(-self.factor()?),
            Tok::Op('+') => self.factor(),
            Tok::Num(n) => Ok(n),
            // Variables that haven't been set count as 0, so counters work straight away.
            Tok::Ident(name) => match self.vars.get(&name) {
                None => Ok(0.0),
                Some(t) => text_to_number(t)
                    .ok_or_else(|| EvalErr::Math(format!("“{name}” is “{t}”, which isn't a number"))),
            },
            Tok::LParen => {
                let v = self.expr()?;
                if self.peek() == Some(&Tok::RParen) {
                    self.pos += 1;
                    Ok(v)
                } else {
                    Err(EvalErr::Syntax)
                }
            }
            _ => Err(EvalErr::Syntax),
        }
    }
}

/// Work out a value: a sum like `gold + 1000` or `(a + b) / 2`, a single variable
/// (`gold`), a number, or otherwise text, where `{name}` is replaced by that variable.
pub fn eval_expr(src: &str, vars: &HashMap<String, String>) -> Result<Value, String> {
    let s = src.trim();
    if s.is_empty() {
        return Ok(Value::Text(String::new()));
    }
    if let Some(toks) = tokenize(s) {
        // A single name: a variable's own value, or (if it isn't set) just that word.
        if let [Tok::Ident(name)] = toks.as_slice() {
            return Ok(match vars.get(name) {
                Some(t) => text_to_number(t).map(Value::Num).unwrap_or_else(|| Value::Text(t.clone())),
                None if s.starts_with('{') => Value::Text(String::new()),
                None => Value::Text(name.clone()),
            });
        }
        let mut p = Parser { toks: &toks, pos: 0, vars };
        match p.expr() {
            Ok(v) if p.pos == toks.len() => return Ok(Value::Num(v)),
            Err(EvalErr::Math(e)) => return Err(e),
            _ => {}
        }
    }
    Ok(Value::Text(fill_vars(s, vars)))
}

// ---------------------------------------------------------------------------
// Finding variables in text (so the editor can show them as chips)
// ---------------------------------------------------------------------------

/// Variables every run has without setting them.
pub const BUILTIN_VARS: [&str; 4] = ["text", "run", "found_x", "found_y"];

/// How a piece of text uses variables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarMode {
    /// Ordinary text: only `{name}` is a variable.
    Text,
    /// A value / sum (Set variable, If): bare names in a sum are variables too.
    Expr,
    /// The whole thing is a variable's name.
    Name,
}

/// Where a variable appears in some text, in characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarRange {
    pub start: usize,
    pub end: usize,
    pub name: String,
    /// Written as `{name}` (rather than a bare name in a sum).
    pub braced: bool,
}

/// A piece of text to show: plain, or a variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Var(String),
}

/// Is this a sum (like `gold + 1000`) rather than plain text?
fn is_sum(s: &str) -> bool {
    let Some(toks) = tokenize(s.trim()) else { return false };
    if toks.len() < 2 {
        return false;
    }
    // Give every name a value, so only the shape of the sum matters.
    let ones: HashMap<String, String> = toks
        .iter()
        .filter_map(|t| match t {
            Tok::Ident(n) => Some((n.clone(), "1".to_string())),
            _ => None,
        })
        .collect();
    let mut p = Parser { toks: &toks, pos: 0, vars: &ones };
    match p.expr() {
        Ok(_) => p.pos == toks.len(),
        Err(EvalErr::Math(_)) => true,
        Err(EvalErr::Syntax) => false,
    }
}

fn braced_ranges(chars: &[char]) -> Vec<VarRange> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            if let Some(off) = chars[i + 1..].iter().position(|c| *c == '}' || *c == '{') {
                let j = i + 1 + off;
                if chars[j] == '}' {
                    let name: String = chars[i + 1..j].iter().collect::<String>().trim().to_string();
                    if is_ident(&name) {
                        out.push(VarRange { start: i, end: j + 1, name, braced: true });
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// Find the variables in `src`. `known` says whether a name is a variable the macro
/// sets (used to tell a lone word like `gold` from plain text like `Complete`).
pub fn var_ranges(src: &str, mode: VarMode, known: &dyn Fn(&str) -> bool) -> Vec<VarRange> {
    let chars: Vec<char> = src.chars().collect();
    let trimmed_range = || {
        let start = chars.iter().position(|c| !c.is_whitespace())?;
        let end = chars.iter().rposition(|c| !c.is_whitespace())? + 1;
        Some((start, end, chars[start..end].iter().collect::<String>()))
    };
    match mode {
        VarMode::Text => braced_ranges(&chars),
        VarMode::Name => match trimmed_range() {
            Some((start, end, name)) if is_ident(&name) => vec![VarRange { start, end, name, braced: false }],
            _ => vec![],
        },
        VarMode::Expr => {
            if is_sum(src) {
                let mut out = braced_ranges(&chars);
                let mut i = 0;
                while i < chars.len() {
                    if let Some(r) = out.iter().find(|r| r.start == i) {
                        i = r.end;
                        continue;
                    }
                    let c = chars[i];
                    let after_digit = i > 0 && (chars[i - 1].is_ascii_digit() || chars[i - 1] == '.');
                    if (c.is_alphabetic() || c == '_') && !after_digit {
                        let start = i;
                        while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                            i += 1;
                        }
                        let name: String = chars[start..i].iter().collect();
                        out.push(VarRange { start, end: i, name, braced: false });
                        continue;
                    }
                    i += 1;
                }
                out.sort_by_key(|r| r.start);
                out
            } else {
                match trimmed_range() {
                    Some((start, end, name)) if is_ident(&name) && known(&name) => {
                        vec![VarRange { start, end, name, braced: false }]
                    }
                    _ => braced_ranges(&chars),
                }
            }
        }
    }
}

/// Split text into plain pieces and variables, for showing it.
pub fn var_spans(src: &str, mode: VarMode, known: &dyn Fn(&str) -> bool) -> Vec<Span> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    for r in var_ranges(src, mode, known) {
        if r.start > at {
            out.push(Span::Text(chars[at..r.start].iter().collect()));
        }
        out.push(Span::Var(r.name));
        at = r.end;
    }
    if at < chars.len() {
        out.push(Span::Text(chars[at..].iter().collect()));
    }
    out
}

impl NodeKind {
    /// The step's summary split into text and variables, for steps that use them
    /// (None = just show `summary()`).
    pub fn summary_spans(&self, known: &dyn Fn(&str) -> bool) -> Option<Vec<Span>> {
        let t = |s: &str| Span::Text(s.to_string());
        let mut out = Vec::new();
        match self {
            NodeKind::SetVar { name, expr } => {
                let n = name.trim();
                if is_ident(n) {
                    out.push(Span::Var(n.to_string()));
                } else {
                    out.push(t(if n.is_empty() { "?" } else { n }));
                }
                out.push(t(" = "));
                out.extend(var_spans(expr.trim(), VarMode::Expr, known));
            }
            NodeKind::IfCompare { left, op, right } => {
                out.extend(var_spans(left.trim(), VarMode::Expr, known));
                out.push(t(&format!(" {} ", op.label())));
                out.extend(var_spans(right.trim(), VarMode::Expr, known));
            }
            NodeKind::TypeText { text, press_enter } => {
                if !text.contains('{') {
                    return None;
                }
                out.push(t("“"));
                out.extend(var_spans(text, VarMode::Text, known));
                out.push(t(if *press_enter { "” ↩" } else { "”" }));
            }
            NodeKind::Notify { title, message, style, duration_ms, wait_for_click, .. } => {
                let text = if !title.trim().is_empty() { title.trim() } else { message.trim() };
                if !text.contains('{') {
                    return None;
                }
                out.push(t(&format!("{} · “", style.label())));
                out.extend(var_spans(text, VarMode::Text, known));
                out.push(t(&if *wait_for_click {
                    "” · waits for a click".to_string()
                } else {
                    format!("” · {:.1}s", *duration_ms as f32 / 1000.0)
                }));
            }
            NodeKind::ReadText { save_to, value, .. } => {
                let save = save_to.trim();
                if !is_ident(save) && !value.contains('{') {
                    return None;
                }
                let summary = self.summary();
                if value.contains('{') {
                    // Show {name} in "Compare with" as a chip too.
                    out.extend(var_spans(&summary, VarMode::Text, known));
                } else {
                    out.push(Span::Text(summary));
                }
                if is_ident(save) {
                    out.push(t(" → "));
                    out.push(Span::Var(save.to_string()));
                }
            }
            _ => return None,
        }
        Some(out)
    }
}

impl Macro {
    /// Every variable some step sets (Set variable, or Read text's "save into").
    pub fn variables_set(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        let mut copy = self.root.clone();
        copy.for_each_mut(&mut |n: &mut Node| match &n.kind {
            NodeKind::SetVar { name, .. } if is_ident(name.trim()) => {
                out.insert(name.trim().to_string());
            }
            NodeKind::ReadText { save_to, .. } if is_ident(save_to.trim()) => {
                out.insert(save_to.trim().to_string());
            }
            _ => {}
        });
        out
    }

    /// Every variable some step reads.
    pub fn variables_used(&self) -> std::collections::BTreeSet<String> {
        let set = self.variables_set();
        let known = |n: &str| set.contains(n) || BUILTIN_VARS.contains(&n);
        let mut out = std::collections::BTreeSet::new();
        let mut copy = self.root.clone();
        copy.for_each_mut(&mut |n: &mut Node| {
            let texts: Vec<(&str, VarMode)> = match &n.kind {
                NodeKind::SetVar { expr, .. } => vec![(expr.as_str(), VarMode::Expr)],
                NodeKind::IfCompare { left, right, .. } => {
                    vec![(left.as_str(), VarMode::Expr), (right.as_str(), VarMode::Expr)]
                }
                NodeKind::TypeText { text, .. } => vec![(text.as_str(), VarMode::Text)],
                NodeKind::Notify { title, message, .. } => {
                    vec![(title.as_str(), VarMode::Text), (message.as_str(), VarMode::Text)]
                }
                NodeKind::ReadText { value, .. } => vec![(value.as_str(), VarMode::Text)],
                _ => vec![],
            };
            for (s, mode) in texts {
                for r in var_ranges(s, mode, &known) {
                    out.insert(r.name);
                }
            }
        });
        out
    }
}

/// Compare two values. Numbers compare as numbers; otherwise as text (ignoring case).
pub fn compare_values(left: &Value, op: CmpOp, right: &Value) -> bool {
    if let (Some(a), Some(b)) = (left.as_num(), right.as_num()) {
        let eps = 1e-9;
        return match op {
            CmpOp::Eq => (a - b).abs() < eps,
            CmpOp::Ne => (a - b).abs() >= eps,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b - eps,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b + eps,
            CmpOp::Contains => format_number(a).contains(&format_number(b)),
            CmpOp::NotContains => !format_number(a).contains(&format_number(b)),
        };
    }
    let a = left.as_text().trim().to_lowercase();
    let b = right.as_text().trim().to_lowercase();
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Ne => a != b,
        CmpOp::Gt => a > b,
        CmpOp::Ge => a >= b,
        CmpOp::Lt => a < b,
        CmpOp::Le => a <= b,
        CmpOp::Contains => a.contains(&b),
        CmpOp::NotContains => !a.contains(&b),
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
    fn variables_and_maths() {
        let mut vars = HashMap::new();
        vars.insert("gold".to_string(), "1500".to_string());
        vars.insert("name".to_string(), "Bob".to_string());
        let ev = |s: &str, v: &HashMap<String, String>| eval_expr(s, v).unwrap();
        assert_eq!(ev("gold + 1000", &vars), Value::Num(2500.0));
        assert_eq!(ev("(gold - 500) / 2", &vars), Value::Num(500.0));
        assert_eq!(ev("count + 1", &vars), Value::Num(1.0)); // unset counts as 0
        assert_eq!(ev("gold", &vars), Value::Num(1500.0));
        assert_eq!(ev("name", &vars), Value::Text("Bob".into()));
        assert_eq!(ev("Hello {name}!", &vars), Value::Text("Hello Bob!".into()));
        assert_eq!(ev("-3 * 2", &vars), Value::Num(-6.0));
        assert!(eval_expr("gold / 0", &vars).is_err());
        assert!(eval_expr("name + 1", &vars).is_err());
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(1500.0), "1500");
        assert!(compare_values(&Value::Text("1 500".into()), CmpOp::Ge, &Value::Num(1000.0)));
        assert!(compare_values(&Value::Text("Upload complete".into()), CmpOp::Contains, &Value::Text("COMPLETE".into())));
        assert_eq!(fill_vars("{gold} gold, {missing}!", &vars), "1500 gold, !");
    }

    #[test]
    fn old_scroll_steps_and_alerts_still_load() {
        let old = r#"{"id":1,"kind":{"type":"Scroll","amount":5,"horizontal":false},"x":0.0,"y":0.0,"delay_ms":0,"note":""}"#;
        let n: Node = serde_json::from_str(old).unwrap();
        assert!(matches!(n.kind, NodeKind::Scroll { amount: 5, strength: 1, .. }));
        // Saved back the same way (no new keys for the defaults).
        assert!(!serde_json::to_string(&n).unwrap().contains("strength"));
        let m = Macro::default();
        let json = serde_json::to_string(&MacroFile::from(m)).unwrap();
        assert!(!json.contains("alert_on_finish") && !json.contains("alert_on_breakpoint"));
        let back: Macro = MacroFile::from(Macro::default()).into();
        assert!(back.alert_on_breakpoint && !back.alert_on_finish);
    }

    #[test]
    fn finds_variables_in_text() {
        let known = |n: &str| n == "gold" || n == "text";
        let names = |s: &str, m: VarMode| -> Vec<String> {
            var_ranges(s, m, &known).into_iter().map(|r| r.name).collect()
        };
        assert_eq!(names("Gold is {gold}!", VarMode::Text), vec!["gold"]);
        assert_eq!(names("{ gold } and {not a var} {x", VarMode::Text), vec!["gold"]);
        assert_eq!(names("gold + bonus * 2", VarMode::Expr), vec!["gold", "bonus"]);
        assert_eq!(names("gold", VarMode::Expr), vec!["gold"]); // a known variable
        assert!(names("Complete", VarMode::Expr).is_empty()); // just a word
        assert!(names("Level Complete", VarMode::Expr).is_empty()); // words, not a sum
        assert_eq!(names("Hello {text}", VarMode::Expr), vec!["text"]);
        assert_eq!(names(" gold_now ", VarMode::Name), vec!["gold_now"]);
        assert_eq!(
            var_spans("Gold: {gold}", VarMode::Text, &known),
            vec![Span::Text("Gold: ".into()), Span::Var("gold".into())]
        );
        let r = &var_ranges("a{gold}b", VarMode::Text, &known)[0];
        assert_eq!((r.start, r.end, r.braced), (1, 7, true));
    }

    #[test]
    fn copy_paste_and_shift() {
        let mut m = Macro::default();
        let a = m.add_node(NodeKind::Wait { ms: 1, jitter_ms: 0 }, 300.0, 180.0);
        let b = m.add_node(
            NodeKind::PixelColor { x: 5, y: 6, color: 0xff0000, tolerance: 10, timeout_ms: 0, interval_ms: 100 },
            600.0,
            180.0,
        );
        m.connect(a, 0, b);
        let clip = m.copy_steps(&[a, b]).unwrap();
        let json = serde_json::to_string(&clip).unwrap();
        let back: StepClip = serde_json::from_str(&json).unwrap();
        let ids = m.paste_steps(&[], &back, 40.0, 40.0);
        assert_eq!(ids.len(), 2);
        assert_eq!(m.next(ids[0], 0), Some(ids[1]));
        m.shift_positions(10, -5);
        assert!(matches!(m.node(b).unwrap().kind, NodeKind::PixelColor { x: 15, y: 1, .. }));
    }

    #[test]
    fn new_settings_have_safe_defaults_in_old_files() {
        let m: Macro = serde_json::from_str(include_str!("../examples/hello.json")).unwrap();
        assert!(!m.human_mouse);
        assert_eq!(m.mouse_speed, 100);
        assert!(m.failsafe);
        assert!(m.root.nodes.iter().all(|n| !n.breakpoint));
        // A step without a breakpoint saves exactly as before.
        assert!(!serde_json::to_string(&m.root.nodes[0]).unwrap().contains("breakpoint"));
        let d: NodeKind = serde_json::from_str(r#"{"type":"Drag","from_x":1,"from_y":2,"to_x":3,"to_y":4}"#).unwrap();
        assert!(matches!(d, NodeKind::Drag { duration_ms: 400, button: MouseBtn::Left, .. }));
    }

    #[test]
    fn positions_scale_between_screens() {
        let from = ScreenRect { x: 0, y: 0, w: 1920, h: 1080 };
        let to = ScreenRect { x: 0, y: 0, w: 2560, h: 1440 };
        let m = Mapping { from, to };
        assert_eq!(m.point(960, 540), (1280, 720)); // the middle stays the middle
        assert_eq!(m.point(1919, 0), (2559, 0));
        assert_eq!(m.inverse().point(1280, 720), (960, 540));
        let mut k = NodeKind::ReadText {
            x: 100, y: 100, w: 200, h: 50,
            compare: TextCompare::Contains, value: String::new(), case_sensitive: false,
            timeout_ms: 0, interval_ms: 500, preview: None, numbers_only: false, save_to: String::new(),
        };
        map_kind(&mut k, &m);
        assert!(matches!(k, NodeKind::ReadText { x: 133, y: 133, w: 267, h: 67, .. }));
        assert!(Mapping::translate(0, 0).is_identity());
        assert_eq!(Mapping::translate(5, -3).point(10, 10), (15, 7));
        // Old macros don't scale, and save exactly as before.
        let old: Macro = serde_json::from_str(include_str!("../examples/hello.json")).unwrap();
        assert!(old.reference.is_none());
        assert!(serde_json::to_value(&old).unwrap().get("reference").is_none());
    }

    #[test]
    fn old_macros_have_no_anchor_and_alerts_on() {
        let m: Macro = serde_json::from_str(include_str!("../examples/hello.json")).unwrap();
        assert!(m.anchor.is_none());
        assert!(m.alert_on_stop);
        let v: serde_json::Value = serde_json::to_value(&m).unwrap();
        assert!(v.get("anchor").is_none(), "no anchor saved for macros that don't use one");
    }

    #[test]
    fn steps_without_a_name_still_open() {
        let old = r#"{"id":3,"kind":{"type":"End"},"x":1.0,"y":2.0,"delay_ms":0,"note":""}"#;
        let n: Node = serde_json::from_str(old).expect("old step should load");
        assert!(n.name.is_empty());
        assert_eq!(n.label(), "End");
        // An unnamed step saves exactly as before.
        assert!(!serde_json::to_string(&n).unwrap().contains("name"));
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
