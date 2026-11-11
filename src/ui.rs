//! The MacroFlow window: toolbar, step palette, flowchart canvas and inspector.

use crate::engine::{self, AreaPurpose, EndKind, Hotkey, LogEntry, LogLevel, RunEnd, WorkerMsg};
use crate::keys;
use crate::marker;
use crate::model::*;
use crate::vision;
use gpui::{
    canvas, div, fill, img, point, prelude::*, px, rgb, rgba, AnyElement, Bounds, ClickEvent,
    Context, Div, ElementId, FocusHandle, FontWeight, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, PathBuilder, PathPromptOptions,
    Pixels, Point, PromptLevel, ScrollDelta, ScrollWheelEvent, SharedString, Stateful, Subscription,
    Task, Window, WindowControlArea, ClipboardItem, Font, TextRun, App, AnyView,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

const BG: u32 = 0x0f1117;
const PANEL2: u32 = 0x1d212c;
const CANVAS_BG: u32 = 0x12141b;
const GRID: u32 = 0x1a1d27;
const NODE_BG: u32 = 0x1c2030;
const BORDER: u32 = 0x2a2f3d;
const TEXT: u32 = 0xe6e8ef;
const MUTED: u32 = 0x8b91a3;
const ACCENT: u32 = 0x7c5cff;
const BTN: u32 = 0x252a38;
const BTN_HOVER: u32 = 0x31374a;
const RED: u32 = 0xef5b5b;
const GREEN: u32 = 0x34c77b;
const YELLOW: u32 = 0xf5b84a;
const PORT_BG: u32 = 0x0f1117;

// Chrome (everything around the flowchart). The canvas and step cards keep the
// colours above, so restyling the panels never changes how the flowchart looks.
/// Title bar, status bar and sidebar background.
const UI_BAR: u32 = 0x111319;
/// Raised surfaces: cards, menus, popovers.
const UI_RAISED: u32 = 0x1a1d26;
/// Text boxes and wells.
const UI_INPUT: u32 = 0x0c0e13;
/// Hairlines between areas.
const UI_LINE: u32 = 0x23262f;
/// Borders of controls.
const UI_EDGE: u32 = 0x2c303b;
/// Hover fill for quiet controls.
const UI_HOVER: u32 = 0x232733;
/// Third-level text: hints, captions.
const UI_FAINT: u32 = 0x666c7e;

/// Height of the custom title bar (the system one is transparent, so we draw our own).
pub const TITLEBAR_H: f32 = 40.0;
const IS_MACOS: bool = cfg!(target_os = "macos");
/// Room for the macOS traffic lights before the app name.
const NAME_INSET: f32 = if IS_MACOS { 80.0 } else { 14.0 };
const SIDEBAR_W: f32 = 340.0;

fn hsla(hex: u32) -> Hsla {
    rgb(hex).into()
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Idle,
    Running,
    Recording,
    RecordingStep,
    Picking(u64),
    Snipping(u64),
    Testing,
    Capturing,
    Working,
}

/// What the mouse is currently dragging on the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    Node { id: u64, dx: f32, dy: f32 },
    Pan { x: f32, y: f32 },
    Connect { from: u64, port: usize },
    /// Shift-drag on empty canvas: draw a box to select the steps inside it.
    Select { x: f32, y: f32 },
}

/// What a right-click menu is for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MenuTarget {
    Step(u64),
    /// Empty canvas; (x, y) is the spot in canvas coordinates.
    Canvas { x: f32, y: f32 },
}

#[derive(Clone, Debug)]
struct ContextMenu {
    /// Where it opened, in window coordinates.
    x: f32,
    y: f32,
    target: MenuTarget,
    /// The "Add step here" list is showing.
    add_open: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum MenuAction {
    Edit(u64),
    Rename(u64),
    RunFrom(u64),
    Duplicate,
    Group,
    OpenGroup(u64),
    Ungroup,
    Disconnect(u64),
    Delete,
    ClearSelection,
    AddHere(usize, f32, f32),
    SelectAll,
    ZoomFit,
    ZoomReset,
    FullScreen,
    LeaveGroup,
    Copy,
    Cut,
    Paste(Option<(f32, f32)>),
    Undo,
    Redo,
    ToggleBreakpoint(u64),
    /// Disable / enable the selected steps.
    ToggleDisabled,
}

enum MenuEntry {
    Header(String),
    Item {
        label: String,
        hint: &'static str,
        action: MenuAction,
        enabled: bool,
        danger: bool,
    },
    /// Opens the "Add step here" list.
    AddSub,
    Sep,
}

const ZOOM_MIN: f32 = 0.25;
const ZOOM_MAX: f32 = 2.0;
const MENU_W: f32 = 240.0;
const MENU_ITEM_H: f32 = 28.0;
const MENU_SEP_H: f32 = 9.0;
const MENU_HEADER_H: f32 = 26.0;

/// The keyboard shortcut shown in menus, for macOS and for Windows.
const fn hint(mac: &'static str, win: &'static str) -> &'static str {
    if IS_MACOS {
        mac
    } else {
        win
    }
}

/// Which page the left sidebar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideTab {
    Steps,
    Edit,
    Macro,
    Log,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    MacroName,
    StartDelay,
    StepDelay,
    Speed,
    Repeat,
    BulkDelay,
    SnipSize,
    Note,
    Delay,
    X,
    Y,
    OffX,
    OffY,
    Clicks,
    Text,
    WaitMs,
    Jitter,
    ScrollAmt,
    RecSpeed,
    RecDelay(usize),
    Threshold,
    Timeout,
    Interval,
    LoopCount,
    W,
    H,
    CompareValue,
    WinText,
    GroupName,
    StepName,
    ToastTitle,
    ToastText,
    ToastMs,
    ClickJitter,
    VarName,
    VarExpr,
    IfLeft,
    IfRight,
    ReadSaveTo,
    PixelHex,
    PixelTol,
    DragFromX,
    DragFromY,
    DragToX,
    DragToY,
    DragMs,
    DragTaps,
    AreaRotation,
    AreaHollow,
    DragHold,
    StepMouseSpeed,
    MouseSpeed,
    ScrollStrength,
}

impl Field {
    fn is_text(self) -> bool {
        matches!(
            self,
            Field::MacroName
                | Field::Note
                | Field::Text
                | Field::CompareValue
                | Field::WinText
                | Field::GroupName
                | Field::StepName
                | Field::VarName
                | Field::VarExpr
                | Field::IfLeft
                | Field::IfRight
                | Field::ReadSaveTo
                | Field::PixelHex
                | Field::ToastTitle
                | Field::ToastText
        )
    }
    fn is_decimal(self) -> bool {
        matches!(self, Field::Threshold | Field::RecSpeed)
    }
}

pub struct MacroApp {
    focus: FocusHandle,
    doc: Macro,
    path: Option<PathBuf>,
    dirty: bool,
    selected: Option<u64>,
    pan: (f32, f32),
    canvas_origin: Rc<Cell<(f32, f32)>>,
    /// Size of the canvas on screen (updated every frame).
    canvas_size: Rc<Cell<(f32, f32)>>,
    /// Canvas zoom: 1.0 = 100%. Screen position = canvas position × zoom + pan.
    zoom: f32,
    /// The open right-click menu, if any.
    menu: Option<ContextMenu>,
    drag: Drag,
    mouse: (f32, f32),
    editing: Option<(Field, String)>,
    /// Text cursor in the field being edited (in characters).
    edit_cursor: usize,
    /// The other end of the selection (Shift + arrows, Ctrl+A, dragging).
    edit_anchor: Option<usize>,
    /// Dragging the mouse to select text in a field.
    field_drag: bool,
    /// Where each text box's text starts on screen, and its font, so a click can
    /// put the cursor between the right letters.
    field_geom: Rc<RefCell<HashMap<Field, (f32, Font, f32)>>>,
    /// Variables some step sets, plus the built-in ones (refreshed every frame).
    vars_known: std::collections::BTreeSet<String>,
    /// What each Read text / Find image step saw the last time it ran (or was tested).
    seen: HashMap<u64, SeenShot>,
    /// Show the original picture (the area when it was selected / the image to find)
    /// instead of what the step last saw.
    show_original: bool,
    /// What each step did on the last run.
    results: HashMap<u64, CardResult>,
    /// Variables and their values during / after the last run.
    live_vars: Vec<(String, String)>,
    /// The variables panel is showing.
    show_vars: bool,
    quick: Option<QuickAdd>,
    run_check: Option<RunCheck>,
    /// The run was started with F8 (so MacroFlow may be hidden).
    hotkey_start: bool,
    /// Editing a step's random area on top of a screenshot.
    shape_edit: Option<ShapeEdit>,
    /// The run-log line being looked at (its picture shows on the canvas).
    log_sel: Option<usize>,
    /// Playing the run log back.
    replay_playing: bool,
    /// Playback speed: 1, 2 or 4 ×.
    replay_speed: u32,
    /// When playback moves to the next line.
    replay_next: Option<Instant>,
    /// The step the selected log line belongs to (outlined like a running step).
    replay_node: Option<u64>,
    /// Also flash the green dot on the real screen when a log line is picked.
    log_on_screen: bool,
    /// Problems with each step (refreshed every frame).
    problems: HashMap<u64, StepProblems>,
    /// The start screen (templates, recent macros) is showing.
    show_start: bool,
    recent: Vec<PathBuf>,
    /// Unsaved work found from last time (MacroFlow closed without saving it).
    autosave_found: Option<Autosave>,
    last_autosave: Instant,
    capturing_key: bool,
    mode: Mode,
    status: String,
    active_node: Option<u64>,
    record_moves: bool,
    bulk_delay: u64,
    snip_size: u32,
    pending_area: Option<AreaPurpose>,
    /// The user already chose "Don't Save" (or saved), so let the window close.
    close_confirmed: bool,
    close_prompt_open: bool,
    side_tab: SideTab,
    sidebar_open: bool,
    /// The Tips & shortcuts list at the bottom of the Steps tab is open.
    tips_open: bool,
    /// The groups you've opened, outermost first. Empty = the top level of the macro.
    view: Vec<u64>,
    /// Extra steps selected with Shift-click (on top of `selected`).
    extra: Vec<u64>,
    windows: Vec<vision::WinInfo>,
    /// The open-windows list is for choosing the window the macro follows.
    picking_anchor: bool,
    /// Pause playback while set (F9 / the Pause button).
    paused: Arc<AtomicBool>,
    /// Pause before every step while set (step-by-step mode, F10).
    step_mode: Arc<AtomicBool>,
    /// Which end of a Drag step "Pick" sets (false = start, true = end).
    drag_pick_end: bool,
    /// Undo / redo: earlier and later versions of the macro.
    undo: Vec<Macro>,
    redo: Vec<Macro>,
    /// The macro as it was after the last change that was recorded for undo.
    history_head: Macro,
    /// Lines from the last run.
    log: Vec<LogEntry>,
    /// How many times the same steps have been pasted (each paste lands a bit further on).
    paste_count: u32,
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
    stop: Arc<AtomicBool>,
    _poll: Task<()>,
    /// Watches for MacroFlow becoming the active window again (cancels a selection).
    _activation: Subscription,
    /// When MacroFlow minimised itself to let you select an area.
    capture_started: Option<Instant>,
    /// MacroFlow minimised itself for "Read now" / "Test now": bring it back afterwards.
    restore_after_test: bool,
    /// The pop-up shown by "Preview" (a new preview replaces it).
    preview_toast: Option<std::process::Child>,
    /// Why the last run ended; its step is highlighted in red until the next run.
    last_end: Option<RunEnd>,
}

impl MacroApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        let (tx, rx) = channel();
        // Ask to save unsaved changes before the window closes.
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |app, cx| app.confirm_close(AfterSave::CloseWindow, window, cx))
                .unwrap_or(true)
        });
        // Pick up results from the worker threads ~30 times a second.
        // Clicking back on MacroFlow while the area selector is open cancels it.
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            let recently_started = this
                .capture_started
                .map_or(true, |t| t.elapsed() < Duration::from_millis(1500));
            if window.is_window_active() && this.mode == Mode::Capturing && !recently_started {
                this.stop.store(true, Ordering::Relaxed);
                this.status = "Selection cancelled.".into();
                cx.notify();
            }
        });
        vision::warm_up_selector();
        engine::spawn_hotkeys(tx.clone());
        let poll = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(33))
                .await;
            if this
                .update_in(cx, |app, window, cx| app.drain(window, cx))
                .is_err()
            {
                break;
            }
        });
        let first_doc = fresh_macro();
        MacroApp {
            focus,
            doc: first_doc.clone(),
            path: None,
            dirty: false,
            selected: None,
            pan: (0.0, 0.0),
            canvas_origin: Rc::new(Cell::new((0.0, 0.0))),
            canvas_size: Rc::new(Cell::new((0.0, 0.0))),
            zoom: 1.0,
            menu: None,
            drag: Drag::None,
            mouse: (0.0, 0.0),
            editing: None,
            edit_cursor: 0,
            edit_anchor: None,
            field_drag: false,
            field_geom: Rc::new(RefCell::new(HashMap::new())),
            vars_known: Default::default(),
            seen: {
                // Pictures from an earlier session aren't needed any more.
                let _ = std::fs::remove_dir_all(engine::seen_dir());
                HashMap::new()
            },
            show_original: false,
            results: HashMap::new(),
            log_sel: None,
            shape_edit: None,
            replay_playing: false,
            replay_speed: 1,
            replay_next: None,
            replay_node: None,
            log_on_screen: true,
            live_vars: Vec::new(),
            show_vars: true,
            quick: None,
            run_check: None,
            hotkey_start: false,
            problems: HashMap::new(),
            show_start: true,
            recent: recent_macros(),
            autosave_found: read_autosave().filter(|a| a.doc.root.nodes.len() > 1),
            last_autosave: Instant::now(),
            capturing_key: false,
            mode: Mode::Idle,
            status: "Ready.".into(),
            active_node: None,
            record_moves: true,
            bulk_delay: 100,
            snip_size: 64,
            pending_area: None,
            close_confirmed: false,
            close_prompt_open: false,
            side_tab: SideTab::Steps,
            sidebar_open: true,
            tips_open: false,
            view: Vec::new(),
            extra: Vec::new(),
            windows: Vec::new(),
            picking_anchor: false,
            paused: Arc::new(AtomicBool::new(false)),
            step_mode: Arc::new(AtomicBool::new(false)),
            drag_pick_end: false,
            undo: Vec::new(),
            redo: Vec::new(),
            history_head: first_doc,
            log: Vec::new(),
            paste_count: 0,
            tx,
            rx,
            stop: Arc::new(AtomicBool::new(false)),
            _poll: poll,
            _activation: activation,
            capture_started: None,
            restore_after_test: false,
            preview_toast: None,
            last_end: None,
        }
    }

    // -----------------------------------------------------------------------
    // Worker messages
    // -----------------------------------------------------------------------

    fn drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                WorkerMsg::Status(s) => self.status = s,
                WorkerMsg::Active(id) => self.active_node = id,
                WorkerMsg::RunFinished(end) => {
                    self.mode = Mode::Idle;
                    self.active_node = None;
                    self.paused.store(false, Ordering::Relaxed);
                    self.step_mode.store(false, Ordering::Relaxed);
                    self.status = end.message.clone();
                    self.last_end = Some(end);
                }
                WorkerMsg::Log(entry) => {
                    if self.log.len() < 5000 {
                        self.log.push(entry);
                    }
                }
                WorkerMsg::Hotkey(Hotkey::StartStop) => match self.mode {
                    Mode::Running => self.stop_worker(),
                    Mode::Idle => {
                        self.hotkey_start = true;
                        self.start_run();
                    }
                    _ => {}
                },
                WorkerMsg::Hotkey(Hotkey::Pause) => self.toggle_pause(),
                WorkerMsg::Hotkey(Hotkey::Step) => self.step_once(),
                WorkerMsg::Pixel { x, y, color } => {
                    if let Mode::Picking(id) = self.mode {
                        let (ax, ay) = self.to_anchor(x, y);
                        self.update_node(id, |k| {
                            if let NodeKind::PixelColor { x: px_, y: py_, color: c, .. } = k {
                                *px_ = ax;
                                *py_ = ay;
                                *c = color;
                            }
                        });
                        self.status = format!("Picked #{color:06X} at ({x}, {y}).");
                    }
                    self.mode = Mode::Idle;
                }
                WorkerMsg::Recorded(events) => {
                    self.mode = Mode::Idle;
                    if events.is_empty() {
                        self.status = "Nothing was recorded.".into();
                    } else {
                        let n = events.len();
                        let mut kind = NodeKind::Recording { events, speed: 1.0 };
                        let saved = self.saved_mapping();
                        map_kind(&mut kind, &saved);
                        self.add_step(kind);
                        self.status = format!("Recorded {n} events. Edit delays in the panel on the right.");
                    }
                }
                WorkerMsg::StepRecorded(kind) => {
                    self.mode = Mode::Idle;
                    let title = kind.title();
                    let mut kind = kind;
                    let saved = self.saved_mapping();
                    map_kind(&mut kind, &saved);
                    self.add_step(kind);
                    self.status = format!("Added a {title} step.");
                }
                WorkerMsg::Position { x, y } => {
                    if let Mode::Picking(id) = self.mode {
                        let (x, y) = self.to_anchor(x, y);
                        if let Some(n) = self.doc.node_mut(id) {
                            match &mut n.kind {
                                NodeKind::Click { target, x: nx, y: ny, .. }
                                | NodeKind::MoveMouse { target, x: nx, y: ny, .. } => {
                                    *target = Target::Fixed;
                                    *nx = x;
                                    *ny = y;
                                }
                                NodeKind::Drag { from_x, from_y, to_x, to_y, .. } => {
                                    if self.drag_pick_end {
                                        (*to_x, *to_y) = (x, y);
                                    } else {
                                        (*from_x, *from_y) = (x, y);
                                    }
                                }
                                _ => {}
                            }
                        }
                        self.dirty = true;
                        self.status = format!("Position set to ({x}, {y}).");
                    }
                    self.mode = Mode::Idle;
                }
                WorkerMsg::Snipped(path) => {
                    if let Mode::Snipping(id) = self.mode {
                        self.forget_seen(id);
                        if let Some(n) = self.doc.node_mut(id) {
                            if let NodeKind::FindImage { template, .. } = &mut n.kind {
                                *template = Some(path.clone());
                            }
                        }
                        self.dirty = true;
                        self.status = format!("Saved snip to {}. Use “Test now” to check it.", path.display());
                    }
                    self.mode = Mode::Idle;
                }
                WorkerMsg::StepResult { step, text, ok } => {
                    let r = self.results.entry(step).or_insert(CardResult { text: String::new(), ok, count: 0 });
                    r.text = text;
                    r.ok = ok;
                    r.count += 1;
                }
                WorkerMsg::Vars(v) => self.live_vars = v,
                WorkerMsg::Refresh => {}
                WorkerMsg::ShapeShot { step, shot } => {
                    self.mode = Mode::Idle;
                    self.restore_after_test = false;
                    show_again(window, cx);
                    self.open_shape_editor(step, shot);
                }
                WorkerMsg::Seen { step, path, caption, ok } => {
                    if let Some(old) = self.seen.insert(step, SeenShot { path, caption, ok }) {
                        let _ = std::fs::remove_file(old.path);
                    }
                }
                WorkerMsg::TestResult(s) => {
                    self.mode = Mode::Idle;
                    self.status = s;
                    if std::mem::take(&mut self.restore_after_test) {
                        show_again(window, cx);
                    }
                }
                WorkerMsg::AreaPicked { purpose, shot, sel } => {
                    self.mode = Mode::Idle;
                    self.pending_area = None;
                    self.capture_started = None;
                    show_again(window, cx);
                    match sel {
                        Some(sel) => self.apply_area(purpose, shot, sel),
                        None => {
                            let _ = std::fs::remove_file(&shot.path);
                            self.status = "Selection cancelled.".into();
                        }
                    }
                }
                WorkerMsg::Cropped { purpose, path } => {
                    self.mode = Mode::Idle;
                    match purpose {
                        AreaPurpose::ReadText(id) => {
                            // A new area: what it saw before is out of date.
                            self.forget_seen(id);
                            self.update_node(id, |k| {
                                if let NodeKind::ReadText { preview, .. } = k {
                                    *preview = Some(path.clone());
                                }
                            });
                        }
                        AreaPurpose::Template(id) => {
                            self.forget_seen(id);
                            self.update_node(id, |k| {
                                if let NodeKind::FindImage { template, .. } = k {
                                    *template = Some(path.clone());
                                }
                            });
                            self.status = format!("Saved {}. Use “Test now” to check it.", path.display());
                        }
                        AreaPurpose::SearchArea(_) | AreaPurpose::ClickArea(_) => {}
                    }
                }
                WorkerMsg::Windows(list) => {
                    self.mode = Mode::Idle;
                    self.status = format!("{} windows open – click one in the panel to use it.", list.len());
                    self.windows = list;
                }
                WorkerMsg::Failed(e) => {
                    self.mode = Mode::Idle;
                    self.active_node = None;
                    self.status = e;
                    let was_selecting = self.pending_area.take().is_some();
                    if was_selecting || std::mem::take(&mut self.restore_after_test) {
                        show_again(window, cx);
                    }
                }
            }
        }
        // Playing the run log back.
        if self.replay_playing {
            let due = self.replay_next.map_or(true, |t| Instant::now() >= t);
            if !self.idle() {
                self.replay_playing = false;
                changed = true;
            } else if due {
                let next = self.log_sel.map_or(0, |i| i + 1);
                if next < self.log.len() {
                    self.select_log(next, true);
                } else {
                    self.replay_playing = false;
                    self.status = "Playback finished.".into();
                }
                changed = true;
            }
        }
        // Keep a copy of unsaved work every few seconds, in case MacroFlow crashes.
        if self.dirty && self.last_autosave.elapsed() >= Duration::from_secs(8) {
            self.last_autosave = Instant::now();
            let saved_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let _ = write_autosave(&Autosave { path: self.path.clone(), saved_ms, doc: self.doc.clone() });
        }
        if changed {
            cx.notify();
        }
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    fn idle(&self) -> bool {
        self.mode == Mode::Idle
    }

    /// Open the quick-add box. With `link`, the new step is connected to that output
    /// and placed at `at` (canvas) / shown at `screen` (window).
    fn open_quick(&mut self, link: Option<(u64, usize)>, at: Option<(f32, f32)>, screen: Option<(f32, f32)>) {
        if !self.idle() {
            return;
        }
        self.commit_edit();
        self.menu = None;
        self.show_start = false;
        self.quick = Some(QuickAdd { query: String::new(), sel: 0, link, at, screen });
    }

    /// The steps matching what's typed in the quick-add box (indexes into the palette).
    fn quick_matches(&self) -> Vec<usize> {
        let Some(q) = &self.quick else { return Vec::new() };
        let words: Vec<String> = q.query.to_lowercase().split_whitespace().map(|w| w.to_string()).collect();
        let mut hits: Vec<(usize, usize)> = NodeKind::palette()
            .iter()
            .enumerate()
            .filter_map(|(i, k)| {
                let title = k.title().to_lowercase();
                let text = format!("{title} {}", step_blurb(k).to_lowercase());
                if !words.iter().all(|w| text.contains(w.as_str())) {
                    return None;
                }
                // Titles that start with what you typed come first.
                let rank = match words.first() {
                    Some(w) if title.starts_with(w.as_str()) => 0,
                    Some(w) if title.contains(w.as_str()) => 1,
                    Some(_) => 2,
                    None => 0,
                };
                Some((rank, i))
            })
            .collect();
        hits.sort();
        hits.into_iter().map(|(_, i)| i).collect()
    }

    /// Add the step chosen in the quick-add box.
    fn quick_pick(&mut self, palette_index: usize) {
        let Some(q) = self.quick.take() else { return };
        let Some(kind) = NodeKind::palette().into_iter().nth(palette_index) else { return };
        match q.at {
            Some((x, y)) => {
                // Put the new step's input dot where the line was let go.
                self.add_step_at(kind, x + 24.0, y + 16.0 - NODE_H / 2.0);
                if let (Some((from, port)), Some(new_id)) = (q.link, self.selected) {
                    self.doc.connect(from, port, new_id);
                }
            }
            None => {
                self.add_step(kind);
                self.side_tab = SideTab::Edit;
                self.sidebar_open = true;
            }
        }
    }

    /// Keys while the quick-add box is open.
    fn quick_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let m = ks.modifiers;
        let cmd = if IS_MACOS { m.platform } else { m.control && !m.alt };
        let matches = self.quick_matches();
        let count = matches.len();
        let Some(q) = self.quick.as_mut() else { return };
        match key {
            "escape" => self.quick = None,
            "k" if cmd => self.quick = None,
            "enter" | "return" => {
                let pick = matches.get(q.sel.min(count.saturating_sub(1))).copied();
                match pick {
                    Some(i) => self.quick_pick(i),
                    None => self.quick = None,
                }
            }
            "down" | "tab" => q.sel = if count == 0 { 0 } else { (q.sel + 1) % count },
            "up" => q.sel = if count == 0 { 0 } else { (q.sel + count - 1) % count },
            "backspace" => {
                if cmd {
                    q.query.clear();
                } else {
                    q.query.pop();
                }
                q.sel = 0;
            }
            _ => {
                let shortcut = if IS_MACOS { m.platform || m.control } else { m.control && !m.alt };
                if !shortcut {
                    match &ks.key_char {
                        Some(ch) => q.query.push_str(ch),
                        None if key == "space" => q.query.push(' '),
                        None => {}
                    }
                    q.sel = 0;
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Stop playback and forget the selected log line.
    fn reset_replay(&mut self) {
        self.log_sel = None;
        self.replay_playing = false;
        self.replay_next = None;
        self.replay_node = None;
    }

    /// Look at one line of the run log: outline its step, show its picture, and
    /// (with `flash`) show the action on the real screen too.
    fn select_log(&mut self, i: usize, flash: bool) {
        let Some(entry) = self.log.get(i).cloned() else { return };
        self.log_sel = Some(i);
        self.replay_node = entry.step.filter(|id| self.doc.node(*id).is_some());
        if let Some(id) = self.replay_node {
            self.reveal_step(id);
        }
        if flash && self.log_on_screen && self.idle() {
            if let Some(a) = &entry.action {
                if let Some((x, y)) = a.at {
                    marker::flash(x, y, a.mark == vision::Mark::Click);
                }
            }
        }
        // When playing, wait about as long as the macro did before the next line.
        if self.replay_playing {
            let gap = self
                .log
                .get(i + 1)
                .map_or(600, |n| n.ms.saturating_sub(entry.ms))
                .clamp(150, 1500);
            let gap = if entry.action.is_some() { gap.max(700) } else { gap };
            self.replay_next = Some(Instant::now() + Duration::from_millis(gap / self.replay_speed.max(1) as u64));
        }
    }

    fn replay_toggle(&mut self) {
        if self.replay_playing {
            self.replay_playing = false;
            return;
        }
        if self.log.is_empty() || !self.idle() {
            return;
        }
        self.replay_playing = true;
        let at_end = self.log_sel.map_or(true, |i| i + 1 >= self.log.len());
        if at_end {
            self.select_log(0, true);
        } else {
            self.replay_next = Some(Instant::now());
        }
        self.status = "Playing the run back… (Space pauses, ← → step)".into();
    }

    fn replay_step(&mut self, forward: bool) {
        if self.log.is_empty() {
            return;
        }
        self.replay_playing = false;
        let i = match (self.log_sel, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(self.log.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.select_log(i, true);
    }

    /// Show a step on the canvas (opening its group if needed) without changing the
    /// sidebar, panning only if it's off screen.
    fn reveal_step(&mut self, id: u64) {
        let Some(path) = self.doc.root.path_to(id) else { return };
        if self.view != path {
            self.view = path;
        }
        let Some(n) = self.doc.node(id) else { return };
        let (cw, ch) = self.canvas_size.get();
        let (sx, sy) = (n.x * self.zoom + self.pan.0, n.y * self.zoom + self.pan.1);
        let (w, h) = (NODE_W * self.zoom, NODE_H * self.zoom);
        let visible = sx >= 20.0 && sy >= 20.0 && sx + w <= cw - 20.0 && sy + h <= ch - 20.0;
        if !visible && cw > 0.0 {
            self.pan = (cw * 0.35 - n.x * self.zoom - w / 2.0, ch * 0.4 - n.y * self.zoom - h / 2.0);
        }
    }

    /// The picture for the selected log line: its own, or the last action before it
    /// in the same step.
    fn replay_picture(&self) -> Option<(PathBuf, usize)> {
        let i = self.log_sel?;
        for j in (0..=i).rev() {
            let e = self.log.get(j)?;
            if let Some(p) = e.action.as_ref().and_then(|a| a.shot.clone()) {
                return Some((p, j));
            }
            if j < i && e.level == LogLevel::Step {
                break;
            }
        }
        None
    }

    /// Drop the picture of what a step last saw (e.g. it has a new area now).
    fn forget_seen(&mut self, id: u64) {
        if let Some(old) = self.seen.remove(&id) {
            let _ = std::fs::remove_file(old.path);
        }
    }

    /// The picture box at the top of a Read text / Find image step: what it saw the
    /// last time it ran, or the original picture (`original`, with its label).
    fn picture_box(
        &self,
        id: u64,
        height: f32,
        original: AnyElement,
        original_label: &'static str,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let seen = self.seen.get(&id);
        let showing_seen = seen.is_some() && !self.show_original;
        if seen.is_some() {
            let tab = |key: &'static str, label: &'static str, on: bool, to_original: bool| {
                chip(SharedString::from(format!("{key}-{id}")), label, on).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.show_original = to_original;
                        cx.notify();
                    },
                ))
            };
            out.push(
                div()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .child(tab("seen-last", "Last run", showing_seen, false))
                    .child(tab("seen-orig", original_label, !showing_seen, true))
                    .into_any_element(),
            );
        }
        let (content, border): (AnyElement, u32) = match seen {
            Some(sh) if showing_seen => (
                img(sh.path.clone())
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                if sh.ok { GREEN } else { VAR_WARN },
            ),
            _ => (original, BORDER),
        };
        out.push(
            div()
                .h(px(height))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(rgb(UI_INPUT))
                .border_1()
                .border_color(rgb(border))
                .p_1()
                .child(content)
                .into_any_element(),
        );
        if let Some(sh) = seen.filter(|_| showing_seen) {
            out.push(
                div()
                    .text_xs()
                    .text_color(rgb(if sh.ok { TEXT } else { VAR_WARN }))
                    .child(sh.caption.clone())
                    .into_any_element(),
            );
        }
        out
    }

    fn begin_worker(&mut self, mode: Mode) -> bool {
        if !self.idle() {
            return false;
        }
        self.commit_edit();
        self.capturing_key = false;
        self.stop.store(false, Ordering::Relaxed);
        self.mode = mode;
        true
    }

    fn start_run(&mut self) {
        self.start_run_with(None, false);
    }

    /// Play the macro (from `from`, or Start). With `stepping`, it pauses before every
    /// step so you can go through it one step at a time.
    fn start_run_with(&mut self, from: Option<u64>, stepping: bool) {
        if !self.idle() {
            return;
        }
        self.commit_edit();
        let hotkey = std::mem::take(&mut self.hotkey_start);
        // Check first: steps that would certainly go wrong.
        let problems: Vec<Problem> = self.doc.problems().into_iter().filter(|p| p.serious).collect();
        if !problems.is_empty() {
            if hotkey {
                // MacroFlow may be hidden behind the app you're using.
                let n = problems.len();
                let _ = vision::show_notification(&vision::Toast {
                    title: "Macro didn't start".into(),
                    message: format!(
                        "{n} step{} would go wrong. Switch to MacroFlow to see {} or run anyway.",
                        if n == 1 { "" } else { "s" },
                        if n == 1 { "it" } else { "them" }
                    ),
                    style: "warning",
                    position: "top-center",
                    duration_ms: 6000,
                    wait: false,
                    sound: true,
                });
            }
            self.run_check = Some(RunCheck { from, stepping, problems });
            return;
        }
        self.run_now(from, stepping);
    }

    /// Play the macro without checking it first.
    fn run_now(&mut self, from: Option<u64>, stepping: bool) {
        self.run_check = None;
        if self.begin_worker(Mode::Running) {
            self.results.clear();
            self.live_vars.clear();
            self.show_vars = true;
            self.show_start = false;
            self.last_end = None;
            self.log.clear();
        self.reset_replay();
            self.paused.store(false, Ordering::Relaxed);
            self.step_mode.store(stepping, Ordering::Relaxed);
            self.status = match (from, stepping) {
                (_, true) => "Step by step: F10 / Step runs the next step, F9 / Resume runs the rest.".into(),
                (Some(id), false) => format!(
                    "Starting from “{}”…",
                    self.doc.node(id).map(|n| n.label()).unwrap_or_default()
                ),
                (None, false) => "Starting…   (F8 stops · F9 pauses · F10 steps)".into(),
            };
            engine::run_macro(
                self.doc.clone(),
                self.tx.clone(),
                self.stop.clone(),
                self.paused.clone(),
                self.step_mode.clone(),
                from,
            );
        }
    }

    /// Run just the next step (F10 / the Step button). When nothing is running,
    /// start the macro one step at a time.
    fn step_once(&mut self) {
        match self.mode {
            Mode::Running => {
                self.step_mode.store(true, Ordering::Relaxed);
                self.paused.store(false, Ordering::Relaxed);
            }
            Mode::Idle => self.start_run_with(None, true),
            _ => {}
        }
    }

    /// Pause or resume a running macro (F9 / the Pause button).
    fn toggle_pause(&mut self) {
        if self.mode != Mode::Running {
            return;
        }
        let now = !self.paused.load(Ordering::Relaxed);
        if !now {
            // Resume runs normally again (until the next breakpoint).
            self.step_mode.store(false, Ordering::Relaxed);
        }
        self.paused.store(now, Ordering::Relaxed);
        self.status = if now {
            "❚❚ Paused – press F9 (or Resume) to carry on, Esc or F8 to stop".into()
        } else {
            "▶ Resumed.".into()
        };
    }

    /// Converts positions on the screen right now into the positions the macro saves
    /// (scaled to its reference screen / followed window). Identity if the macro uses
    /// plain screen positions, or if the followed window can't be found.
    fn saved_mapping(&mut self) -> Mapping {
        match vision::current_mapping(self.doc.reference, self.doc.anchor.as_ref()) {
            Ok(Some(m)) => m.inverse(),
            Ok(None) => Mapping::identity(),
            Err(e) => {
                self.status = format!("{e} Positions are saved as if nothing had moved.");
                Mapping::identity()
            }
        }
    }

    /// A position on screen right now -> the position to save in the macro.
    fn to_anchor(&mut self, x: i32, y: i32) -> (i32, i32) {
        self.saved_mapping().point(x, y)
    }

    /// Converts saved positions to the screen right now, for testing a step;
    /// None (with a message) if the followed window isn't open.
    fn test_mapping(&mut self) -> Option<Mapping> {
        match vision::current_mapping(self.doc.reference, self.doc.anchor.as_ref()) {
            Ok(m) => Some(m.unwrap_or_else(Mapping::identity)),
            Err(e) => {
                self.status = e;
                None
            }
        }
    }

    /// Turn screen-size scaling on or off. Every saved position keeps pointing at
    /// the same place on screen it does right now.
    fn set_scaling(&mut self, on: bool) {
        // First make every position an exact position on the screen as it is now.
        match vision::current_mapping(self.doc.reference, self.doc.anchor.as_ref()) {
            Ok(Some(m)) => self.doc.map_positions(&m),
            Ok(None) => {}
            Err(e) => {
                self.status = format!("{e} Open it first, then change this.");
                return;
            }
        }
        if let Some(a) = self.doc.anchor.as_mut() {
            // Following a window: its current position becomes the reference point.
            if let Ok(r) = vision::anchor_rect(a) {
                a.x = r.x;
                a.y = r.y;
            }
        }
        self.doc.reference = if on {
            match vision::target_rect(self.doc.anchor.as_ref()) {
                Ok(r) => Some(r),
                Err(e) => {
                    self.status = e;
                    None
                }
            }
        } else {
            None
        };
        self.dirty = true;
        if on && self.doc.reference.is_some() {
            self.status = "Positions now scale with the screen size, so they land in the same place on another monitor or computer.".into();
        } else if !on {
            self.status = "Positions are exact screen positions again.".into();
        }
    }

    /// Play the macro once, starting at this step (from the right-click menu).
    fn start_run_from(&mut self, id: u64) {
        self.start_run_with(Some(id), false);
    }

    fn toggle_breakpoint(&mut self, id: u64) {
        if let Some(n) = self.doc.node_mut(id) {
            if matches!(n.kind, NodeKind::GroupIn | NodeKind::GroupOut) {
                return;
            }
            n.breakpoint = !n.breakpoint;
            let on = n.breakpoint;
            let label = n.label();
            self.dirty = true;
            self.status = if on {
                format!("Breakpoint on “{label}”: the macro will pause just before it runs.")
            } else {
                format!("Breakpoint removed from “{label}”.")
            };
        }
    }

    /// Disable the selected steps (or enable them, if they're all disabled already).
    /// A disabled step or group is skipped when the macro runs.
    fn toggle_disabled(&mut self) {
        self.commit_edit();
        let ids: Vec<u64> = self.selection().into_iter().filter(|id| self.doc.can_disable(*id)).collect();
        if ids.is_empty() {
            self.status = "Start and a group's start / end can't be disabled.".into();
            return;
        }
        let all_off = ids.iter().all(|id| self.doc.node(*id).map_or(false, |n| n.disabled));
        let off = !all_off;
        for id in &ids {
            if let Some(n) = self.doc.node_mut(*id) {
                n.disabled = off;
            }
        }
        self.dirty = true;
        let what = match (ids.len(), self.doc.node(ids[0])) {
            (1, Some(n)) => format!("“{}”", n.label()),
            (k, _) => format!("{k} steps"),
        };
        self.status = if off {
            format!("Disabled {what}: runs skip it and carry on with what it's connected to. {SHORTCUT_KEY}E turns it back on.")
        } else {
            format!("Enabled {what} again.")
        };
    }

    fn start_drag_pick(&mut self, id: u64, end: bool) {
        self.drag_pick_end = end;
        self.start_pick(id);
    }

    fn start_record(&mut self) {
        if self.begin_worker(Mode::Recording) {
            self.status = "● Getting ready to record…".into();
            engine::record_macro(self.tx.clone(), self.stop.clone(), self.record_moves);
        }
    }

    fn start_record_step(&mut self) {
        if self.begin_worker(Mode::RecordingStep) {
            self.status = "◎ Getting ready…".into();
            engine::record_step(self.tx.clone(), self.stop.clone());
        }
    }

    fn start_pick(&mut self, id: u64) {
        if self.begin_worker(Mode::Picking(id)) {
            engine::pick_position(self.tx.clone(), self.stop.clone());
        }
    }

    fn start_snip(&mut self, id: u64) {
        if self.begin_worker(Mode::Snipping(id)) {
            engine::snip(self.tx.clone(), self.stop.clone(), self.snip_size);
        }
    }

    fn start_test(&mut self, id: u64, window: &mut Window) {
        let Some(NodeKind::FindImage { template, threshold, search_area, .. }) =
            self.doc.node(id).map(|n| n.kind.clone())
        else {
            return;
        };
        let Some(path) = template else {
            self.status = "Snip or choose an image first.".into();
            return;
        };
        let Some(mapping) = self.test_mapping() else { return };
        let search_area = search_area.map(|r| mapping.rect(r));
        let scale = mapping.scale();
        if self.begin_worker(Mode::Testing) {
            self.status = if search_area.is_some() {
                "Searching the search area…".into()
            } else {
                "Searching the screen…".into()
            };
            // Get out of the way so MacroFlow (and its preview of the image) isn't searched.
            self.restore_after_test = true;
            window.minimize_window();
            engine::test_find(self.tx.clone(), id, path, threshold, search_area, scale, 600);
        }
    }

    /// Take a screenshot and let the user drag a box on it.
    /// Normally MacroFlow hides itself so the screenshot shows what's behind it;
    /// with `countdown` the user gets 3 seconds to switch windows instead.
    fn start_area(&mut self, purpose: AreaPurpose, countdown: bool, window: &mut Window, _cx: &mut Context<Self>) {
        if !self.begin_worker(Mode::Capturing) {
            return;
        }
        self.pending_area = Some(purpose);
        self.status = if countdown {
            "Switch to the window you want…".into()
        } else {
            "Capturing the screen…".into()
        };
        // Get MacroFlow out of the way so you can select anything on screen.
        self.capture_started = Some(Instant::now());
        window.minimize_window();
        engine::capture_for_area(self.tx.clone(), self.stop.clone(), countdown, purpose);
    }

    /// Use a box the user dragged on screen. `sel` is (x0, y0, x1, y1) in the
    /// screenshot's display coordinates (points on macOS, pixels on Windows).
    fn apply_area(&mut self, purpose: AreaPurpose, shot: vision::Shot, sel: (f32, f32, f32, f32)) {
        let x0 = sel.0.min(sel.2).floor().max(0.0) as u32;
        let y0 = sel.1.min(sel.3).floor().max(0.0) as u32;
        let x1 = (sel.0.max(sel.2).ceil().max(0.0) as u32).min(shot.w);
        let y1 = (sel.1.max(sel.3).ceil().max(0.0) as u32).min(shot.h);
        let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        if w < 4 || h < 4 {
            let _ = std::fs::remove_file(&shot.path);
            self.status = "That box was too small – try again and drag a bigger one.".into();
            return;
        }
        // The box on screen -> the area the macro saves (scaled / relative to its window).
        let saved = self.saved_mapping().rect(ScreenRect {
            x: shot.ox + x0 as i32,
            y: shot.oy + y0 as i32,
            w: w as i32,
            h: h as i32,
        });
        let (gx, gy) = (saved.x, saved.y);
        match purpose {
            AreaPurpose::ReadText(id) => {
                self.update_node(id, |k| {
                    if let NodeKind::ReadText { x, y, w: nw, h: nh, .. } = k {
                        *x = gx;
                        *y = gy;
                        *nw = saved.w;
                        *nh = saved.h;
                    }
                });
                self.status = format!("Area set to {w}×{h} at ({gx}, {gy}). Press “Read now” to test it.");
            }
            AreaPurpose::Template(_) => self.status = "Saving the picture…".into(),
            AreaPurpose::SearchArea(id) => {
                let _ = std::fs::remove_file(&shot.path);
                let rect = saved;
                self.update_node(id, |k| {
                    if let NodeKind::FindImage { search_area, .. } = k {
                        *search_area = Some(rect);
                    }
                });
                self.status = format!(
                    "Find image will only look inside {w}×{h} at ({gx}, {gy}). Press “Test now” to check it."
                );
                return;
            }
            AreaPurpose::ClickArea(id) => {
                let _ = std::fs::remove_file(&shot.path);
                let rect = saved;
                let mut taps = false;
                self.update_node(id, |k| {
                    if let Some((area, _)) = k.area_parts_mut() {
                        *area = Some(rect);
                    }
                    match k {
                        NodeKind::Click { target, .. } | NodeKind::MoveMouse { target, .. } => *target = Target::Area,
                        NodeKind::Drag { tap_in_area, .. } => {
                            *tap_in_area = true;
                            taps = true;
                        }
                        _ => {}
                    }
                });
                self.status = if taps {
                    format!("The taps will be spread round {w}×{h} at ({gx}, {gy}). Give it a shape below.")
                } else {
                    format!("It will click a random spot inside {w}×{h} at ({gx}, {gy}) each time.")
                };
                return;
            }
        }
        self.mode = Mode::Working;
        engine::crop_area(self.tx.clone(), shot, (x0, y0, w, h), purpose);
    }

    // -----------------------------------------------------------------------
    // Unsaved changes
    // -----------------------------------------------------------------------

    /// Called when the window is about to close or the app is about to quit.
    /// Returns true if it's fine to go ahead now.
    fn confirm_close(&mut self, after: AfterSave, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.commit_edit();
        if !self.dirty || self.close_confirmed {
            clear_autosave();
            return true;
        }
        if self.close_prompt_open {
            return false;
        }
        self.close_prompt_open = true;
        let question = format!("Do you want to save the changes to “{}”?", self.doc.name);
        let answer = window.prompt(
            PromptLevel::Warning,
            &question,
            Some("Your changes will be lost if you don't save them."),
            &["Save", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = answer.await.ok();
            let _ = this.update_in(cx, |app, window, cx| {
                app.close_prompt_open = false;
                match choice {
                    Some(0) => app.save_then(after, window, cx),
                    Some(1) => app.finish_close(after, window, cx),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        false
    }

    /// Save (asking where if needed), then close or quit if it worked.
    fn save_then(&mut self, after: AfterSave, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.path.clone() {
            self.write_to(p);
            if !self.dirty {
                self.finish_close(after, window, cx);
            }
            return;
        }
        let dir = app_dir();
        let _ = std::fs::create_dir_all(&dir);
        let suggested = format!("{}.json", safe_file_name(&self.doc.name));
        let rx = cx.prompt_for_new_path(&dir, Some(suggested.as_str()));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(p))) = rx.await {
                let _ = this.update_in(cx, |app, window, cx| {
                    app.write_to(p);
                    if !app.dirty {
                        app.finish_close(after, window, cx);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn finish_close(&mut self, after: AfterSave, window: &mut Window, cx: &mut Context<Self>) {
        // Closing on purpose (saved, or "Don't Save"): nothing to restore next time.
        clear_autosave();
        self.close_confirmed = true;
        self.stop.store(true, Ordering::Relaxed);
        match after {
            AfterSave::CloseWindow => window.remove_window(),
            AfterSave::Quit => cx.quit(),
        }
    }

    fn on_quit(&mut self, _: &crate::Quit, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_close(AfterSave::Quit, window, cx) {
            cx.quit();
        }
    }

    // -----------------------------------------------------------------------
    // Shape editor
    // -----------------------------------------------------------------------

    /// Take a screenshot (with MacroFlow out of the way) and open the shape editor
    /// for a step's random area on top of it.
    fn start_shape_editor(&mut self, id: u64, window: &mut Window) {
        self.commit_edit();
        let Some(saved) = self.doc.node(id).and_then(|n| n.kind.area_parts().map(|(a, _)| a)) else {
            return;
        };
        // Screenshot the display the area is on (or the one under the mouse).
        let at = match saved {
            Some(r) => {
                let Some(mapping) = self.test_mapping() else { return };
                let r = mapping.rect(r);
                Some((r.x + r.w / 2, r.y + r.h / 2))
            }
            None => None,
        };
        if self.begin_worker(Mode::Working) {
            self.status = "Capturing the screen…".into();
            self.restore_after_test = true;
            window.minimize_window();
            engine::capture_for_editor(self.tx.clone(), self.stop.clone(), id, at);
        }
    }

    /// The screenshot arrived: open the editor with the step's area on it.
    fn open_shape_editor(&mut self, step: u64, shot: vision::Shot) {
        let Some((area, shape, taps)) = self.doc.node(step).and_then(|n| {
            let taps = match &n.kind {
                NodeKind::Drag { taps, .. } => *taps,
                _ => 0,
            };
            n.kind.area_parts().map(|(a, sh)| (a, sh.clone(), taps))
        }) else {
            let _ = std::fs::remove_file(&shot.path);
            return;
        };
        let mapping = self.test_mapping().unwrap_or_else(Mapping::identity);
        let mut shape = shape;
        let (cx, cy, w, h) = match area {
            Some(r) => {
                let r = mapping.rect(r);
                if shape.hollow_px > 0 {
                    shape.hollow_px = ((shape.hollow_px as f64 * mapping.scale()).round() as i32).max(1);
                }
                (r.x as f64 + r.w as f64 / 2.0, r.y as f64 + r.h as f64 / 2.0, r.w as f64, r.h as f64)
            }
            // No area yet: a box in the middle of the screen to start from.
            None => (
                shot.ox as f64 + shot.w as f64 / 2.0,
                shot.oy as f64 + shot.h as f64 / 2.0,
                shot.w as f64 * 0.3,
                shot.h as f64 * 0.3,
            ),
        };
        self.shape_edit = Some(ShapeEdit {
            step,
            shot,
            cx,
            cy,
            w,
            h,
            shape,
            taps,
            drag: EditDrag::None,
            drawing: false,
            k: 0.5,
            origin: Rc::new(Cell::new((0.0, 0.0))),
        });
        self.status = "Drag the shape into place, then Save (Enter). Esc cancels.".into();
    }

    fn close_shape_editor(&mut self) {
        if let Some(ed) = self.shape_edit.take() {
            let _ = std::fs::remove_file(&ed.shot.path);
        }
    }

    /// Save what was drawn back into the step.
    fn save_shape_edit(&mut self) {
        let Some(ed) = self.shape_edit.clone() else { return };
        if ed.is_poly() && ed.shape.points.len() < 3 {
            self.status = "A freeform outline needs at least 3 corners.".into();
            return;
        }
        let back = self.saved_mapping();
        let saved = back.rect(ed.rect());
        let mut shape = ed.shape.clone();
        shape.rotation = (shape.rotation + 180.0).rem_euclid(360.0) - 180.0;
        if shape.hollow_px > 0 {
            shape.hollow_px = ((shape.hollow_px as f64 * back.scale()).round() as i32).max(1);
        }
        let desc = if shape.is_plain() { "rectangle".to_string() } else { shape.describe() };
        self.update_node(ed.step, |k| {
            if let Some((area, sh)) = k.area_parts_mut() {
                *area = Some(saved);
                *sh = shape;
            }
            match k {
                NodeKind::Click { target, .. } | NodeKind::MoveMouse { target, .. } => *target = Target::Area,
                NodeKind::Drag { tap_in_area, .. } => *tap_in_area = true,
                _ => {}
            }
        });
        self.close_shape_editor();
        self.status = format!("Area saved: {desc}, {} × {}.", saved.w, saved.h);
    }

    fn edit_mouse_down(&mut self, pos: Point<Pixels>, right: bool) {
        let Some(ed) = self.shape_edit.as_mut() else { return };
        let p = ed.screen_at(f32::from(pos.x), f32::from(pos.y));
        let tol = ed.tolerance();
        if right {
            // Right-click a freeform corner to remove it.
            if ed.is_poly() && !ed.drawing && ed.shape.points.len() > 3 {
                if let Some(i) = ed.poly().iter().position(|q| dist(*q, p) <= tol) {
                    ed.shape.points.remove(i);
                    ed.refit();
                }
            }
            return;
        }
        if ed.drawing {
            let pts = ed.poly();
            if pts.len() >= 3 && dist(pts[0], p) <= tol * 1.4 {
                // Back at the first corner: the outline is done.
                ed.drawing = false;
            } else if ed.shape.points.is_empty() {
                ed.cx = p.0;
                ed.cy = p.1;
                ed.w = 1.0;
                ed.h = 1.0;
                ed.shape.points.push([0.5, 0.5]);
            } else {
                let n = ed.shape.points.len();
                ed.set_point(n, p.0, p.1);
            }
            return;
        }
        if dist(ed.rot_handle(), p) <= tol * 1.2 {
            ed.drag = EditDrag::Rotate;
            return;
        }
        if ed.is_poly() {
            if let Some(i) = ed.poly().iter().position(|q| dist(*q, p) <= tol) {
                ed.drag = EditDrag::Point(i);
                return;
            }
        }
        if let Some(i) = (0..4).find(|i| dist(ed.corner(*i), p) <= tol) {
            ed.drag = EditDrag::Corner(i);
            return;
        }
        if let Some(i) = (0..4).find(|i| dist(ed.edge(*i), p) <= tol) {
            ed.drag = EditDrag::Edge(i);
            return;
        }
        if ed.is_poly() && ed.shape.points.len() >= 3 {
            let pts = ed.poly();
            if inside_poly(&pts, p.0, p.1) {
                ed.drag = EditDrag::Move { dx: p.0 - ed.cx, dy: p.1 - ed.cy };
            } else {
                // Outside the outline: a new corner on the nearest side.
                let n = pts.len();
                let side = (0..n)
                    .min_by(|a, b| {
                        seg_dist(p, pts[*a], pts[(*a + 1) % n])
                            .partial_cmp(&seg_dist(p, pts[*b], pts[(*b + 1) % n]))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(n - 1);
                let (lx, ly) = ed.to_local(p.0, p.1);
                let f = [(lx / ed.w.max(1.0) + 0.5) as f32, (ly / ed.h.max(1.0) + 0.5) as f32];
                ed.shape.points.insert(side + 1, f);
                ed.refit();
                ed.drag = EditDrag::Point(side + 1);
            }
            return;
        }
        let (lx, ly) = ed.to_local(p.0, p.1);
        if lx.abs() <= ed.w / 2.0 && ly.abs() <= ed.h / 2.0 {
            ed.drag = EditDrag::Move { dx: p.0 - ed.cx, dy: p.1 - ed.cy };
        } else {
            // Outside: draw a new box from here.
            ed.drag = EditDrag::NewBox { x0: p.0, y0: p.1 };
            ed.shape.rotation = 0.0;
            ed.cx = p.0;
            ed.cy = p.1;
            ed.w = 1.0;
            ed.h = 1.0;
        }
    }

    fn edit_mouse_move(&mut self, pos: Point<Pixels>, shift: bool) {
        let Some(ed) = self.shape_edit.as_mut() else { return };
        let p = ed.screen_at(f32::from(pos.x), f32::from(pos.y));
        match ed.drag {
            EditDrag::None => {}
            EditDrag::Move { dx, dy } => {
                ed.cx = p.0 - dx;
                ed.cy = p.1 - dy;
            }
            EditDrag::Corner(i) => {
                let (sx, sy) = CORNER_SIGNS[i];
                // The opposite corner stays put.
                let o = (-sx * ed.w / 2.0, -sy * ed.h / 2.0);
                let l = ed.to_local(p.0, p.1);
                let (nw, nh) = ((l.0 - o.0).abs().max(4.0), (l.1 - o.1).abs().max(4.0));
                let mid = ((l.0 + o.0) / 2.0, (l.1 + o.1) / 2.0);
                let (ncx, ncy) = ed.to_screen(mid.0, mid.1);
                (ed.cx, ed.cy, ed.w, ed.h) = (ncx, ncy, nw, nh);
            }
            EditDrag::Edge(i) => {
                let (sx, sy) = EDGE_SIGNS[i];
                let l = ed.to_local(p.0, p.1);
                if sx == 0.0 {
                    let o = -sy * ed.h / 2.0;
                    let nh = (l.1 - o).abs().max(4.0);
                    let (ncx, ncy) = ed.to_screen(0.0, (l.1 + o) / 2.0);
                    (ed.cx, ed.cy, ed.h) = (ncx, ncy, nh);
                } else {
                    let o = -sx * ed.w / 2.0;
                    let nw = (l.0 - o).abs().max(4.0);
                    let (ncx, ncy) = ed.to_screen((l.0 + o) / 2.0, 0.0);
                    (ed.cx, ed.cy, ed.w) = (ncx, ncy, nw);
                }
            }
            EditDrag::Rotate => {
                // Straight up = 0°, clockwise positive. Shift snaps to 15°.
                let mut deg = (p.0 - ed.cx).atan2(-(p.1 - ed.cy)).to_degrees();
                if shift {
                    deg = (deg / 15.0).round() * 15.0;
                }
                ed.shape.rotation = deg as f32;
            }
            EditDrag::Point(i) => ed.set_point(i, p.0, p.1),
            EditDrag::NewBox { x0, y0 } => {
                ed.cx = (x0 + p.0) / 2.0;
                ed.cy = (y0 + p.1) / 2.0;
                ed.w = (p.0 - x0).abs().max(1.0);
                ed.h = (p.1 - y0).abs().max(1.0);
            }
        }
    }

    fn edit_mouse_up(&mut self) {
        if let Some(ed) = self.shape_edit.as_mut() {
            if let EditDrag::NewBox { .. } = ed.drag {
                // A click rather than a drag: keep a usable box.
                if ed.w < 6.0 || ed.h < 6.0 {
                    ed.w = ed.w.max(40.0);
                    ed.h = ed.h.max(40.0);
                }
            }
            ed.drag = EditDrag::None;
        }
    }

    /// Keys while the shape editor is open.
    fn edit_key(&mut self, key: &str, shift: bool) {
        let Some(ed) = self.shape_edit.as_mut() else { return };
        match key {
            "escape" => {
                if ed.drawing && ed.shape.points.len() >= 3 {
                    ed.drawing = false;
                } else {
                    self.close_shape_editor();
                    self.status = "Area not changed.".into();
                }
            }
            "enter" | "return" => {
                if ed.drawing {
                    if ed.shape.points.len() >= 3 {
                        ed.drawing = false;
                    }
                } else {
                    self.save_shape_edit();
                }
            }
            "backspace" | "delete" if ed.drawing => {
                ed.shape.points.pop();
                ed.refit();
            }
            "left" | "right" if !ed.drawing => {
                let step = if shift { 15.0 } else { 1.0 };
                ed.shape.rotation += if key == "left" { -step } else { step };
            }
            _ => {}
        }
    }

    fn render_shape_edit(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(ed) = &self.shape_edit else { return div().id("shape-edit") };
        let (iw, ih) = (ed.shot.w as f32 * ed.k, ed.shot.h as f32 * ed.k);
        let snap = ed.clone();
        let origin = ed.origin.clone();
        let overlay = canvas(
            move |bounds, _, _| origin.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y))),
            move |bounds, _, window, _| paint_shape_edit(&snap, bounds, window),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let kind = ed.shape.kind;
        let poly = ed.is_poly();
        let drawing = ed.drawing;
        let hollow = ed.shape.hollow_px;
        let set_shape = |k: ShapeKind| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, _, cx| {
                if let Some(ed) = this.shape_edit.as_mut() {
                    ed.shape.kind = k;
                    ed.drawing = false;
                    if k == ShapeKind::Polygon && ed.shape.points.len() < 3 {
                        ed.shape.points = default_polygon_points();
                    }
                }
                cx.notify();
            })
        };
        let mut shapes = div().flex().flex_row().flex_wrap().gap_1();
        for k in ShapeKind::ALL {
            shapes = shapes.child(chip(("edit-shape", k as usize), k.label(), kind == k).on_click(set_shape(k)));
        }
        let bump_hollow = |id: &'static str, label: &'static str, d: i32| {
            small_btn(id, label).on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if let Some(ed) = this.shape_edit.as_mut() {
                    let step = if ev.modifiers().shift { d * 5 } else { d };
                    ed.shape.hollow_px = (ed.shape.hollow_px + step).clamp(0, 2000);
                }
                cx.notify();
            }))
        };
        let tools = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(shapes)
            .child(divider())
            .child(div().text_xs().text_color(rgb(MUTED)).child("Hollow edge"))
            .child(bump_hollow("edit-hollow-minus", "−", -5))
            .child(
                div()
                    .min_w(px(54.))
                    .text_center()
                    .text_xs()
                    .child(if hollow > 0 { format!("{hollow} px") } else { "off".to_string() }),
            )
            .child(bump_hollow("edit-hollow-plus", "+", 5))
            .child(divider())
            .child(div().text_xs().text_color(rgb(MUTED)).child(format!("Turn {:.0}°", ed.shape.rotation)))
            .child(small_btn("edit-turn-reset", "Straighten").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some(ed) = this.shape_edit.as_mut() {
                    ed.shape.rotation = 0.0;
                }
                cx.notify();
            })))
            .when(poly, |d| {
                d.child(divider()).child(
                    small_btn("edit-draw", if drawing { "Finish outline" } else { "Draw a new outline" }).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(ed) = this.shape_edit.as_mut() {
                                if ed.drawing {
                                    if ed.shape.points.len() >= 3 {
                                        ed.drawing = false;
                                    }
                                } else {
                                    ed.drawing = true;
                                    ed.shape.points.clear();
                                    ed.shape.rotation = 0.0;
                                }
                            }
                            cx.notify();
                        }),
                    ),
                )
            })
            .child(div().flex_1())
            .child(small_btn("edit-cancel", "Cancel  (Esc)").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.close_shape_editor();
                this.status = "Area not changed.".into();
                cx.notify();
            })))
            .child(
                small_btn("edit-save", "Save  (Enter)")
                    .bg(rgb(ACCENT))
                    .text_color(rgb(0xffffff))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.save_shape_edit();
                        cx.notify();
                    })),
            );
        let hint = if drawing {
            "Click each corner of the outline in turn. Click the first corner again (or press Enter) to finish · Backspace removes the last corner."
        } else if poly {
            "Drag a corner dot to move it · click outside the outline to add a corner · right-click a dot to remove it · drag inside to move it all · squares resize · the round handle turns it (Shift snaps to 15°)."
        } else {
            "Drag inside to move · the squares resize · the round handle on top turns it (Shift snaps to 15°, ← → nudge) · drag outside the box to draw a new one."
        };
        div()
            .id("shape-edit")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            // This layer covers the whole window and blocks the window's own mouse
            // tracking, so it follows drags itself (even outside the picture).
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                let dragging = this.shape_edit.as_ref().map_or(false, |ed| ed.drag != EditDrag::None);
                if !dragging {
                    return;
                }
                if ev.pressed_button == Some(MouseButton::Left) {
                    this.edit_mouse_move(ev.position, ev.modifiers.shift);
                } else {
                    this.edit_mouse_up();
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.edit_mouse_up();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.edit_mouse_up();
                    cx.notify();
                }),
            )
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .p_3()
            .bg(rgba(0x0b0d12f2))
            .text_sm()
            .text_color(rgb(TEXT))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().font_weight(FontWeight::BOLD).child(if ed.taps > 0 {
                        "Where to tap"
                    } else {
                        "Where to click"
                    }))
                    .child(tools),
            )
            .child(
                div()
                    .id("shape-edit-picture")
                    .relative()
                    .flex_none()
                    .w(px(iw))
                    .h(px(ih))
                    .border_1()
                    .border_color(rgb(UI_EDGE))
                    .cursor_crosshair()
                    .child(img(ed.shot.path.clone()).size_full())
                    .child(overlay)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.edit_mouse_down(ev.position, false);
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.edit_mouse_down(ev.position, true);
                            cx.notify();
                        }),
                    ),
            )
            .child(div().text_xs().text_color(rgb(MUTED)).child(hint))
    }

    /// Flash the green dot at a few random spots in a step's click area, on the real
    /// screen, so you can see where clicks will land.
    fn show_area_on_screen(&mut self, id: u64) {
        self.commit_edit();
        let Some((Some(r), shape, taps)) = self.doc.node(id).and_then(|n| {
            let taps = match &n.kind {
                NodeKind::Drag { taps, .. } => *taps,
                _ => 0,
            };
            n.kind.area_parts().map(|(a, sh)| (a, sh.clone(), taps))
        }) else {
            self.status = "Select an area first.".into();
            return;
        };
        let Some(mapping) = self.test_mapping() else { return };
        let r = mapping.rect(r);
        let mut shape = shape;
        if shape.hollow_px > 0 {
            shape.hollow_px = ((shape.hollow_px as f64 * mapping.scale()).round() as i32).max(1);
        }
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(7)
            | 1;
        let mut unit = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0
        };
        let points: Vec<(i32, i32)> = if taps > 0 {
            spread_taps(r, &shape, taps.min(80), &mut unit)
        } else {
            (0..24).map(|_| area_point(r, &shape, &mut unit)).collect()
        };
        let n = points.len();
        marker::tour(points);
        self.status = if taps > 0 {
            format!("Showing where {n} taps would go, in order, with the green dot…")
        } else {
            "Showing 24 random spots in the area with the green dot…".into()
        };
    }

    /// Scroll wherever the mouse is in 3 seconds, the way this Scroll step would.
    fn start_scroll_test(&mut self, id: u64) {
        self.commit_edit();
        let Some(NodeKind::Scroll { amount, horizontal, strength }) = self.doc.node(id).map(|n| n.kind.clone()) else {
            return;
        };
        if amount == 0 {
            self.status = "Set how many wheel clicks to scroll first.".into();
            return;
        }
        if self.begin_worker(Mode::Testing) {
            engine::test_scroll(self.tx.clone(), amount, horizontal, strength);
        }
    }

    fn start_read_test(&mut self, id: u64, window: &mut Window) {
        self.commit_edit();
        let Some(NodeKind::ReadText {
            x,
            y,
            w,
            h,
            compare,
            value,
            case_sensitive,
            numbers_only,
            ..
        }) = self.doc.node(id).map(|n| n.kind.clone())
        else {
            return;
        };
        if w < 2 || h < 2 {
            self.status = "Select an area on screen first.".into();
            return;
        }
        let Some(mapping) = self.test_mapping() else { return };
        let r = mapping.rect(ScreenRect { x, y, w, h });
        let (x, y, w, h) = (r.x, r.y, r.w, r.h);
        if self.begin_worker(Mode::Working) {
            self.status = "Reading the screen…".into();
            // Get out of the way so MacroFlow can't cover the area it's reading.
            self.restore_after_test = true;
            window.minimize_window();
            engine::test_read(
                self.tx.clone(),
                id,
                (x, y, w, h),
                compare,
                value,
                case_sensitive,
                numbers_only,
                600,
            );
        }
    }

    fn start_pixel_pick(&mut self, id: u64) {
        if self.begin_worker(Mode::Picking(id)) {
            engine::pick_pixel(self.tx.clone(), self.stop.clone());
        }
    }

    fn start_pixel_test(&mut self, id: u64, window: &mut Window) {
        self.commit_edit();
        let Some(NodeKind::PixelColor { x, y, color, tolerance, .. }) = self.doc.node(id).map(|n| n.kind.clone()) else {
            return;
        };
        let Some(mapping) = self.test_mapping() else { return };
        let (x, y) = mapping.point(x, y);
        if self.begin_worker(Mode::Working) {
            self.status = "Checking the colour…".into();
            self.restore_after_test = true;
            window.minimize_window();
            engine::test_pixel(self.tx.clone(), x, y, color, tolerance, 600);
        }
    }

    // -----------------------------------------------------------------------
    // Undo / redo
    // -----------------------------------------------------------------------

    /// Record the macro for undo if it changed since last time. Called on every
    /// redraw; skipped while dragging, so a whole drag is one undo step.
    fn checkpoint(&mut self) {
        if self.drag != Drag::None || self.mode == Mode::Running {
            return;
        }
        if self.doc != self.history_head {
            let previous = std::mem::replace(&mut self.history_head, self.doc.clone());
            self.undo.push(previous);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    /// Forget undo history (after opening or starting a macro).
    fn reset_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.history_head = self.doc.clone();
    }

    fn undo(&mut self) {
        if !self.idle() {
            return;
        }
        self.commit_edit();
        self.checkpoint();
        let Some(prev) = self.undo.pop() else {
            self.status = "Nothing to undo.".into();
            return;
        };
        let now = std::mem::replace(&mut self.doc, prev);
        self.redo.push(now);
        self.after_history_jump();
        self.status = "Undone.".into();
    }

    fn redo(&mut self) {
        if !self.idle() {
            return;
        }
        self.commit_edit();
        self.checkpoint();
        let Some(next) = self.redo.pop() else {
            self.status = "Nothing to redo.".into();
            return;
        };
        let now = std::mem::replace(&mut self.doc, next);
        self.undo.push(now);
        self.after_history_jump();
        self.status = "Redone.".into();
    }

    fn after_history_jump(&mut self) {
        self.history_head = self.doc.clone();
        self.dirty = true;
        self.editing = None;
        if self.doc.root.at(&self.view).is_none() {
            self.view.clear();
        }
        let exists = |id: &u64, d: &Macro| d.node(*id).is_some();
        if !self.selected.map_or(true, |id| exists(&id, &self.doc)) {
            self.selected = None;
        }
        let doc = &self.doc;
        self.extra.retain(|id| exists(id, doc));
    }

    // -----------------------------------------------------------------------
    // Copy / paste
    // -----------------------------------------------------------------------

    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let ids = self.selection();
        match self.doc.copy_steps(&ids) {
            Some(clip) => match serde_json::to_string_pretty(&clip) {
                Ok(json) => {
                    let n = clip.nodes.len();
                    cx.write_to_clipboard(ClipboardItem::new_string(json));
                    self.paste_count = 0;
                    self.status = format!("Copied {n} step{}.", if n == 1 { "" } else { "s" });
                    true
                }
                Err(e) => {
                    self.status = format!("Couldn't copy: {e}");
                    false
                }
            },
            None => {
                self.status = "Select the steps to copy first (Start and group start/end can't be copied).".into();
                false
            }
        }
    }

    fn cut_selection(&mut self, cx: &mut Context<Self>) {
        if self.idle() && self.copy_selection(cx) {
            self.delete_selected();
            self.status = "Cut – paste with Ctrl+V (⌘V on a Mac).".into();
        }
    }

    /// Paste copied steps. `at` (canvas position) puts their top-left corner there;
    /// otherwise each paste lands a little further down and right.
    fn paste(&mut self, at: Option<(f32, f32)>, cx: &mut Context<Self>) {
        if !self.idle() {
            return;
        }
        self.commit_edit();
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            self.status = "Nothing to paste – copy some steps first.".into();
            return;
        };
        let clip: StepClip = match serde_json::from_str(&text) {
            Ok(c) => c,
            Err(_) => {
                self.status = "The clipboard doesn't hold MacroFlow steps – copy some steps first.".into();
                return;
            }
        };
        if clip.nodes.is_empty() {
            return;
        }
        let min_x = clip.nodes.iter().map(|n| n.x).fold(f32::MAX, f32::min);
        let min_y = clip.nodes.iter().map(|n| n.y).fold(f32::MAX, f32::min);
        let (dx, dy) = match at {
            Some((x, y)) => (x - min_x, y - min_y),
            None => {
                self.paste_count += 1;
                let step = 40.0 * self.paste_count as f32;
                (step, step)
            }
        };
        let path = self.view.clone();
        let ids = self.doc.paste_steps(&path, &clip, dx.round(), dy.round());
        self.clear_selection();
        let mut it = ids.iter().copied();
        self.selected = it.next();
        self.extra = it.collect();
        self.side_tab = SideTab::Edit;
        self.dirty = true;
        self.status = format!("Pasted {} step{}.", ids.len(), if ids.len() == 1 { "" } else { "s" });
    }

    // -----------------------------------------------------------------------
    // Following a window
    // -----------------------------------------------------------------------

    /// Make the macro's positions follow `w` (or plain screen positions with None),
    /// keeping every saved position pointing at the same place it does right now.
    fn set_anchor(&mut self, w: Option<vision::WinInfo>) {
        // Make every position an exact position on the screen as it is now...
        if let Ok(Some(m)) = vision::current_mapping(self.doc.reference, self.doc.anchor.as_ref()) {
            self.doc.map_positions(&m);
        }
        self.doc.anchor = w.as_ref().map(|w| WindowAnchor {
            app: w.app.clone(),
            title: w.title.clone(),
            x: w.x,
            y: w.y,
        });
        // ...then, if positions scale, measure them against the new window / screen.
        if self.doc.reference.is_some() {
            self.doc.reference = match &w {
                Some(w) => Some(ScreenRect {
                    x: w.x,
                    y: w.y,
                    w: w.w as i32,
                    h: w.h as i32,
                }),
                None => vision::primary_screen().ok(),
            };
        }
        self.picking_anchor = false;
        self.windows.clear();
        self.dirty = true;
        self.status = match w {
            Some(w) => format!(
                "Positions now follow “{}”. Move the window and the macro still clicks the right places.",
                if w.title.is_empty() { w.app } else { format!("{} – {}", w.app, w.title) }
            ),
            None => "Positions are plain screen positions again.".into(),
        };
    }

    fn start_list_windows(&mut self) {
        if self.begin_worker(Mode::Working) {
            self.status = "Listing open windows…".into();
            engine::list_windows(self.tx.clone());
        }
    }

    fn stop_worker(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);
        self.step_mode.store(false, Ordering::Relaxed);
        self.status = "Stopping…".into();
    }

    /// The steps you're looking at: the top level, or the inside of the open group.
    fn view(&self) -> &Graph {
        self.doc.root.at(&self.view).unwrap_or(&self.doc.root)
    }

    fn view_mut(&mut self) -> &mut Graph {
        if self.doc.root.at(&self.view).is_none() {
            self.view.clear();
        }
        let path = self.view.clone();
        self.doc.root.at_mut(&path).expect("the top level always exists")
    }

    /// Every selected step (the clicked one plus any Shift-clicked ones).
    fn selection(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.selected.into_iter().collect();
        for id in &self.extra {
            if !v.contains(id) {
                v.push(*id);
            }
        }
        v
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.extra.clear();
    }

    fn enter_group(&mut self, id: u64) {
        if !matches!(self.doc.node(id).map(|n| &n.kind), Some(NodeKind::Group { .. })) {
            return;
        }
        self.commit_edit();
        let Some(mut path) = self.doc.root.path_to(id) else { return };
        path.push(id);
        self.view = path;
        self.clear_selection();
        self.pan = (0.0, 0.0);
        self.side_tab = SideTab::Steps;
        self.status = format!("Inside “{}”. Esc or the path at the top takes you back out.", self.group_name(id));
    }

    /// Go back out to level `depth` (0 = the top level of the macro).
    fn leave_group_to(&mut self, depth: usize) {
        if depth >= self.view.len() {
            return;
        }
        self.commit_edit();
        let came_from = self.view[depth];
        self.view.truncate(depth);
        self.clear_selection();
        // Select the group you just left, so it's easy to see where you were.
        self.selected = Some(came_from);
        self.pan = (0.0, 0.0);
        self.status = if depth == 0 {
            "Back at the top level of the macro.".into()
        } else {
            format!("Back in “{}”.", self.group_name(self.view[depth - 1]))
        };
    }

    fn group_name(&self, id: u64) -> String {
        match self.doc.node(id).map(|n| &n.kind) {
            Some(NodeKind::Group { name, .. }) if !name.trim().is_empty() => name.trim().to_string(),
            _ => "Group".into(),
        }
    }

    /// Group the selected steps, or (with one step selected) the straight line starting at it.
    fn group_selection(&mut self) {
        if !self.idle() {
            return;
        }
        self.commit_edit();
        // Start / group start / group end can't go in a group: just leave them out.
        let mut ids: Vec<u64> = self
            .selection()
            .into_iter()
            .filter(|id| self.doc.node(*id).map_or(false, |n| !n.kind.is_fixed()))
            .collect();
        if ids.is_empty() {
            self.status = "Select the steps to group first (click one, then Shift-click or Shift-drag for more).".into();
            return;
        }
        if ids.len() == 1 {
            ids = self.doc.straight_run(ids[0]);
            if ids.len() < 2 {
                self.status = "There's no straight line of steps after this one to group. Shift-click the steps you want instead.".into();
                return;
            }
        }
        match self.doc.group_steps(&ids) {
            Ok(group_id) => {
                self.clear_selection();
                self.selected = Some(group_id);
                self.side_tab = SideTab::Edit;
                self.sidebar_open = true;
                self.dirty = true;
                self.status = format!("Grouped {} steps. Double-click the group to open it.", ids.len());
            }
            Err(e) => self.status = e,
        }
    }

    fn ungroup_selected(&mut self) {
        if !self.idle() {
            return;
        }
        let Some(id) = self.selected else { return };
        if !matches!(self.doc.node(id).map(|n| &n.kind), Some(NodeKind::Group { .. })) {
            self.status = "Select a group to ungroup it.".into();
            return;
        }
        match self.doc.ungroup(id) {
            Ok(ids) => {
                self.clear_selection();
                self.selected = ids.first().copied();
                self.extra = ids.into_iter().skip(1).collect();
                self.dirty = true;
                self.status = "Ungrouped – the steps are back in place.".into();
            }
            Err(e) => self.status = e,
        }
    }

    /// Add a step. If a step is selected and its main output is free,
    /// the new step is placed after it and connected automatically.
    fn add_step(&mut self, kind: NodeKind) {
        self.commit_edit();
        let kind = match kind {
            NodeKind::Group { graph, .. } if graph.nodes.is_empty() => self.doc.new_group_kind(),
            k => k,
        };
        let link = self
            .selected
            .filter(|id| self.view().nodes.iter().any(|n| n.id == *id))
            .and_then(|id| self.doc.node(id))
            .filter(|n| !n.kind.outputs().is_empty())
            .map(|n| (n.id, n.x + NODE_W + 70.0, n.y));
        let (x, y, from) = match link {
            Some((id, x, y)) => (x, y, Some(id)),
            None => {
                let k = (self.view().nodes.len() % 6) as f32;
                let (x, y) = self.screen_to_canvas(260.0, 120.0);
                (x + k * 26.0, y + k * 26.0, None)
            }
        };
        let path = self.view.clone();
        let new_id = self.doc.add_node_in(&path, kind, x, y);
        if let Some(from) = from {
            if self.doc.next(from, 0).is_none() {
                self.doc.connect(from, 0, new_id);
            }
        }
        self.clear_selection();
        self.selected = Some(new_id);
        self.dirty = true;
    }

    /// Add a step at a spot on the canvas (from the right-click menu), not linked to anything.
    fn add_step_at(&mut self, kind: NodeKind, x: f32, y: f32) {
        self.commit_edit();
        let kind = match kind {
            NodeKind::Group { graph, .. } if graph.nodes.is_empty() => self.doc.new_group_kind(),
            k => k,
        };
        let path = self.view.clone();
        let id = self.doc.add_node_in(&path, kind, (x - 24.0).round(), (y - 16.0).round());
        self.clear_selection();
        self.selected = Some(id);
        self.side_tab = SideTab::Edit;
        self.sidebar_open = true;
        self.dirty = true;
    }

    fn select_all(&mut self) {
        let ids: Vec<u64> = self.view().nodes.iter().map(|n| n.id).collect();
        self.clear_selection();
        let mut it = ids.into_iter();
        self.selected = it.next();
        self.extra = it.collect();
        if self.selected.is_some() {
            self.side_tab = SideTab::Edit;
        }
    }

    /// Remove every connection into and out of a step.
    fn disconnect_all(&mut self, id: u64) {
        if let Some(g) = self.doc.root.owner_mut(id) {
            let before = g.edges.len();
            g.edges.retain(|e| e.from != id && e.to != id);
            if g.edges.len() != before {
                self.dirty = true;
            }
        }
    }

    fn delete_selected(&mut self) {
        let ids = self.selection();
        if ids.is_empty() {
            return;
        }
        let mut removed = 0;
        for id in ids {
            match self.doc.node(id).map(|n| n.kind.clone()) {
                Some(k) if k.is_fixed() => {
                    self.status = format!("The {} step can't be deleted.", k.title());
                }
                Some(_) => {
                    self.doc.remove_node(id);
                    removed += 1;
                }
                None => {}
            }
        }
        if removed > 0 {
            self.clear_selection();
            self.dirty = true;
        }
    }

    fn duplicate_selected(&mut self) {
        let Some(n) = self.selected.and_then(|id| self.doc.node(id)).cloned() else {
            return;
        };
        if n.kind.is_fixed() {
            return;
        }
        let mut kind = n.kind.clone();
        self.doc.refresh_ids(&mut kind);
        let path = self.view.clone();
        let id = self.doc.add_node_in(&path, kind, n.x + 30.0, n.y + 30.0);
        if let Some(copy) = self.doc.node_mut(id) {
            copy.delay_ms = n.delay_ms;
            copy.note = n.note;
            copy.name = n.name;
        }
        self.clear_selection();
        self.selected = Some(id);
        self.dirty = true;
    }

    fn update_node(&mut self, id: u64, f: impl FnOnce(&mut NodeKind)) {
        if let Some(n) = self.doc.node_mut(id) {
            f(&mut n.kind);
            self.dirty = true;
        }
    }

    fn new_doc(&mut self) {
        if !self.idle() {
            return;
        }
        self.doc = fresh_macro();
        self.path = None;
        self.dirty = false;
        self.selected = None;
        self.extra.clear();
        self.view.clear();
        self.last_end = None;
        self.editing = None;
        self.pan = (0.0, 0.0);
        self.zoom = 1.0;
        self.log.clear();
        self.reset_replay();
        self.results.clear();
        self.live_vars.clear();
        self.reset_history();
        self.recent = recent_macros();
        self.show_start = true;
        self.status = "New macro.".into();
    }

    /// Start from one of the ready-made macros.
    fn use_template(&mut self, i: usize) {
        let Some((name, _, mut doc)) = templates().into_iter().nth(i) else { return };
        if !self.idle() {
            return;
        }
        doc.reference = vision::primary_screen().ok();
        self.replace_doc(doc, None);
        self.dirty = true;
        self.status = format!("“{name}” is ready. Steps with a ⚠ still need setting up.");
    }

    /// Bring back unsaved work from last time.
    fn restore_autosave(&mut self) {
        let Some(a) = self.autosave_found.take() else { return };
        if !self.idle() {
            return;
        }
        self.replace_doc(a.doc, a.path);
        self.dirty = true;
        self.status = "Restored your unsaved work. Save it to keep it.".into();
    }

    /// Show a different macro (without asking about the old one).
    fn replace_doc(&mut self, doc: Macro, path: Option<PathBuf>) {
        self.doc = doc;
        self.path = path;
        self.selected = None;
        self.extra.clear();
        self.view.clear();
        self.last_end = None;
        self.editing = None;
        self.pan = (0.0, 0.0);
        self.zoom = 1.0;
        self.log.clear();
        self.reset_replay();
        self.results.clear();
        self.live_vars.clear();
        self.reset_history();
        self.show_start = false;
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        if !self.idle() {
            return;
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open macro".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.into_iter().next() {
                    let _ = this.update(cx, |app, cx| {
                        app.load(p);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn load(&mut self, path: PathBuf) {
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<Macro>(&s).map_err(|e| e.to_string()));
        match parsed {
            Ok(doc) => {
                remember_recent(&path);
                self.recent = recent_macros();
                self.show_start = false;
                self.results.clear();
                self.live_vars.clear();
                self.doc = doc;
                self.path = Some(path.clone());
                self.dirty = false;
                self.selected = None;
                self.extra.clear();
                self.view.clear();
                self.last_end = None;
                self.editing = None;
                self.pan = (0.0, 0.0);
                self.zoom = 1.0;
                self.log.clear();
        self.reset_replay();
                self.reset_history();
                let file = path.file_name().map(|f| f.to_string_lossy().to_string());
                self.status = format!("Opened {}", file.unwrap_or_else(|| path.display().to_string()));
            }
            Err(e) => self.status = format!("Couldn't open {}: {e}", path.display()),
        }
    }

    fn save(&mut self, save_as: bool, cx: &mut Context<Self>) {
        self.commit_edit();
        if let (Some(p), false) = (self.path.clone(), save_as) {
            self.write_to(p);
            return;
        }
        let dir = app_dir();
        let _ = std::fs::create_dir_all(&dir);
        let suggested = format!("{}.json", safe_file_name(&self.doc.name));
        let rx = cx.prompt_for_new_path(&dir, Some(suggested.as_str()));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(p))) = rx.await {
                let _ = this.update(cx, |app, cx| {
                    app.write_to(p);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn write_to(&mut self, path: PathBuf) {
        let result = serde_json::to_string_pretty(&self.doc)
            .map_err(|e| e.to_string())
            .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                remember_recent(&path);
                self.recent = recent_macros();
                clear_autosave();
                self.path = Some(path);
                self.dirty = false;
            }
            Err(e) => self.status = format!("Couldn't save: {e}"),
        }
    }

    fn choose_template(&mut self, id: u64, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use image".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(p) = paths.into_iter().next() {
                    let _ = this.update(cx, |app, cx| {
                        app.update_node(id, |k| {
                            if let NodeKind::FindImage { template, .. } = k {
                                *template = Some(p);
                            }
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    // -----------------------------------------------------------------------
    // Inspector fields
    // -----------------------------------------------------------------------

    fn start_edit(&mut self, field: Field, window: &mut Window) {
        self.commit_edit();
        self.capturing_key = false;
        let value = self.get_field(field);
        // Everything selected to begin with, so typing replaces it (a click then
        // puts the cursor where you clicked).
        self.edit_cursor = value.chars().count();
        self.edit_anchor = Some(0);
        self.editing = Some((field, value));
        self.focus.focus(window);
    }

    fn commit_edit(&mut self) {
        if let Some((f, buf)) = self.editing.take() {
            match self.set_field(f, &buf) {
                Ok(()) => self.dirty = true,
                Err(e) => self.status = e,
            }
        }
    }

    fn get_field(&self, f: Field) -> String {
        let d = &self.doc;
        match f {
            Field::MacroName => return d.name.clone(),
            Field::StartDelay => return d.start_delay_ms.to_string(),
            Field::StepDelay => return d.step_delay_ms.to_string(),
            Field::Speed => return d.speed_pct.to_string(),
            Field::Repeat => return d.repeat.to_string(),
            Field::BulkDelay => return self.bulk_delay.to_string(),
            Field::SnipSize => return self.snip_size.to_string(),
            Field::MouseSpeed => return d.mouse_speed.to_string(),
            _ => {}
        }
        let Some(n) = self.selected.and_then(|id| d.node(id)) else {
            return String::new();
        };
        match (f, &n.kind) {
            (Field::StepName, _) => n.name.clone(),
            (Field::Note, _) => n.note.clone(),
            (Field::Delay, _) => n.delay_ms.to_string(),
            (
                Field::X,
                NodeKind::Click { x, .. }
                | NodeKind::MoveMouse { x, .. }
                | NodeKind::ReadText { x, .. }
                | NodeKind::PixelColor { x, .. },
            ) => x.to_string(),
            (
                Field::Y,
                NodeKind::Click { y, .. }
                | NodeKind::MoveMouse { y, .. }
                | NodeKind::ReadText { y, .. }
                | NodeKind::PixelColor { y, .. },
            ) => y.to_string(),
            (Field::VarName, NodeKind::SetVar { name, .. }) => name.clone(),
            (Field::VarExpr, NodeKind::SetVar { expr, .. }) => expr.clone(),
            (Field::IfLeft, NodeKind::IfCompare { left, .. }) => left.clone(),
            (Field::IfRight, NodeKind::IfCompare { right, .. }) => right.clone(),
            (Field::ReadSaveTo, NodeKind::ReadText { save_to, .. }) => save_to.clone(),
            (Field::PixelHex, NodeKind::PixelColor { color, .. }) => format!("#{color:06X}"),
            (Field::PixelTol, NodeKind::PixelColor { tolerance, .. }) => tolerance.to_string(),
            (Field::DragFromX, NodeKind::Drag { from_x, .. }) => from_x.to_string(),
            (Field::DragFromY, NodeKind::Drag { from_y, .. }) => from_y.to_string(),
            (Field::DragToX, NodeKind::Drag { to_x, .. }) => to_x.to_string(),
            (Field::DragToY, NodeKind::Drag { to_y, .. }) => to_y.to_string(),
            (Field::DragMs, NodeKind::Drag { duration_ms, .. }) => duration_ms.to_string(),
            (Field::DragTaps, NodeKind::Drag { taps, .. }) => taps.to_string(),
            (Field::W, NodeKind::ReadText { w, .. }) => w.to_string(),
            (Field::H, NodeKind::ReadText { h, .. }) => h.to_string(),
            (Field::CompareValue, NodeKind::ReadText { value, .. }) => value.clone(),
            (Field::WinText, NodeKind::WindowCheck { text, .. }) => text.clone(),
            (Field::GroupName, NodeKind::Group { name, .. }) => name.clone(),
            (Field::ToastTitle, NodeKind::Notify { title, .. }) => title.clone(),
            (Field::ToastText, NodeKind::Notify { message, .. }) => message.clone(),
            (Field::ToastMs, NodeKind::Notify { duration_ms, .. }) => duration_ms.to_string(),
            (
                Field::ClickJitter,
                NodeKind::Click { jitter_px, .. } | NodeKind::MoveMouse { jitter_px, .. },
            ) => jitter_px.to_string(),
            (Field::AreaRotation, k) if k.area_parts().is_some() => {
                format!("{:.0}", k.area_parts().map_or(0.0, |(_, sh)| sh.rotation))
            }
            (Field::AreaHollow, k) if k.area_parts().is_some() => {
                k.area_parts().map_or(0, |(_, sh)| sh.hollow_px).to_string()
            }
            (Field::DragHold, NodeKind::Drag { hold_ms, .. }) => hold_ms.to_string(),
            (
                Field::StepMouseSpeed,
                NodeKind::Click { move_speed, .. } | NodeKind::MoveMouse { move_speed, .. },
            ) => move_speed.to_string(),
            (
                Field::OffX,
                NodeKind::Click { offset_x, .. } | NodeKind::MoveMouse { offset_x, .. },
            ) => offset_x.to_string(),
            (
                Field::OffY,
                NodeKind::Click { offset_y, .. } | NodeKind::MoveMouse { offset_y, .. },
            ) => offset_y.to_string(),
            (Field::Clicks, NodeKind::Click { clicks, .. }) => clicks.to_string(),
            (Field::Text, NodeKind::TypeText { text, .. }) => text.clone(),
            (Field::WaitMs, NodeKind::Wait { ms, .. }) => ms.to_string(),
            (Field::Jitter, NodeKind::Wait { jitter_ms, .. }) => jitter_ms.to_string(),
            (Field::ScrollAmt, NodeKind::Scroll { amount, .. }) => amount.to_string(),
            (Field::ScrollStrength, NodeKind::Scroll { strength, .. }) => strength.to_string(),
            (Field::RecSpeed, NodeKind::Recording { speed, .. }) => format!("{speed:.2}"),
            (Field::RecDelay(i), NodeKind::Recording { events, .. }) => events
                .get(i)
                .map(|e| e.delay_ms.to_string())
                .unwrap_or_default(),
            (Field::Threshold, NodeKind::FindImage { threshold, .. }) => format!("{threshold:.2}"),
            (
                Field::Timeout,
                NodeKind::FindImage { timeout_ms, .. }
                | NodeKind::ReadText { timeout_ms, .. }
                | NodeKind::WindowCheck { timeout_ms, .. }
                | NodeKind::PixelColor { timeout_ms, .. },
            ) => timeout_ms.to_string(),
            (
                Field::Interval,
                NodeKind::FindImage { interval_ms, .. }
                | NodeKind::ReadText { interval_ms, .. }
                | NodeKind::WindowCheck { interval_ms, .. }
                | NodeKind::PixelColor { interval_ms, .. },
            ) => interval_ms.to_string(),
            (Field::LoopCount, NodeKind::Loop { count }) => count.to_string(),
            _ => String::new(),
        }
    }

    fn set_field(&mut self, f: Field, raw: &str) -> Result<(), String> {
        let s = raw.trim();
        let int = || -> Result<i64, String> {
            s.parse::<f64>()
                .map(|v| v.round() as i64)
                .map_err(|_| format!("“{s}” isn't a number."))
        };
        let num = || -> Result<f32, String> {
            s.parse::<f32>().map_err(|_| format!("“{s}” isn't a number."))
        };
        match f {
            Field::MacroName => {
                self.doc.name = if s.is_empty() { "Untitled macro".into() } else { s.to_string() };
                return Ok(());
            }
            Field::StartDelay => {
                self.doc.start_delay_ms = int()?.max(0) as u64;
                return Ok(());
            }
            Field::StepDelay => {
                self.doc.step_delay_ms = int()?.max(0) as u64;
                return Ok(());
            }
            Field::Speed => {
                self.doc.speed_pct = int()?.clamp(5, 2000) as u32;
                return Ok(());
            }
            Field::Repeat => {
                self.doc.repeat = int()?.max(0) as u32;
                return Ok(());
            }
            Field::BulkDelay => {
                self.bulk_delay = int()?.max(0) as u64;
                return Ok(());
            }
            Field::SnipSize => {
                self.snip_size = int()?.clamp(12, 400) as u32;
                return Ok(());
            }
            Field::MouseSpeed => {
                self.doc.mouse_speed = int()?.clamp(10, 1000) as u32;
                return Ok(());
            }
            _ => {}
        }
        let Some(id) = self.selected else { return Ok(()) };
        let Some(n) = self.doc.node_mut(id) else { return Ok(()) };
        match (f, &mut n.kind) {
            (Field::StepName, _) => n.name = raw.trim().to_string(),
            (Field::Note, _) => n.note = raw.to_string(),
            (Field::Delay, _) => n.delay_ms = int()?.max(0) as u64,
            (
                Field::X,
                NodeKind::Click { x, .. }
                | NodeKind::MoveMouse { x, .. }
                | NodeKind::ReadText { x, .. }
                | NodeKind::PixelColor { x, .. },
            ) => *x = int()? as i32,
            (
                Field::Y,
                NodeKind::Click { y, .. }
                | NodeKind::MoveMouse { y, .. }
                | NodeKind::ReadText { y, .. }
                | NodeKind::PixelColor { y, .. },
            ) => *y = int()? as i32,
            (Field::VarName, NodeKind::SetVar { name, .. }) => {
                let t = raw.trim();
                if !t.is_empty() && !is_ident(t) {
                    return Err(format!("“{t}” can't be a variable name. Use letters, numbers and _ (e.g. gold_now)."));
                }
                *name = t.to_string()
            }
            (Field::VarExpr, NodeKind::SetVar { expr, .. }) => *expr = raw.to_string(),
            (Field::IfLeft, NodeKind::IfCompare { left, .. }) => *left = raw.to_string(),
            (Field::IfRight, NodeKind::IfCompare { right, .. }) => *right = raw.to_string(),
            (Field::ReadSaveTo, NodeKind::ReadText { save_to, .. }) => {
                let t = raw.trim();
                if !t.is_empty() && !is_ident(t) {
                    return Err(format!("“{t}” can't be a variable name. Use letters, numbers and _ (e.g. gold_now)."));
                }
                *save_to = t.to_string()
            }
            (Field::PixelHex, NodeKind::PixelColor { color, .. }) => {
                let h = raw.trim().trim_start_matches('#');
                *color = u32::from_str_radix(h, 16)
                    .ok()
                    .filter(|_| h.len() == 6)
                    .ok_or_else(|| format!("“{}” isn't a colour – use #RRGGBB, e.g. #34C77B.", raw.trim()))?
            }
            (Field::PixelTol, NodeKind::PixelColor { tolerance, .. }) => *tolerance = int()?.clamp(0, 255) as u32,
            (Field::DragFromX, NodeKind::Drag { from_x, .. }) => *from_x = int()? as i32,
            (Field::DragFromY, NodeKind::Drag { from_y, .. }) => *from_y = int()? as i32,
            (Field::DragToX, NodeKind::Drag { to_x, .. }) => *to_x = int()? as i32,
            (Field::DragToY, NodeKind::Drag { to_y, .. }) => *to_y = int()? as i32,
            (Field::DragMs, NodeKind::Drag { duration_ms, .. }) => *duration_ms = int()?.clamp(0, 20_000) as u64,
            (Field::DragTaps, NodeKind::Drag { taps, .. }) => *taps = int()?.clamp(0, 500) as u32,
            (Field::W, NodeKind::ReadText { w, .. }) => *w = int()?.clamp(0, 10_000) as i32,
            (Field::H, NodeKind::ReadText { h, .. }) => *h = int()?.clamp(0, 10_000) as i32,
            (Field::CompareValue, NodeKind::ReadText { value, .. }) => *value = raw.to_string(),
            (Field::WinText, NodeKind::WindowCheck { text, .. }) => *text = raw.to_string(),
            (Field::GroupName, NodeKind::Group { name, .. }) => *name = raw.trim().to_string(),
            (Field::ToastTitle, NodeKind::Notify { title, .. }) => *title = raw.to_string(),
            (Field::ToastText, NodeKind::Notify { message, .. }) => *message = raw.to_string(),
            (Field::ToastMs, NodeKind::Notify { duration_ms, .. }) => {
                *duration_ms = int()?.clamp(500, 120_000) as u64
            }
            (
                Field::ClickJitter,
                NodeKind::Click { jitter_px, .. } | NodeKind::MoveMouse { jitter_px, .. },
            ) => *jitter_px = int()?.clamp(0, 500) as i32,
            (Field::AreaRotation, k) if k.area_parts().is_some() => {
                // Keep it between -180° and 180° (so 190 becomes -170).
                let v = int()? as f32;
                if let Some((_, sh)) = k.area_parts_mut() {
                    sh.rotation = (v + 180.0).rem_euclid(360.0) - 180.0;
                }
            }
            (Field::AreaHollow, k) if k.area_parts().is_some() => {
                let v = int()?.clamp(0, 5000) as i32;
                if let Some((_, sh)) = k.area_parts_mut() {
                    sh.hollow_px = v;
                }
            }
            (Field::DragHold, NodeKind::Drag { hold_ms, .. }) => *hold_ms = int()?.clamp(0, 10_000) as u64,
            (
                Field::StepMouseSpeed,
                NodeKind::Click { move_speed, .. } | NodeKind::MoveMouse { move_speed, .. },
            ) => {
                // 0 = use the macro's speed; otherwise 10–1000 %.
                let v = int()?.max(0) as u32;
                *move_speed = if v == 0 { 0 } else { v.clamp(10, 1000) };
            }
            (
                Field::OffX,
                NodeKind::Click { offset_x, .. } | NodeKind::MoveMouse { offset_x, .. },
            ) => *offset_x = int()? as i32,
            (
                Field::OffY,
                NodeKind::Click { offset_y, .. } | NodeKind::MoveMouse { offset_y, .. },
            ) => *offset_y = int()? as i32,
            (Field::Clicks, NodeKind::Click { clicks, .. }) => *clicks = int()?.clamp(1, 10) as u32,
            (Field::Text, NodeKind::TypeText { text, .. }) => *text = raw.to_string(),
            (Field::WaitMs, NodeKind::Wait { ms, .. }) => *ms = int()?.max(0) as u64,
            (Field::Jitter, NodeKind::Wait { jitter_ms, .. }) => *jitter_ms = int()?.max(0) as u64,
            (Field::ScrollAmt, NodeKind::Scroll { amount, .. }) => {
                *amount = int()?.clamp(-500, 500) as i32
            }
            (Field::ScrollStrength, NodeKind::Scroll { strength, .. }) => {
                *strength = int()?.clamp(1, 20) as u32
            }
            (Field::RecSpeed, NodeKind::Recording { speed, .. }) => *speed = num()?.clamp(0.05, 20.0),
            (Field::RecDelay(i), NodeKind::Recording { events, .. }) => {
                let v = int()?.max(0) as u64;
                if let Some(e) = events.get_mut(i) {
                    e.delay_ms = v;
                }
            }
            (Field::Threshold, NodeKind::FindImage { threshold, .. }) => {
                *threshold = num()?.clamp(0.3, 0.999)
            }
            (
                Field::Timeout,
                NodeKind::FindImage { timeout_ms, .. }
                | NodeKind::ReadText { timeout_ms, .. }
                | NodeKind::WindowCheck { timeout_ms, .. }
                | NodeKind::PixelColor { timeout_ms, .. },
            ) => *timeout_ms = int()?.max(0) as u64,
            (
                Field::Interval,
                NodeKind::FindImage { interval_ms, .. }
                | NodeKind::ReadText { interval_ms, .. }
                | NodeKind::WindowCheck { interval_ms, .. }
                | NodeKind::PixelColor { interval_ms, .. },
            ) => *interval_ms = int()?.clamp(50, 60_000) as u64,
            (Field::LoopCount, NodeKind::Loop { count }) => *count = int()?.max(0) as u32,
            _ => {}
        }
        Ok(())
    }

    fn bump(&mut self, f: Field, delta: f64) {
        self.commit_edit();
        let cur: f64 = self.get_field(f).trim().parse().unwrap_or(0.0);
        let next = cur + delta;
        let s = if f.is_decimal() {
            format!("{next:.2}")
        } else {
            format!("{}", next.round() as i64)
        };
        match self.set_field(f, &s) {
            Ok(()) => self.dirty = true,
            Err(e) => self.status = e,
        }
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    /// Window position -> canvas position (undoing pan and zoom).
    fn to_local(&self, p: Point<Pixels>) -> (f32, f32) {
        let (ox, oy) = self.canvas_origin.get();
        self.screen_to_canvas(f32::from(p.x) - ox, f32::from(p.y) - oy)
    }

    /// Position on the canvas element -> canvas position.
    fn screen_to_canvas(&self, sx: f32, sy: f32) -> (f32, f32) {
        ((sx - self.pan.0) / self.zoom, (sy - self.pan.1) / self.zoom)
    }

    // -----------------------------------------------------------------------
    // Zoom
    // -----------------------------------------------------------------------

    /// Zoom so that the point (sx, sy) on the canvas element stays where it is.
    fn zoom_around(&mut self, new_zoom: f32, sx: f32, sy: f32) {
        let new_zoom = new_zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let (cx_, cy_) = self.screen_to_canvas(sx, sy);
        self.zoom = new_zoom;
        self.pan = (sx - cx_ * new_zoom, sy - cy_ * new_zoom);
    }

    fn canvas_centre(&self) -> (f32, f32) {
        let (w, h) = self.canvas_size.get();
        if w > 0.0 && h > 0.0 {
            (w / 2.0, h / 2.0)
        } else {
            (400.0, 300.0)
        }
    }

    fn zoom_by(&mut self, factor: f32) {
        let (sx, sy) = self.canvas_centre();
        self.zoom_around(self.zoom * factor, sx, sy);
    }

    fn zoom_reset(&mut self) {
        let (sx, sy) = self.canvas_centre();
        self.zoom_around(1.0, sx, sy);
    }

    /// Zoom and pan so every step you're looking at fits on screen.
    fn zoom_fit(&mut self) {
        let nodes = &self.view().nodes;
        if nodes.is_empty() {
            self.zoom = 1.0;
            self.pan = (0.0, 0.0);
            return;
        }
        let min_x = nodes.iter().map(|n| n.x).fold(f32::MAX, f32::min) - 16.0;
        let min_y = nodes.iter().map(|n| n.y).fold(f32::MAX, f32::min) - 30.0;
        let max_x = nodes.iter().map(|n| n.x + NODE_W).fold(f32::MIN, f32::max) + 16.0;
        let max_y = nodes.iter().map(|n| n.y + NODE_H).fold(f32::MIN, f32::max) + 16.0;
        let (w, h) = self.canvas_size.get();
        let (w, h) = if w > 0.0 && h > 0.0 { (w, h) } else { (800.0, 600.0) };
        let (bw, bh) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
        let z = ((w - 80.0) / bw).min((h - 80.0) / bh).clamp(ZOOM_MIN, 1.25);
        self.zoom = z;
        self.pan = (
            w / 2.0 - (min_x + bw / 2.0) * z,
            h / 2.0 - (min_y + bh / 2.0) * z,
        );
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let m = ks.modifiers;

        if self.shape_edit.is_some() {
            self.edit_key(key, m.shift);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.quick.is_some() {
            self.quick_key(ev, cx);
            return;
        }
        if self.run_check.is_some() {
            match key {
                "escape" => self.run_check = None,
                "enter" | "return" => {
                    if let Some(c) = self.run_check.take() {
                        self.run_now(c.from, c.stepping);
                    }
                }
                _ => {}
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // Going through the run log: Space plays / pauses, ← → step, Esc stops.
        if self.log_sel.is_some() && self.editing.is_none() && self.menu.is_none() && self.idle() {
            let handled = match key {
                "space" => {
                    self.replay_toggle();
                    true
                }
                "left" | "up" => {
                    self.replay_step(false);
                    true
                }
                "right" | "down" => {
                    self.replay_step(true);
                    true
                }
                "escape" => {
                    self.reset_replay();
                    true
                }
                _ => false,
            };
            if handled {
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        // Esc closes an open right-click menu first.
        if self.menu.is_some() && key == "escape" {
            self.menu = None;
            cx.stop_propagation();
            cx.notify();
            return;
        }

        if self.capturing_key {
            cx.stop_propagation();
            if key == "escape" && !m.modified() {
                self.capturing_key = false;
                self.status = "Key capture cancelled.".into();
                cx.notify();
                return;
            }
            match keys::from_gpui(key) {
                Some(name) if !keys::is_modifier(&name) => {
                    let mods = Mods {
                        cmd: m.platform,
                        shift: m.shift,
                        alt: m.alt,
                        ctrl: m.control,
                    };
                    let label = keys::combo_label(&name, &mods);
                    if let Some(id) = self.selected {
                        self.update_node(id, |k| {
                            if let NodeKind::KeyPress { key, mods: md } = k {
                                *key = name;
                                *md = mods;
                            }
                        });
                    }
                    self.capturing_key = false;
                    self.status = format!("Key set to {label}.");
                }
                Some(_) => {}
                None => self.status = format!("MacroFlow can't send the “{key}” key."),
            }
            cx.notify();
            return;
        }

        if let Some((f, buf)) = self.editing.take() {
            cx.stop_propagation();
            let mut ed = TextEdit::new(&buf, self.edit_cursor, self.edit_anchor);
            let len = ed.chars.len();
            let shift = m.shift;
            // `{name}` pills move and delete as one block.
            let pills: Vec<(usize, usize)> = f.edit_pills(&buf).iter().map(|r| (r.start, r.end)).collect();
            let pill_ending = |at: usize| pills.iter().find(|p| p.1 == at).map(|p| p.0);
            let pill_starting = |at: usize| pills.iter().find(|p| p.0 == at).map(|p| p.1);
            // Word jumps: ⌥ on a Mac, Ctrl on Windows. ⌘ ← / → on a Mac jumps to the start / end.
            let word = if IS_MACOS { m.alt } else { m.control };
            let line = IS_MACOS && m.platform;
            // The shortcut key: ⌘ on a Mac, Ctrl on Windows (but not Ctrl+Alt, which is AltGr).
            let cmd = if IS_MACOS { m.platform } else { m.control && !m.alt };
            let mut done: Option<bool> = None; // Some(true) = save, Some(false) = cancel
            match key {
                "enter" | "return" | "tab" => done = Some(true),
                "escape" => done = Some(false),
                "left" => {
                    let to = if line {
                        0
                    } else if word {
                        prev_word(&ed.chars, ed.cur)
                    } else if let (false, Some((a, _))) = (shift, ed.selection()) {
                        a
                    } else {
                        pill_ending(ed.cur).unwrap_or(ed.cur.saturating_sub(1))
                    };
                    ed.move_to(to, shift);
                }
                "right" => {
                    let to = if line {
                        len
                    } else if word {
                        next_word(&ed.chars, ed.cur)
                    } else if let (false, Some((_, b))) = (shift, ed.selection()) {
                        b
                    } else {
                        pill_starting(ed.cur).unwrap_or((ed.cur + 1).min(len))
                    };
                    ed.move_to(to, shift);
                }
                "home" | "up" => ed.move_to(0, shift),
                "end" | "down" => ed.move_to(len, shift),
                "backspace" => {
                    if !ed.delete_selection() {
                        let from = if line {
                            0
                        } else if word {
                            prev_word(&ed.chars, ed.cur)
                        } else {
                            pill_ending(ed.cur).unwrap_or(ed.cur.saturating_sub(1))
                        };
                        ed.delete(from, ed.cur);
                    }
                }
                "delete" => {
                    if !ed.delete_selection() {
                        let to = if line {
                            len
                        } else if word {
                            next_word(&ed.chars, ed.cur)
                        } else {
                            pill_starting(ed.cur).unwrap_or((ed.cur + 1).min(len))
                        };
                        ed.delete(ed.cur, to);
                    }
                }
                "a" if cmd => {
                    ed.anchor = Some(0);
                    ed.cur = len;
                }
                "c" if cmd => {
                    if ed.selection().is_some() {
                        cx.write_to_clipboard(ClipboardItem::new_string(ed.selected_text()));
                    }
                }
                "x" if cmd => {
                    if ed.selection().is_some() {
                        cx.write_to_clipboard(ClipboardItem::new_string(ed.selected_text()));
                        ed.delete_selection();
                    }
                }
                "v" if cmd => {
                    if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        ed.insert(&t.replace(['\r', '\n'], " "));
                    }
                }
                _ => {
                    // Ctrl+Alt is AltGr on Windows, which types characters like @ and €.
                    let shortcut = if IS_MACOS { m.platform || m.control } else { m.control && !m.alt };
                    if !shortcut {
                        match &ks.key_char {
                            Some(ch) => ed.insert(ch),
                            None if key == "space" => ed.insert(" "),
                            None => {}
                        }
                    }
                }
            }
            match done {
                Some(true) => {
                    self.editing = Some((f, ed.text()));
                    self.commit_edit();
                }
                Some(false) => {}
                None => {
                    self.edit_cursor = ed.cur;
                    self.edit_anchor = ed.anchor;
                    self.editing = Some((f, ed.text()));
                }
            }
            cx.notify();
            return;
        }

        if m.secondary() {
            match key {
                "s" => self.save(m.shift, cx),
                "o" => self.open(cx),
                "n" => self.new_doc(),
                "d" => self.duplicate_selected(),
                "e" => self.toggle_disabled(),
                "r" => self.start_run(),
                "k" => self.open_quick(None, None, None),
                "b" => self.sidebar_open = !self.sidebar_open,
                "g" if m.shift => self.ungroup_selected(),
                "g" => self.group_selection(),
                "a" => self.select_all(),
                "z" if m.shift => self.redo(),
                "z" => self.undo(),
                "y" => self.redo(),
                "c" => {
                    self.copy_selection(cx);
                }
                "x" => self.cut_selection(cx),
                "v" => self.paste(None, cx),
                "=" | "+" => self.zoom_by(1.2),
                "-" | "_" => self.zoom_by(1.0 / 1.2),
                "0" => self.zoom_reset(),
                "1" => self.zoom_fit(),
                // ⌃⌘F on macOS
                "f" if IS_MACOS && m.control => window.toggle_fullscreen(),
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        match key {
            "backspace" | "delete" => self.delete_selected(),
            "escape" => {
                self.drag = Drag::None;
                if self.selected.is_some() || !self.extra.is_empty() {
                    self.clear_selection();
                } else if !self.view.is_empty() {
                    let depth = self.view.len() - 1;
                    self.leave_group_to(depth);
                }
            }
            "enter" => {
                if let Some(id) = self.selected {
                    self.enter_group(id);
                }
            }
            "f2" => {
                if let Some(id) = self.selected {
                    self.run_menu_action(MenuAction::Rename(id), window, cx);
                }
            }
            "f11" if !IS_MACOS => window.toggle_fullscreen(),
            _ => return,
        }
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Right-click menu
    // -----------------------------------------------------------------------

    fn open_step_menu(&mut self, id: u64, pos: Point<Pixels>) {
        self.commit_edit();
        self.capturing_key = false;
        self.drag = Drag::None;
        // Right-clicking a step outside the selection selects just that step.
        if !self.selection().contains(&id) {
            self.clear_selection();
            self.selected = Some(id);
        }
        self.menu = Some(ContextMenu {
            x: f32::from(pos.x),
            y: f32::from(pos.y),
            target: MenuTarget::Step(id),
            add_open: false,
        });
    }

    fn open_canvas_menu(&mut self, pos: Point<Pixels>) {
        self.commit_edit();
        self.capturing_key = false;
        self.drag = Drag::None;
        let (x, y) = self.to_local(pos);
        self.menu = Some(ContextMenu {
            x: f32::from(pos.x),
            y: f32::from(pos.y),
            target: MenuTarget::Canvas { x, y },
            add_open: false,
        });
    }

    fn menu_entries(&self, menu: &ContextMenu) -> Vec<MenuEntry> {
        let item = |label: &str, hint: &'static str, action: MenuAction, enabled: bool| MenuEntry::Item {
            label: label.to_string(),
            hint,
            action,
            enabled,
            danger: false,
        };
        let danger = |label: &str, hint: &'static str, action: MenuAction, enabled: bool| MenuEntry::Item {
            label: label.to_string(),
            hint,
            action,
            enabled,
            danger: true,
        };
        let idle = self.idle();
        let mut out = Vec::new();
        match menu.target {
            MenuTarget::Step(id) => {
                let Some(n) = self.doc.node(id) else { return out };
                let sel = self.selection();
                if sel.len() > 1 && sel.contains(&id) {
                    out.push(MenuEntry::Header(format!("{} steps selected", sel.len())));
                    out.push(item("Group these steps", hint("⌘G", "Ctrl+G"), MenuAction::Group, idle));
                    out.push(item("Copy", hint("⌘C", "Ctrl+C"), MenuAction::Copy, true));
                    out.push(item("Cut", hint("⌘X", "Ctrl+X"), MenuAction::Cut, idle));
                    let all_off = sel.iter().filter(|i| self.doc.can_disable(**i)).all(|i| {
                        self.doc.node(*i).map_or(false, |n| n.disabled)
                    });
                    out.push(item(
                        if all_off { "Enable these steps" } else { "Disable these steps" },
                        hint("⌘E", "Ctrl+E"),
                        MenuAction::ToggleDisabled,
                        idle && sel.iter().any(|i| self.doc.can_disable(*i)),
                    ));
                    out.push(MenuEntry::Sep);
                    out.push(item("Clear selection", "Esc", MenuAction::ClearSelection, true));
                    out.push(danger("Delete these steps", hint("⌫", "Del"), MenuAction::Delete, idle));
                    return out;
                }
                let marker = matches!(n.kind, NodeKind::GroupIn | NodeKind::GroupOut);
                out.push(MenuEntry::Header(n.label()));
                out.push(item("Edit settings", "", MenuAction::Edit(id), true));
                out.push(item("Rename…", "F2", MenuAction::Rename(id), idle && !marker));
                out.push(item(
                    "Run from here",
                    "",
                    MenuAction::RunFrom(id),
                    idle && !matches!(n.kind, NodeKind::GroupOut),
                ));
                out.push(item(
                    if n.breakpoint { "Remove breakpoint" } else { "Add breakpoint" },
                    "",
                    MenuAction::ToggleBreakpoint(id),
                    !marker,
                ));
                let what = if n.kind.is_group() { "group" } else { "step" };
                out.push(item(
                    &if n.disabled { format!("Enable {what}") } else { format!("Disable {what} (skip it)") },
                    hint("⌘E", "Ctrl+E"),
                    MenuAction::ToggleDisabled,
                    idle && !n.kind.is_fixed(),
                ));
                out.push(MenuEntry::Sep);
                if n.kind.is_group() {
                    out.push(item("Open group", "Enter", MenuAction::OpenGroup(id), true));
                    out.push(item("Ungroup", hint("⇧⌘G", "Ctrl+Shift+G"), MenuAction::Ungroup, idle));
                } else {
                    let run = if n.kind.is_fixed() { 0 } else { self.doc.straight_run(id).len() };
                    let label = if run >= 2 {
                        format!("Group this + next {} step{}", run - 1, if run == 2 { "" } else { "s" })
                    } else {
                        "Group…".to_string()
                    };
                    out.push(item(&label, hint("⌘G", "Ctrl+G"), MenuAction::Group, idle && run >= 2));
                }
                out.push(item("Duplicate", hint("⌘D", "Ctrl+D"), MenuAction::Duplicate, idle && !n.kind.is_fixed()));
                out.push(item("Copy", hint("⌘C", "Ctrl+C"), MenuAction::Copy, !n.kind.is_fixed()));
                out.push(item("Cut", hint("⌘X", "Ctrl+X"), MenuAction::Cut, idle && !n.kind.is_fixed()));
                let linked = self
                    .doc
                    .root
                    .owner(id)
                    .map_or(false, |g| g.edges.iter().any(|e| e.from == id || e.to == id));
                out.push(item("Disconnect all", "", MenuAction::Disconnect(id), idle && linked));
                out.push(MenuEntry::Sep);
                out.push(danger("Delete", hint("⌫", "Del"), MenuAction::Delete, idle && !n.kind.is_fixed()));
            }
            MenuTarget::Canvas { x, y } => {
                out.push(MenuEntry::AddSub);
                out.push(item("Paste here", hint("⌘V", "Ctrl+V"), MenuAction::Paste(Some((x, y))), idle));
                out.push(MenuEntry::Sep);
                out.push(item("Undo", hint("⌘Z", "Ctrl+Z"), MenuAction::Undo, idle && !self.undo.is_empty()));
                out.push(item("Redo", hint("⇧⌘Z", "Ctrl+Y"), MenuAction::Redo, idle && !self.redo.is_empty()));
                out.push(MenuEntry::Sep);
                out.push(item("Select all", hint("⌘A", "Ctrl+A"), MenuAction::SelectAll, !self.view().nodes.is_empty()));
                out.push(MenuEntry::Sep);
                out.push(item("Zoom to fit", hint("⌘1", "Ctrl+1"), MenuAction::ZoomFit, true));
                out.push(item(
                    &format!("Reset zoom ({:.0}%)", self.zoom * 100.0),
                    hint("⌘0", "Ctrl+0"),
                    MenuAction::ZoomReset,
                    (self.zoom - 1.0).abs() > 0.001,
                ));
                out.push(item("Full screen", hint("⌃⌘F", "F11"), MenuAction::FullScreen, true));
                if !self.view.is_empty() {
                    out.push(MenuEntry::Sep);
                    out.push(item("Back out of group", "Esc", MenuAction::LeaveGroup, true));
                }
            }
        }
        out
    }

    fn run_menu_action(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let select_only = |this: &mut Self, id: u64| {
            if !this.selection().contains(&id) {
                this.clear_selection();
                this.selected = Some(id);
            }
        };
        match action {
            MenuAction::Edit(id) => {
                self.clear_selection();
                self.selected = Some(id);
                self.side_tab = SideTab::Edit;
                self.sidebar_open = true;
            }
            MenuAction::Rename(id) => {
                self.clear_selection();
                self.selected = Some(id);
                self.side_tab = SideTab::Edit;
                self.sidebar_open = true;
                let field = if matches!(self.doc.node(id).map(|n| &n.kind), Some(NodeKind::Group { .. })) {
                    Field::GroupName
                } else {
                    Field::StepName
                };
                self.start_edit(field, window);
                self.status = "Type a name, then press Enter (Esc cancels).".into();
            }
            MenuAction::RunFrom(id) => self.start_run_from(id),
            MenuAction::Duplicate => self.duplicate_selected(),
            MenuAction::Group => self.group_selection(),
            MenuAction::OpenGroup(id) => self.enter_group(id),
            MenuAction::Ungroup => self.ungroup_selected(),
            MenuAction::Disconnect(id) => {
                select_only(self, id);
                self.disconnect_all(id);
            }
            MenuAction::Delete => self.delete_selected(),
            MenuAction::ClearSelection => self.clear_selection(),
            MenuAction::AddHere(i, x, y) => {
                if let Some(kind) = NodeKind::palette().into_iter().nth(i) {
                    self.add_step_at(kind, x, y);
                }
            }
            MenuAction::SelectAll => self.select_all(),
            MenuAction::ZoomFit => self.zoom_fit(),
            MenuAction::ZoomReset => self.zoom_reset(),
            MenuAction::FullScreen => window.toggle_fullscreen(),
            MenuAction::Copy => {
                self.copy_selection(cx);
            }
            MenuAction::Cut => self.cut_selection(cx),
            MenuAction::Paste(at) => self.paste(at, cx),
            MenuAction::Undo => self.undo(),
            MenuAction::ToggleBreakpoint(id) => self.toggle_breakpoint(id),
            MenuAction::ToggleDisabled => self.toggle_disabled(),
            MenuAction::Redo => self.redo(),
            MenuAction::LeaveGroup => {
                if !self.view.is_empty() {
                    let depth = self.view.len() - 1;
                    self.leave_group_to(depth);
                }
            }
        }
        cx.notify();
    }

    fn render_menu(&self, menu: &ContextMenu, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let entries = self.menu_entries(menu);
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let height: f32 = 8.0
            + entries
                .iter()
                .map(|e| match e {
                    MenuEntry::Header(_) => MENU_HEADER_H,
                    MenuEntry::Sep => MENU_SEP_H,
                    _ => MENU_ITEM_H,
                })
                .sum::<f32>();
        // Keep it inside the window.
        let x = menu.x.min(vw - MENU_W - 6.0).max(4.0);
        let y = menu.y.min(vh - height - 6.0).max(4.0);

        let mut list = div()
            .id("context-menu")
            .occlude()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(MENU_W))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded_lg()
            .bg(rgb(UI_RAISED))
            .border_1()
            .border_color(rgb(UI_EDGE))
            .shadow_lg()
            .text_sm()
            .text_color(rgb(TEXT))
            .cursor_default()
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                if this.menu.is_some() {
                    this.menu = None;
                    cx.notify();
                }
            }));

        let mut cursor_y = 4.0;
        let mut sub_at: Option<f32> = None;
        for (i, entry) in entries.into_iter().enumerate() {
            match entry {
                MenuEntry::Header(text) => {
                    list = list.child(
                        div()
                            .h(px(MENU_HEADER_H))
                            .px_2()
                            .flex()
                            .items_center()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(UI_FAINT))
                            .truncate()
                            .child(text),
                    );
                    cursor_y += MENU_HEADER_H;
                }
                MenuEntry::Sep => {
                    list = list.child(
                        div()
                            .h(px(MENU_SEP_H))
                            .px_1()
                            .flex()
                            .items_center()
                            .child(div().w_full().h(px(1.)).bg(rgb(UI_LINE))),
                    );
                    cursor_y += MENU_SEP_H;
                }
                MenuEntry::AddSub => {
                    sub_at = Some(cursor_y);
                    let open = menu.add_open;
                    list = list.child(
                        div()
                            .id(("menu-item", i))
                            .h(px(MENU_ITEM_H))
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded_md()
                            .when(open, |d| d.bg(rgb(UI_HOVER)))
                            .hover(|s| s.bg(rgb(UI_HOVER)))
                            .child(div().flex_1().child("Add step here"))
                            .child(div().text_color(rgb(MUTED)).child("›"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                if *hovered {
                                    if let Some(m) = this.menu.as_mut() {
                                        if !m.add_open {
                                            m.add_open = true;
                                            cx.notify();
                                        }
                                    }
                                }
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(m) = this.menu.as_mut() {
                                        m.add_open = !m.add_open;
                                    }
                                    cx.notify();
                                }),
                            ),
                    );
                    cursor_y += MENU_ITEM_H;
                }
                MenuEntry::Item {
                    label,
                    hint,
                    action,
                    enabled,
                    danger,
                } => {
                    let color = if danger { RED } else { TEXT };
                    let mut row = div()
                        .id(("menu-item", i))
                        .h(px(MENU_ITEM_H))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .text_color(rgb(color))
                        .child(div().flex_1().truncate().child(label))
                        .child(div().flex_none().text_xs().text_color(rgb(UI_FAINT)).child(hint))
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if *hovered {
                                if let Some(m) = this.menu.as_mut() {
                                    if m.add_open {
                                        m.add_open = false;
                                        cx.notify();
                                    }
                                }
                            }
                        }));
                    if enabled {
                        row = row
                            .hover(move |s| {
                                if danger {
                                    s.bg(rgb(RED)).text_color(rgb(0xffffff))
                                } else {
                                    s.bg(rgba((ACCENT << 8) | 0x3d)).text_color(rgb(0xffffff))
                                }
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.run_menu_action(action.clone(), window, cx);
                                }),
                            );
                    } else {
                        row = row.opacity(0.4).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()),
                        );
                    }
                    list = list.child(row);
                    cursor_y += MENU_ITEM_H;
                }
            }
        }

        let mut layer = div().absolute().top_0().left_0().child(list);

        // The "Add step here" list, to the side of the menu.
        if let (true, Some(row_y), MenuTarget::Canvas { x: cxp, y: cyp }) = (menu.add_open, sub_at, menu.target) {
            let kinds = NodeKind::palette();
            let sub_w = 230.0;
            let sub_h = 8.0 + kinds.len() as f32 * MENU_ITEM_H;
            let sx = if x + MENU_W + 2.0 + sub_w <= vw - 4.0 {
                x + MENU_W + 2.0
            } else {
                (x - sub_w - 2.0).max(4.0)
            };
            let sy = (y + row_y - 4.0).min(vh - sub_h - 6.0).max(4.0);
            let mut sub = div()
                .id("context-submenu")
                .occlude()
                .absolute()
                .left(px(sx))
                .top(px(sy))
                .w(px(sub_w))
                .p(px(4.))
                .flex()
                .flex_col()
                .rounded_lg()
                .bg(rgb(UI_RAISED))
                .border_1()
                .border_color(rgb(UI_EDGE))
                .shadow_lg()
                .text_sm()
                .text_color(rgb(TEXT))
                .cursor_default();
            for (i, kind) in kinds.into_iter().enumerate() {
                let color = kind.color();
                sub = sub.child(
                    div()
                        .id(("menu-add", i))
                        .h(px(MENU_ITEM_H))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .hover(|s| s.bg(rgba((ACCENT << 8) | 0x3d)).text_color(rgb(0xffffff)))
                        .child(
                            div()
                                .w(px(18.))
                                .flex_none()
                                .text_color(rgb(color))
                                .font_weight(FontWeight::BOLD)
                                .child(kind.icon()),
                        )
                        .child(div().flex_1().truncate().child(kind.title()))
                        // Acts on mouse-down: the click also counts as "outside the menu",
                        // which closes it, but this handler still gets the click.
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.run_menu_action(MenuAction::AddHere(i, cxp, cyp), window, cx);
                            }),
                        ),
                );
            }
            layer = layer.child(sub);
        }
        layer
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let (wx, wy) = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.mouse = (wx, wy);
        if let Some(ed) = &self.shape_edit {
            if ed.drag != EditDrag::None {
                if ev.pressed_button == Some(MouseButton::Left) {
                    self.edit_mouse_move(ev.position, ev.modifiers.shift);
                } else {
                    self.edit_mouse_up();
                }
                cx.notify();
            }
            return;
        }
        if ev.pressed_button != Some(MouseButton::Left) {
            if self.drag != Drag::None {
                self.drag = Drag::None;
                cx.notify();
            }
            return;
        }
        match self.drag {
            Drag::Node { id, dx, dy } => {
                let (lx, ly) = self.to_local(ev.position);
                if let Some(n) = self.doc.node(id) {
                    let (mx, my) = ((lx - dx).round() - n.x, (ly - dy).round() - n.y);
                    let sel = self.selection();
                    let moving = if sel.contains(&id) { sel } else { vec![id] };
                    for m in moving {
                        if let Some(n) = self.doc.node_mut(m) {
                            n.x += mx;
                            n.y += my;
                        }
                    }
                    self.dirty = true;
                }
                cx.notify();
            }
            Drag::Pan { x, y } => {
                self.pan.0 += wx - x;
                self.pan.1 += wy - y;
                self.drag = Drag::Pan { x: wx, y: wy };
                cx.notify();
            }
            Drag::Connect { .. } | Drag::Select { .. } => cx.notify(),
            Drag::None => {}
        }
    }

    fn node_mouse_down(&mut self, id: u64, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        // Control-click is right-click on a Mac.
        if IS_MACOS && ev.modifiers.control {
            self.open_step_menu(id, ev.position);
            self.focus.focus(window);
            cx.notify();
            return;
        }
        self.menu = None;
        self.commit_edit();
        self.capturing_key = false;
        // Double-click a group to open it.
        if ev.click_count >= 2 && matches!(self.doc.node(id).map(|n| &n.kind), Some(NodeKind::Group { .. })) {
            self.enter_group(id);
            self.drag = Drag::None;
            cx.notify();
            return;
        }
        // Shift-click adds or removes a step from the selection (for grouping).
        if ev.modifiers.shift {
            if self.selected == Some(id) {
                self.selected = self.extra.pop();
            } else if self.extra.contains(&id) {
                self.extra.retain(|e| *e != id);
            } else if self.selected.is_none() {
                self.selected = Some(id);
            } else {
                self.extra.push(id);
            }
            self.side_tab = SideTab::Edit;
            self.sidebar_open = true;
            self.focus.focus(window);
            cx.notify();
            return;
        }
        if !self.selection().contains(&id) {
            self.extra.clear();
        }
        if self.selected != Some(id) {
            // Show the clicked step's settings.
            self.side_tab = SideTab::Edit;
            self.sidebar_open = true;
        }
        if !self.extra.contains(&id) {
            self.selected = Some(id);
        }
        let (lx, ly) = self.to_local(ev.position);
        if let Some(n) = self.doc.node(id) {
            self.drag = Drag::Node {
                id,
                dx: lx - n.x,
                dy: ly - n.y,
            };
        }
        // Bring the step to the front.
        let nodes = &mut self.view_mut().nodes;
        if let Some(i) = nodes.iter().position(|n| n.id == id) {
            let n = nodes.remove(i);
            nodes.push(n);
        }
        self.focus.focus(window);
        cx.notify();
    }

    /// Mouse released anywhere in the window.
    fn finish_drag(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        self.field_drag = false;
        if self.shape_edit.is_some() {
            self.edit_mouse_up();
            cx.notify();
            return;
        }
        // Let go of a new line on empty canvas: pick a step to add there, connected.
        if let Drag::Connect { from, port } = self.drag {
            let (ox, oy) = self.canvas_origin.get();
            let (cw, ch) = self.canvas_size.get();
            let (wx, wy) = (f32::from(pos.x), f32::from(pos.y));
            let on_canvas = wx >= ox && wy >= oy && wx <= ox + cw && wy <= oy + ch;
            let (x, y) = self.to_local(pos);
            let moved = self.doc.node(from).map_or(false, |n| {
                let (px0, py0) = out_port_pos(n, port);
                ((x - px0).powi(2) + (y - py0).powi(2)).sqrt() * self.zoom > 24.0
            });
            if on_canvas && moved {
                self.drag = Drag::None;
                self.open_quick(Some((from, port)), Some((x, y)), Some((wx, wy)));
                cx.notify();
                return;
            }
        }
        if let Drag::Select { x, y } = self.drag {
            let (ox, oy) = self.canvas_origin.get();
            let (px0, py0) = self.screen_to_canvas(x - ox, y - oy);
            let (px1, py1) = self.to_local(pos);
            let (x0, x1) = (px0.min(px1), px0.max(px1));
            let (y0, y1) = (py0.min(py1), py0.max(py1));
            let hits: Vec<u64> = self
                .view()
                .nodes
                .iter()
                .filter(|n| n.x < x1 && n.x + NODE_W > x0 && n.y < y1 && n.y + NODE_H > y0)
                .map(|n| n.id)
                .collect();
            for id in hits {
                if self.selected.is_none() {
                    self.selected = Some(id);
                } else if self.selected != Some(id) && !self.extra.contains(&id) {
                    self.extra.push(id);
                }
            }
            if self.selected.is_some() {
                self.side_tab = SideTab::Edit;
                self.sidebar_open = true;
            }
        }
        if self.drag != Drag::None {
            self.drag = Drag::None;
            cx.notify();
        }
    }

    fn node_mouse_up(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Drag::Connect { from, port } = self.drag {
            cx.stop_propagation();
            let accepts = self.doc.node(id).map_or(false, |n| n.kind.has_input());
            if from != id && accepts {
                self.doc.connect(from, port, id);
                self.dirty = true;
            }
            self.drag = Drag::None;
            cx.notify();
        }
    }

    // -----------------------------------------------------------------------
    // Rendering: toolbar, palette, status bar
    // -----------------------------------------------------------------------

    fn render_titlebar(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let idle = self.idle();
        let busy = !idle;
        let is_max = window.is_maximized();
        let name = self.doc.name.clone();
        let dirty = self.dirty;
        let sidebar_open = self.sidebar_open;
        let m = SHORTCUT_KEY;
        // Square icon button for the title bar.
        let icon_btn = |id: &'static str, label: &'static str, enabled: bool| {
            ghost_btn(id, label, enabled).w(px(28.)).justify_center().px_0()
        };
        // A solid button for Run / Stop / Pause.
        let solid = |id: &'static str, label: &'static str, bg: u32, hover: u32, fg: u32| {
            tool_btn_colored(id, label, true, bg, hover)
                .h(px(28.))
                .px_3()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(fg))
        };

        let left = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_0p5()
            .h_full()
            .child(
                div()
                    .h_full()
                    .flex_none()
                    .pl(px(NAME_INSET))
                    .pr_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .window_control_area(WindowControlArea::Drag)
                    .child(logo_mark())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT))
                            .child("MacroFlow"),
                    ),
            )
            .child(
                div()
                    .id("tb-sidebar")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(UI_HOVER)))
                    .child(sidebar_icon(sidebar_open))
                    .tooltip(tip_view(format!("Show / hide panel ({m}B)")))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.sidebar_open = !this.sidebar_open;
                        cx.notify();
                    })),
            )
            .child(divider())
            .child(
                ghost_btn("tb-new", "New", idle)
                    .text_xs()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.new_doc();
                        cx.notify();
                    })),
            )
            .child(
                ghost_btn("tb-open", "Open", idle)
                    .text_xs()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open(cx))),
            )
            .child(
                ghost_btn("tb-save", "Save", idle)
                    .text_xs()
                    .tooltip(tip_view(format!("Save ({m}S). Shift-click to save as…")))
                    .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                        this.save(ev.modifiers().shift, cx);
                        cx.notify();
                    })),
            )
            .child(divider())
            .child(
                icon_btn("tb-undo", "↶", idle && !self.undo.is_empty())
                    .tooltip(tip_view(format!("Undo ({m}Z)")))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.undo();
                        cx.notify();
                    })),
            )
            .child(
                icon_btn("tb-redo", "↷", idle && !self.redo.is_empty())
                    .tooltip(tip_view("Redo"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.redo();
                        cx.notify();
                    })),
            );

        // The macro's name in the middle (click to rename it in the Macro tab).
        let title = div()
            .id("tb-name")
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .px_2p5()
            .h(px(26.))
            .max_w(px(300.))
            .rounded_md()
            .cursor_pointer()
            .text_xs()
            .hover(|s| s.bg(rgb(UI_HOVER)))
            .child(div().min_w(px(0.)).truncate().text_color(rgb(TEXT)).child(name))
            .when(dirty, |d| {
                d.child(div().flex_none().size(px(6.)).rounded_full().bg(rgb(YELLOW)))
            })
            .tooltip(tip_view(if dirty { "Unsaved changes. Click to rename." } else { "Click to rename" }))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.selected = None;
                this.side_tab = SideTab::Macro;
                this.sidebar_open = true;
                cx.notify();
            }));

        // Recording: Record, Record step, and whether to keep mouse moves.
        let moves = self.record_moves;
        let record = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(28.))
            .rounded_md()
            .border_1()
            .border_color(rgb(UI_EDGE))
            .overflow_hidden()
            .child(
                div()
                    .id("tb-rec")
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .px_2p5()
                    .h_full()
                    .text_xs()
                    .text_color(rgb(TEXT))
                    .when(idle, |d| d.cursor_pointer().hover(|s| s.bg(rgb(UI_HOVER))))
                    .when(!idle, |d| d.opacity(0.35))
                    .child(div().size(px(8.)).rounded_full().bg(rgb(RED)))
                    .child("Record")
                    .tooltip(tip_view("Record a whole macro (Esc stops)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.start_record();
                        cx.notify();
                    })),
            )
            .child(div().w(px(1.)).h_full().bg(rgb(UI_EDGE)))
            .child(
                div()
                    .id("tb-step")
                    .flex()
                    .items_center()
                    .px_2p5()
                    .h_full()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .when(idle, |d| d.cursor_pointer().hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT))))
                    .when(!idle, |d| d.opacity(0.35))
                    .child("One step")
                    .tooltip(tip_view("Record a single click or key press as a new step"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.start_record_step();
                        cx.notify();
                    })),
            )
            .child(div().w(px(1.)).h_full().bg(rgb(UI_EDGE)))
            .child(
                div()
                    .id("tb-moves")
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .h_full()
                    .text_xs()
                    .text_color(rgb(if moves { TEXT } else { UI_FAINT }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(UI_HOVER)))
                    .child(switch_sized(moves, 22.0, 13.0))
                    .child("Moves")
                    .tooltip(tip_view(if moves {
                        "Recording keeps mouse movements. Click to record clicks and keys only."
                    } else {
                        "Recording keeps clicks and keys only. Click to also record mouse movements."
                    }))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.record_moves = !this.record_moves;
                        cx.notify();
                    })),
            );

        // Run controls.
        let mut run = div().flex().flex_row().flex_none().items_center().gap_1p5();
        if self.mode == Mode::Running {
            let paused = self.paused.load(Ordering::Relaxed);
            run = run.child(
                if paused {
                    solid("tb-pause", "▶  Resume", GREEN, 0x45d98c, 0x0b1a10)
                } else {
                    solid("tb-pause", "❚❚  Pause", UI_RAISED, UI_HOVER, TEXT).border_1().border_color(rgb(UI_EDGE))
                }
                .tooltip(tip_view("Pause / resume (F9)"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_pause();
                    cx.notify();
                })),
            );
            if paused {
                run = run.child(
                    solid("tb-step-next", "Step", UI_RAISED, UI_HOVER, TEXT)
                        .border_1()
                        .border_color(rgb(UI_EDGE))
                        .tooltip(tip_view("Run the next step (F10)"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.step_once();
                            cx.notify();
                        })),
                );
            }
        }
        if busy {
            run = run.child(
                solid("tb-stop", "■  Stop", RED, 0xf47373, 0xffffff)
                    .tooltip(tip_view("Stop (Esc or F8)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.stop_worker();
                        cx.notify();
                    })),
            );
        } else {
            run = run
                .child(
                    ghost_btn("tb-step-through", "Step", true)
                        .text_xs()
                        .tooltip(tip_view("Run the macro one step at a time (F10)"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.step_once();
                            cx.notify();
                        })),
                )
                .child(
                    solid("tb-run", "▶  Run", GREEN, 0x45d98c, 0x0b1a10)
                        .tooltip(tip_view("Run the macro (F8)"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.start_run();
                            cx.notify();
                        })),
                );
        }

        let drag_space = || div().flex_1().min_w(px(8.)).h_full().window_control_area(WindowControlArea::Drag);
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .w_full()
            .h(px(TITLEBAR_H))
            .bg(rgb(UI_BAR))
            .border_b_1()
            .border_color(rgb(UI_LINE))
            .child(left)
            .child(drag_space())
            .child(title)
            .child(drag_space())
            .child(record)
            .child(div().w(px(6.)))
            .child(divider())
            .child(div().w(px(4.)))
            .child(run)
            .child(div().w(px(if IS_MACOS { 12. } else { 8. })))
            // Windows draws no title bar for us, so add minimise / maximise / close.
            .when(!IS_MACOS, |d| d.child(self.render_window_controls(is_max, cx)))
    }

    /// Minimise / maximise / close for Windows (macOS has its traffic lights).
    /// These are ordinary buttons that MacroFlow handles itself. The system title-bar
    /// button areas didn't respond reliably on Windows.
    fn render_window_controls(&self, is_maximized: bool, cx: &mut Context<Self>) -> Div {
        let cell = |id: &'static str| {
            div()
                .id(id)
                .w(px(46.))
                .h(px(TITLEBAR_H))
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
        };
        let max_icon = if is_maximized {
            // "Restore": two overlapping squares.
            div()
                .relative()
                .size(px(11.))
                .child(
                    div()
                        .absolute()
                        .left(px(2.))
                        .top_0()
                        .size(px(8.))
                        .border_1()
                        .border_color(rgb(TEXT)),
                )
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(2.))
                        .size(px(8.))
                        .bg(rgb(UI_BAR))
                        .border_1()
                        .border_color(rgb(TEXT)),
                )
        } else {
            div().size(px(10.)).border_1().border_color(rgb(TEXT))
        };
        let close_icon = canvas(
            |_, _, _| {},
            |bounds, _, window, _| {
                let x0 = f32::from(bounds.origin.x);
                let y0 = f32::from(bounds.origin.y);
                let w = f32::from(bounds.size.width);
                let h = f32::from(bounds.size.height);
                let color = hsla(TEXT);
                for (a, b) in [((x0, y0), (x0 + w, y0 + h)), ((x0 + w, y0), (x0, y0 + h))] {
                    let mut p = PathBuilder::stroke(px(1.));
                    p.move_to(point(px(a.0), px(a.1)));
                    p.line_to(point(px(b.0), px(b.1)));
                    if let Ok(path) = p.build() {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .size(px(10.));
        div()
            .flex()
            .flex_none()
            .h_full()
            .child(
                cell("win-min")
                    .hover(|s| s.bg(rgb(UI_HOVER)))
                    .child(div().w(px(10.)).h(px(1.)).bg(rgb(TEXT)))
                    .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.minimize_window())),
            )
            .child(
                cell("win-max")
                    .hover(|s| s.bg(rgb(UI_HOVER)))
                    .child(max_icon)
                    .on_click(cx.listener(|_, _: &ClickEvent, window, _| toggle_maximize(window))),
            )
            .child(
                cell("win-close")
                    .hover(|s| s.bg(rgb(0xc42b1c)))
                    .child(close_icon)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.request_close(window, cx)
                    })),
            )
    }

    /// The close button: asks to save first if there are unsaved changes.
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_close(AfterSave::CloseWindow, window, cx) {
            self.finish_close(AfterSave::CloseWindow, window, cx);
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let items: Vec<AnyElement> = match self.side_tab {
            SideTab::Steps => self.steps_tab(cx),
            SideTab::Edit if !self.extra.is_empty() => self.multi_settings(cx),
            SideTab::Edit => match self.selected.and_then(|id| self.doc.node(id)).cloned() {
                Some(n) => self.node_settings(&n, cx),
                None => self.edit_empty(cx),
            },
            SideTab::Macro => self.macro_settings(cx),
            SideTab::Log => self.log_tab(cx),
        };
        let mut content = div()
            .id("sidebar-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .overflow_x_hidden()
            .px_4()
            .pt_1()
            .pb_6()
            .flex()
            .flex_col()
            .gap_2p5();
        for el in items {
            content = content.child(el);
        }
        div()
            .w(px(SIDEBAR_W))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(UI_BAR))
            .border_r_1()
            .border_color(rgb(UI_LINE))
            .child(self.render_tabs(cx))
            .child(content)
    }

    /// The Steps / Edit / Macro switcher at the top of the sidebar.
    fn render_tabs(&self, cx: &mut Context<Self>) -> Div {
        let selected_kind = self.selected.and_then(|id| self.doc.node(id)).map(|n| n.kind.color());
        let tabs = [
            (SideTab::Steps, "Steps"),
            (SideTab::Edit, "Edit"),
            (SideTab::Macro, "Macro"),
            (SideTab::Log, "Log"),
        ];
        let current = self.side_tab;
        div().flex_none().px_4().pt_3().pb_3().child(
            div()
                .flex()
                .gap_0p5()
                .p(px(3.))
                .rounded_lg()
                .bg(rgb(UI_INPUT))
                .border_1()
                .border_color(rgb(UI_LINE))
                .text_xs()
                .children(tabs.into_iter().map(|(tab, label)| {
                    let active = tab == current;
                    div()
                        .id(("side-tab", tab as usize))
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_1p5()
                        .h(px(26.))
                        .rounded_md()
                        .cursor_pointer()
                        .when(active, |d| {
                            d.bg(rgb(UI_RAISED))
                                .border_1()
                                .border_color(rgb(UI_EDGE))
                                .text_color(rgb(TEXT))
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(!active, |d| {
                            d.text_color(rgb(MUTED)).hover(|s| s.text_color(rgb(TEXT)))
                        })
                        .when_some(
                            selected_kind.filter(|_| tab == SideTab::Edit),
                            |d, color| d.child(div().size(px(6.)).rounded_full().bg(rgb(color))),
                        )
                        .child(label)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.commit_edit();
                            this.capturing_key = false;
                            this.side_tab = tab;
                            cx.notify();
                        }))
                })),
        )
    }

    fn steps_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        // Looks like a search box; opens the quick-add finder (⌘K / Ctrl+K).
        out.push(
            div()
                .id("steps-search")
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .h(px(32.))
                .px_2p5()
                .rounded_md()
                .bg(rgb(UI_INPUT))
                .border_1()
                .border_color(rgb(UI_EDGE))
                .cursor_pointer()
                .hover(|s| s.border_color(rgba((ACCENT << 8) | 0x99)))
                .child(div().flex_none().text_color(rgb(UI_FAINT)).child("⌕"))
                .child(div().flex_1().text_xs().text_color(rgb(UI_FAINT)).child("Find a step…"))
                .child(kbd(format!("{SHORTCUT_KEY}K")))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.open_quick(None, None, None);
                    cx.notify();
                }))
                .into_any_element(),
        );
        // Where a new step will go, in one short line.
        let after = match self.selected.and_then(|id| self.doc.node(id)) {
            Some(n) if !n.kind.outputs().is_empty() => Some(n.label()),
            _ => None,
        };
        out.push(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .text_xs()
                .text_color(rgb(UI_FAINT))
                .child(
                    div()
                        .flex_none()
                        .size(px(6.))
                        .rounded_full()
                        .bg(rgb(if after.is_some() { ACCENT } else { UI_EDGE })),
                )
                .child(match after {
                    Some(name) => div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .child(format!("Adds after “{name}”")),
                    None => div().flex_1().child("Select a step to chain new ones after it"),
                })
                .into_any_element(),
        );

        let groups: [(&'static str, usize); 7] = [
            ("Mouse", 0),
            ("Keyboard", 1),
            ("Smart checks", 2),
            ("Variables", 6),
            ("Notify", 5),
            ("Flow & timing", 3),
            ("Recording", 4),
        ];
        let palette: Vec<(usize, NodeKind)> = NodeKind::palette().into_iter().enumerate().collect();
        for (gi, (title, g)) in groups.into_iter().enumerate() {
            out.push(
                if gi == 0 { section_head(title).mt_2() } else { section_title(title) }.into_any_element(),
            );
            let mut list = div().flex().flex_col().gap_0p5().mx(px(-6.));
            for (i, kind) in palette.iter().filter(|(_, k)| step_group(k) == g) {
                let i = *i;
                let kind = kind.clone();
                let color = kind.color();
                list = list.child(
                    div()
                        .id(("pal", i))
                        .group(SharedString::from(format!("pal-{i}")))
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .px_1p5()
                        .h(px(40.))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(UI_HOVER)))
                        .child(
                            div()
                                .size(px(26.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_md()
                                .bg(rgba((color << 8) | 0x1f))
                                .text_size(px(12.))
                                .text_color(rgb(color))
                                .font_weight(FontWeight::BOLD)
                                .child(kind.icon()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .child(div().text_sm().line_height(px(17.)).text_color(rgb(TEXT)).child(kind.title()))
                                .child(
                                    div()
                                        .text_xs()
                                        .line_height(px(15.))
                                        .text_color(rgb(UI_FAINT))
                                        .truncate()
                                        .child(step_blurb(&kind)),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .size(px(22.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_md()
                                .text_color(rgb(TEXT))
                                .bg(rgb(UI_EDGE))
                                .invisible()
                                .group_hover(SharedString::from(format!("pal-{i}")), |s| s.visible())
                                .child("+"),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if this.idle() {
                                this.add_step(kind.clone());
                                cx.notify();
                            }
                        }))
                        .into_any_element(),
                );
            }
            out.push(list.into_any_element());
        }

        // Tips & shortcuts, folded away until you want them.
        let open = self.tips_open;
        out.push(
            div()
                .id("tips-toggle")
                .mt_2()
                .pt_3()
                .border_t_1()
                .border_color(rgb(UI_LINE))
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .cursor_pointer()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(UI_FAINT))
                .hover(|s| s.text_color(rgb(MUTED)))
                .child(div().flex_1().child("TIPS & SHORTCUTS"))
                .child(div().flex_none().child(if open { "▾" } else { "▸" }))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.tips_open = !this.tips_open;
                    cx.notify();
                }))
                .into_any_element(),
        );
        if open {
            for t in [
                "Drag from a step's right-hand dot to another step to connect them. Click the × on a line to remove it.",
                "Drag the empty grid or scroll to move around. Ctrl + scroll zooms.",
                "Right-click a step or the canvas to rename, run from here, group, or add a step there.",
                "Shift-click or Shift-drag to select several steps. Double-click a group to open it; Esc goes back out.",
                "Shift-click “Select area” for a 3 s countdown first.",
            ] {
                out.push(tip(t).into_any_element());
            }
            let m = SHORTCUT_KEY;
            let keys: [(String, &'static str); 10] = [
                ("F8".into(), "Start / stop"),
                ("F9".into(), "Pause / resume"),
                ("F10".into(), "Run one step"),
                ("Esc".into(), "Stop"),
                (format!("{m}K"), "Find a step"),
                (format!("{m}Z"), "Undo"),
                (format!("{m}S"), "Save"),
                (format!("{m}D"), "Duplicate"),
                (format!("{m}G"), "Group"),
                (format!("{m}B"), "Toggle panel"),
            ];
            let mut grid = div().flex().flex_row().flex_wrap().gap_y_1p5().pt_1();
            for (k, what) in keys {
                grid = grid.child(
                    div()
                        .w(px(150.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(div().w(px(36.)).flex().child(kbd(k)))
                        .child(what),
                );
            }
            out.push(grid.into_any_element());
        }
        out
    }

    fn log_tab(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        out.push(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(div().flex_1().child(section_head("Run log")))
                .when(!self.log.is_empty() && self.idle(), |d| {
                    d.child(
                        small_btn("log-play", if self.replay_playing { "❚❚ Pause" } else { "▶ Play back" })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.replay_toggle();
                                cx.notify();
                            })),
                    )
                })
                .child(small_btn("log-copy", "Copy").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    let text: String = this
                        .log
                        .iter()
                        .map(|e| format!("[+{:.2}s] {}\n", e.ms as f32 / 1000.0, e.text))
                        .collect();
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    this.status = "Copied the run log.".into();
                    cx.notify();
                })))
                .child(small_btn("log-clear", "Clear").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.log.clear();
                    this.reset_replay();
                    cx.notify();
                })))
                .into_any_element(),
        );
        if let Some(shot) = self.last_end.as_ref().and_then(|e| e.screenshot.clone()) {
            out.push(help("The screen when it went wrong:").into_any_element());
            out.push(
                div()
                    .h(px(150.))
                    .w_full()
                    .rounded_md()
                    .bg(rgb(UI_INPUT))
                    .border_1()
                    .border_color(rgb(RED))
                    .p_1()
                    .child(img(shot.clone()).size_full().object_fit(ObjectFit::Contain))
                    .into_any_element(),
            );
            out.push(help(&shot.display().to_string()).into_any_element());
        }
        if self.log.is_empty() {
            out.push(
                div()
                    .mt_2()
                    .p_4()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1p5()
                    .rounded_lg()
                    .border_1()
                    .border_dashed()
                    .border_color(rgb(UI_EDGE))
                    .child(div().text_sm().text_color(rgb(TEXT)).font_weight(FontWeight::MEDIUM).child("No runs yet"))
                    .child(
                        div()
                            .text_xs()
                            .line_height(px(17.))
                            .text_color(rgb(UI_FAINT))
                            .text_center()
                            .child("Each run lists the steps it took, with timings and which way each branch went. Click a line to jump to its step."),
                    )
                    .into_any_element(),
            );
            return out;
        }
        if self.log_sel.is_none() {
            out.push(help("Click a line to see what happened at that moment (a green dot shows where it clicked), or press ▶ Play back to watch the whole run again.").into_any_element());
        }
        const SHOW: usize = 600;
        let skip = self.log.len().saturating_sub(SHOW);
        if skip > 0 {
            out.push(help(&format!("…{skip} earlier lines (Copy includes them all)")).into_any_element());
        }
        for (i, e) in self.log.iter().enumerate().skip(skip) {
            let (dot, color, bold) = match e.level {
                LogLevel::Step => (MUTED, TEXT, true),
                LogLevel::Info => (BORDER, MUTED, false),
                LogLevel::Good => (GREEN, GREEN, false),
                LogLevel::Warn => (YELLOW, YELLOW, false),
                LogLevel::Bad => (RED, RED, true),
            };
            let step = e.step.filter(|id| self.doc.node(*id).is_some());
            let picked = self.log_sel == Some(i);
            let mut row = div()
                .id(("log-row", i))
                .when(picked, |d| d.bg(rgba((ACCENT << 8) | 0x40)))
                .flex()
                .items_start()
                .gap_2()
                .px_1()
                .py_0p5()
                .rounded_sm()
                .text_xs()
                .child(
                    div()
                        .w(px(46.))
                        .flex_none()
                        .text_color(rgb(MUTED))
                        .child(format!("+{:.2}s", e.ms as f32 / 1000.0)),
                )
                .child(div().mt(px(5.)).size(px(6.)).flex_none().rounded_full().bg(rgb(dot)))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_color(rgb(color))
                        .when(bold, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .when(e.level != LogLevel::Step, |d| d.pl_2())
                        .child(e.text.clone()),
                );
            if step.is_some() || e.action.is_some() {
                row = row
                    .cursor_pointer()
                    .when(!picked, |d| d.hover(|s| s.bg(rgb(UI_RAISED))))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.replay_playing = false;
                        this.select_log(i, true);
                        cx.notify();
                    }));
            }
            out.push(row.into_any_element());
        }
        out
    }

    fn edit_empty(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![div()
            .mt_4()
            .p_4()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_dashed()
            .border_color(rgb(UI_EDGE))
            .child(div().text_color(rgb(TEXT)).font_weight(FontWeight::SEMIBOLD).child("No step selected"))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .text_center()
                    .child("Click a step on the canvas to edit it, or add a new one from the Steps tab."),
            )
            .child(small_btn("edit-go-steps", "Browse steps").on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.side_tab = SideTab::Steps;
                    cx.notify();
                },
            )))
            .into_any_element()]
    }

    fn render_statusbar(&self, cx: &mut Context<Self>) -> Div {
        // After a run, explain why it ended (until something else is shown).
        let end = self
            .last_end
            .as_ref()
            .filter(|e| self.mode == Mode::Idle && e.message == self.status);
        let (badge, color) = match (self.mode, end.map(|e| e.kind)) {
            (Mode::Idle, Some(EndKind::Error)) => ("ERROR", RED),
            (Mode::Idle, Some(EndKind::Stopped)) => ("STOPPED", RED),
            (Mode::Idle, Some(EndKind::Finished)) => ("ENDED", RED),
            (Mode::Running, _) if self.paused.load(Ordering::Relaxed) && self.step_mode.load(Ordering::Relaxed) => ("STEP", YELLOW),
            (Mode::Running, _) if self.paused.load(Ordering::Relaxed) => ("PAUSED", YELLOW),
            _ => match self.mode {
            Mode::Idle => ("READY", MUTED),
            Mode::Running => ("RUNNING", GREEN),
            Mode::Recording => ("● REC", RED),
            Mode::RecordingStep => ("◎ STEP", RED),
            Mode::Picking(_) => ("PICK", YELLOW),
            Mode::Snipping(_) => ("SNIP", YELLOW),
            Mode::Testing => ("SEARCH", YELLOW),
            Mode::Capturing => ("CAPTURE", YELLOW),
            Mode::Working => ("BUSY", YELLOW),
            },
        };
        let culprit = end.and_then(|e| e.step).filter(|id| self.doc.node(*id).is_some());
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .px_3()
            .h(px(28.))
            .bg(rgb(UI_BAR))
            .border_t_1()
            .border_color(rgb(UI_LINE))
            .text_xs()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .h(px(18.))
                    .rounded_full()
                    .bg(rgba((color << 8) | 0x1f))
                    .text_size(px(10.5))
                    .text_color(rgb(color))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().size(px(6.)).rounded_full().bg(rgb(color)))
                    .child(badge.trim_start_matches(['●', '◎', ' '])),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(if end.is_some() { rgb(0xffb4b4) } else { rgb(TEXT) })
                            .child(self.status.clone()),
                    )
                    .when_some(culprit, |d, id| {
                        d.child(
                            div()
                                .id("status-show-step")
                                .flex_none()
                                .px_1p5()
                                .rounded_sm()
                                .text_color(rgb(TEXT))
                                .bg(rgb(UI_RAISED))
                                .hover(|s| s.bg(rgb(UI_HOVER)))
                                .cursor_pointer()
                                .child("Show step")
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.jump_to_step(id);
                                    cx.notify();
                                })),
                        )
                    })
                    .when(end.map_or(false, |e| e.kind == EndKind::Error || e.unexpected()), |d| {
                        d.child(
                            div()
                                .id("status-open-log")
                                .flex_none()
                                .px_1p5()
                                .rounded_sm()
                                .text_color(rgb(TEXT))
                                .bg(rgb(UI_RAISED))
                                .hover(|s| s.bg(rgb(UI_HOVER)))
                                .cursor_pointer()
                                .child("Log")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.side_tab = SideTab::Log;
                                    this.sidebar_open = true;
                                    cx.notify();
                                })),
                        )
                    })
                    .when(end.is_some(), |d| {
                        d.child(
                            div()
                                .id("status-clear-end")
                                .flex_none()
                                .px_1()
                                .rounded_sm()
                                .text_color(rgb(MUTED))
                                .hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT)))
                                .cursor_pointer()
                                .child("✕")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.last_end = None;
                                    this.status = "Ready.".into();
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .child({
                        let g = self.view();
                        let place = if self.view.is_empty() {
                            String::new()
                        } else {
                            format!("in “{}” · ", self.group_name(*self.view.last().unwrap()))
                        };
                        format!("{place}{} steps · {} links", g.step_count(), g.edges.len())
                    }),
            )
    }

    // -----------------------------------------------------------------------
    // Rendering: flowchart
    // -----------------------------------------------------------------------

    fn render_graph(&self, cx: &mut Context<Self>) -> Div {
        let (pan_x, pan_y) = self.pan;
        let z = self.zoom;
        let origin = self.canvas_origin.clone();
        let size_cell = self.canvas_size.clone();
        let (ox, oy) = origin.get();
        // Canvas position -> position on the canvas element.
        let sx = move |x: f32| x * z + pan_x;
        let sy = move |y: f32| y * z + pan_y;

        // Connection geometry, in on-screen coordinates.
        let mut lines: Vec<(f32, f32, f32, f32, Hsla, bool)> = Vec::new();
        let mut handles: Vec<(f32, f32, u64, usize)> = Vec::new();
        let view = self.view();
        let inside = !self.view.is_empty();
        for e in &view.edges {
            let (Some(a), Some(b)) = (self.doc.node(e.from), self.doc.node(e.to)) else {
                continue;
            };
            let (x0, y0) = out_port_pos(a, e.port);
            let (x1, y1) = in_port_pos(b);
            let (x0, y0, x1, y1) = (sx(x0), sy(y0), sx(x1), sy(y1));
            let active = self.is_active(a);
            lines.push((x0, y0, x1, y1, hsla(a.kind.color()), active));
            let (mx, my) = bezier_mid(x0, y0, x1, y1, z);
            handles.push((mx, my, e.from, e.port));
        }
        let pending = match self.drag {
            Drag::Connect { from, port } => self.doc.node(from).map(|n| {
                let (x0, y0) = out_port_pos(n, port);
                (sx(x0), sy(y0), self.mouse.0 - ox, self.mouse.1 - oy)
            }),
            _ => None,
        };

        let painter = canvas(
            move |bounds, _, _| {
                origin.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y)));
                size_cell.set((f32::from(bounds.size.width), f32::from(bounds.size.height)));
            },
            move |bounds, _, window, _| {
                let bx = f32::from(bounds.origin.x);
                let by = f32::from(bounds.origin.y);
                let bw = f32::from(bounds.size.width);
                let bh = f32::from(bounds.size.height);
                let grid = hsla(GRID);
                // 24 px squares at 100%; doubled when zoomed far out so it doesn't turn solid.
                let mut step = 24.0 * z;
                while step < 12.0 {
                    step *= 2.0;
                }
                let mut gx = pan_x.rem_euclid(step);
                while gx < bw {
                    window.paint_quad(fill(
                        Bounds {
                            origin: point(px(bx + gx), px(by)),
                            size: gpui::size(px(1.), px(bh)),
                        },
                        grid,
                    ));
                    gx += step;
                }
                let mut gy = pan_y.rem_euclid(step);
                while gy < bh {
                    window.paint_quad(fill(
                        Bounds {
                            origin: point(px(bx), px(by + gy)),
                            size: gpui::size(px(bw), px(1.)),
                        },
                        grid,
                    ));
                    gy += step;
                }
                for (x0, y0, x1, y1, color, active) in lines {
                    let width = if active { 3.5 } else { 2.0 };
                    stroke_curve(window, bx + x0, by + y0, bx + x1, by + y1, (width * z).max(1.0), color, false, z);
                }
                if let Some((x0, y0, x1, y1)) = pending {
                    stroke_curve(window, bx + x0, by + y0, bx + x1, by + y1, (2.0 * z).max(1.0), hsla(ACCENT), true, z);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let mut graph = div()
            .relative()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(rgb(CANVAS_BG))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.menu = None;
                    // Control-click is right-click on a Mac.
                    if IS_MACOS && ev.modifiers.control {
                        this.open_canvas_menu(ev.position);
                        cx.notify();
                        return;
                    }
                    this.commit_edit();
                    this.capturing_key = false;
                    let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
                    this.mouse = (x, y);
                    if ev.modifiers.shift {
                        // Shift-drag draws a selection box (adds to what's selected).
                        this.drag = Drag::Select { x, y };
                    } else {
                        this.clear_selection();
                        this.drag = Drag::Pan { x, y };
                    }
                    this.focus.focus(window);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.open_canvas_menu(ev.position);
                    this.focus.focus(window);
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                this.menu = None;
                let (dx, dy) = match ev.delta {
                    ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                    ScrollDelta::Lines(l) => (l.x * 24.0, l.y * 24.0),
                };
                if ev.modifiers.control || ev.modifiers.platform {
                    // Ctrl + scroll (⌘ + scroll on a Mac also works) zooms around the mouse.
                    let (ox, oy) = this.canvas_origin.get();
                    let mx = f32::from(ev.position.x) - ox;
                    let my = f32::from(ev.position.y) - oy;
                    let factor = (dy.clamp(-120.0, 120.0) * 0.004).exp();
                    this.zoom_around(this.zoom * factor, mx, my);
                } else {
                    this.pan.0 += dx;
                    this.pan.1 += dy;
                }
                cx.notify();
            }))
            .child(painter);

        // Inside a group: tint the canvas and frame it in the group colour, so it's
        // obvious you're not at the top level. (The grid itself is unchanged.)
        if inside {
            graph = graph.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(rgba((GROUP_COLOR << 8) | 0x0b))
                    .border_2()
                    .border_color(rgba((GROUP_COLOR << 8) | 0x99)),
            );
        }

        for n in &view.nodes {
            // Groups look like a little stack of cards.
            if n.kind.is_group() {
                for off in [8.0, 4.0] {
                    graph = graph.child(
                        div()
                            .when(n.disabled, |d| d.opacity(0.5))
                            .absolute()
                            .left(px(sx(n.x + off)))
                            .top(px(sy(n.y + off)))
                            .w(px(NODE_W * z))
                            .h(px(NODE_H * z))
                            .rounded(px(8. * z))
                            .bg(rgb(NODE_BG))
                            .border_1()
                            .border_color(rgba((GROUP_COLOR << 8) | 0x66)),
                    );
                }
            }
            graph = graph.child(self.render_node(n, pan_x, pan_y, z, cx));
        }

        for (mx, my, from, port) in handles {
            let hs = (16.0 * z).max(10.0);
            graph = graph.child(
                div()
                    .absolute()
                    .left(px(mx - hs / 2.))
                    .top(px(my - hs / 2.))
                    .size(px(hs))
                    .rounded_full()
                    .bg(rgb(PANEL2))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px((12.0 * z).max(8.0)))
                    .text_color(rgb(MUTED))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(RED)).text_color(rgb(0xffffff)))
                    .child("×")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.doc.disconnect(from, port);
                            this.dirty = true;
                            cx.notify();
                        }),
                    ),
            );
        }

        if let Drag::Select { x, y } = self.drag {
            let (x0, y0) = (x - ox, y - oy);
            let (x1, y1) = (self.mouse.0 - ox, self.mouse.1 - oy);
            graph = graph.child(
                div()
                    .absolute()
                    .left(px(x0.min(x1)))
                    .top(px(y0.min(y1)))
                    .w(px((x1 - x0).abs()))
                    .h(px((y1 - y0).abs()))
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(ACCENT))
                    .bg(rgba((ACCENT << 8) | 0x1f)),
            );
        }

        if inside {
            graph = graph.child(self.render_breadcrumb(cx));
            if self.view.iter().any(|g| self.doc.node(*g).map_or(false, |n| n.disabled)) {
                graph = graph.child(
                    div()
                        .absolute()
                        .top(px(56.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py_1()
                                .rounded_lg()
                                .bg(rgba(0x161922f0))
                                .border_1()
                                .border_color(rgb(BORDER))
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child("⊘ This group is disabled: runs skip everything in it."),
                        ),
                );
            }
        }
        if let Some(bar) = self.render_selection_bar(cx) {
            graph = graph.child(bar);
        }
        graph = graph.child(self.render_zoom_controls(cx));

        if !inside && view.nodes.len() <= 1 && !self.show_start {
            graph = graph.child(
                div()
                    .absolute()
                    .bottom(px(16.))
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .h(px(32.))
                            .rounded_lg()
                            .bg(rgba((UI_RAISED << 8) | 0xf2))
                            .border_1()
                            .border_color(rgb(UI_EDGE))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("Add steps from the panel, press")
                            .child(kbd(format!("{SHORTCUT_KEY}K")))
                            .child("to search, or hit Record."),
                    ),
            );
        }
        if !self.live_vars.is_empty() && self.show_vars && !self.show_start {
            graph = graph.child(self.render_vars_panel(cx));
        }
        if self.log_sel.is_some() && !self.show_start {
            graph = graph.child(self.render_replay(cx));
        }
        if self.show_start && !inside {
            graph = graph.child(self.render_start(cx));
        }
        graph
    }

    /// The run-log playback card, bottom right of the canvas: the screen at that
    /// moment with the action drawn on, what happened, and the playback buttons.
    fn render_replay(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let i = self.log_sel.unwrap_or(0);
        let total = self.log.len();
        let entry = self.log.get(i);
        let picture = self.replay_picture();
        let (content, from_earlier): (AnyElement, bool) = match &picture {
            Some((p, j)) if p.exists() => (
                img(p.clone()).size_full().object_fit(ObjectFit::Contain).into_any_element(),
                *j != i,
            ),
            Some(_) => (
                div().text_color(rgb(MUTED)).child("Picture still saving…").into_any_element(),
                false,
            ),
            None => (
                div()
                    .text_color(rgb(MUTED))
                    .text_center()
                    .px_4()
                    .child("No picture for this line. Clicks, moves, drags, scrolls and keys have one.")
                    .into_any_element(),
                false,
            ),
        };
        let step_label = entry
            .and_then(|e| e.step)
            .and_then(|id| self.doc.node(id))
            .map(|n| n.label())
            .unwrap_or_default();
        let caption = entry.map(|e| e.text.clone()).unwrap_or_default();
        let when = entry.map(|e| format!("+{:.2}s", e.ms as f32 / 1000.0)).unwrap_or_default();
        let speed_label: &'static str = match self.replay_speed {
            1 => "1×",
            2 => "2×",
            _ => "4×",
        };
        let ctl = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(px(26.))
                .min_w(px(30.))
                .px_2()
                .rounded_md()
                .bg(rgb(UI_RAISED))
                .text_color(rgb(TEXT))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(UI_HOVER)))
                .child(label)
        };
        div()
            .id("replay-card")
            .occlude()
            .absolute()
            .bottom(px(56.))
            .right(px(16.))
            .w(px(420.))
            .flex()
            .flex_col()
            .rounded_lg()
            .bg(rgba(0x161922f5))
            .border_1()
            .border_color(rgb(if self.replay_playing { GREEN } else { BORDER }))
            .shadow_lg()
            .text_xs()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .pl_2()
                    .pr_1()
                    .h(px(28.))
                    .border_b_1()
                    .border_color(rgb(UI_EDGE))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Playback"))
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(MUTED))
                            .child(format!("· line {} of {total} · {when}", i + 1)),
                    )
                    .child(
                        div()
                            .id("replay-close")
                            .px_1p5()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(MUTED))
                            .hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT)))
                            .child("×")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.reset_replay();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .h(px(240.))
                    .m_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(rgb(UI_INPUT))
                    .border_1()
                    .border_color(rgb(UI_EDGE))
                    .p_1()
                    .child(content),
            )
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(div().text_color(rgb(TEXT)).child(caption))
                    .child(div().text_color(rgb(MUTED)).child(match (step_label.is_empty(), from_earlier) {
                        (true, _) => String::new(),
                        (false, false) => format!("Step: {step_label}"),
                        (false, true) => format!("Step: {step_label} · picture from its last action"),
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .pb_2()
                    .child(ctl("replay-first", "⏮").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.replay_playing = false;
                        this.select_log(0, true);
                        cx.notify();
                    })))
                    .child(ctl("replay-prev", "◀").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.replay_step(false);
                        cx.notify();
                    })))
                    .child(
                        ctl("replay-play", if self.replay_playing { "❚❚" } else { "▶" })
                            .min_w(px(44.))
                            .bg(rgb(if self.replay_playing { BTN } else { GREEN }))
                            .text_color(rgb(if self.replay_playing { TEXT } else { 0x0b1a12 }))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.replay_toggle();
                                cx.notify();
                            })),
                    )
                    .child(ctl("replay-next", "▶|").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.replay_step(true);
                        cx.notify();
                    })))
                    .child(ctl("replay-speed", speed_label).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.replay_speed = match this.replay_speed {
                            1 => 2,
                            2 => 4,
                            _ => 1,
                        };
                        cx.notify();
                    })))
                    .child(div().flex_1())
                    .child(
                        chip("replay-on-screen", "Show on screen", self.log_on_screen).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.log_on_screen = !this.log_on_screen;
                                cx.notify();
                            },
                        )),
                    ),
            )
    }

    /// Every variable and its value, bottom left of the canvas, during and after a run.
    fn render_vars_panel(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let live = self.mode == Mode::Running;
        let mut list = div().id("vars-list").flex().flex_col().p_1().gap_0p5().overflow_y_scroll();
        // Your own variables first, then the built-in ones.
        let mut vars: Vec<&(String, String)> = self.live_vars.iter().collect();
        vars.sort_by_key(|(n, _)| (BUILTIN_VARS.contains(&n.as_str()), n.clone()));
        for (name, value) in vars {
            let shown: String = value.replace('\n', " ⏎ ").chars().take(40).collect();
            let shown = if shown.is_empty() { "(empty)".to_string() } else { shown };
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .min_h(px(20.))
                    .child(var_pill(name, true, false, 1.0))
                    .child(div().text_color(rgb(MUTED)).child("="))
                    .child(div().flex_1().min_w(px(0.)).truncate().text_color(rgb(TEXT)).child(shown)),
            );
        }
        div()
            .id("vars-panel")
            .occlude()
            .absolute()
            .bottom(px(16.))
            .left(px(16.))
            .w(px(270.))
            .max_h(px(300.))
            .flex()
            .flex_col()
            .rounded_lg()
            .bg(rgba((UI_RAISED << 8) | 0xf2))
            .border_1()
            .border_color(rgb(if live { ACCENT } else { BORDER }))
            .shadow_md()
            .text_xs()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .pl_2()
                    .pr_1()
                    .h(px(28.))
                    .border_b_1()
                    .border_color(rgb(UI_EDGE))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Variables"))
                    .child(div().flex_1().text_color(rgb(MUTED)).child(if live { "· live" } else { "· last run" }))
                    .child(
                        div()
                            .id("vars-close")
                            .px_1p5()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(MUTED))
                            .hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT)))
                            .child("×")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.show_vars = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(list)
    }

    /// Templates, recent macros and unsaved work, over the canvas when MacroFlow opens.
    fn render_start(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let tile = |id: SharedString, title: String, text: String| {
            div()
                .id(id)
                .w(px(190.))
                .h(px(96.))
                .px_3()
                .py_2p5()
                .flex()
                .flex_col()
                .gap_1()
                .overflow_hidden()
                .rounded_lg()
                .bg(rgb(UI_INPUT))
                .border_1()
                .border_color(rgb(UI_EDGE))
                .cursor_pointer()
                .hover(|s| s.border_color(rgba((ACCENT << 8) | 0xcc)).bg(rgb(UI_HOVER)))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT))
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .line_height(px(16.))
                        .text_color(rgb(UI_FAINT))
                        .line_clamp(3)
                        .text_ellipsis()
                        .child(text.clone()),
                )
                .tooltip(tip_view(text))
        };
        let mut tiles = div().flex().flex_row().flex_wrap().gap_2p5();
        tiles = tiles.child(
            tile("start-blank".into(), "Blank macro".into(), "Just a Start step. Add steps from the left.".into())
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.show_start = false;
                    cx.notify();
                })),
        );
        for (i, (name, text, _)) in templates().into_iter().enumerate() {
            tiles = tiles.child(
                tile(SharedString::from(format!("start-template-{i}")), name.to_string(), text.to_string()).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.use_template(i);
                        cx.notify();
                    }),
                ),
            );
        }
        tiles = tiles.child(
            tile("start-open".into(), "Open a file…".into(), "A macro you saved before (.json).".into()).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.open(cx);
                    cx.notify();
                }),
            ),
        );

        let mut card = div()
            .w(px(640.))
            .px_5()
            .pt_5()
            .pb_4()
            .flex()
            .flex_col()
            .gap_3()
            .rounded_xl()
            .bg(rgb(UI_RAISED))
            .border_1()
            .border_color(rgb(UI_EDGE))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(logo_mark())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child("Welcome to MacroFlow"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(UI_FAINT))
                                    .child("Start from a template, or pick up where you left off."),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("start-close")
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(MUTED))
                            .hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT)))
                            .child("×")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.show_start = false;
                                cx.notify();
                            })),
                    ),
            );

        if let Some(a) = &self.autosave_found {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let mins = now.saturating_sub(a.saved_ms) / 60_000;
            let when = match mins {
                0 => "just now".to_string(),
                1..=59 => format!("{mins} min ago"),
                60..=1439 => format!("{} h ago", mins / 60),
                _ => format!("{} days ago", mins / 1440),
            };
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(rgba((VAR_WARN << 8) | 0x14))
                    .border_1()
                    .border_color(rgba((VAR_WARN << 8) | 0x77))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("Unsaved work: “{}”", a.doc.name)),
                            )
                            .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                                "MacroFlow closed before it was saved (kept {when}).",
                            ))),
                    )
                    .child(small_btn("start-restore", "Restore").on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.restore_autosave();
                            cx.notify();
                        },
                    )))
                    .child(small_btn("start-discard", "Discard").on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.autosave_found = None;
                            if !this.dirty {
                                clear_autosave();
                            }
                            cx.notify();
                        },
                    ))),
            );
        }

        card = card.child(section_head("Start from").mt_1()).child(tiles);
        card = card.child(section_title("Recent"));
        if self.recent.is_empty() {
            card = card.child(help("Macros you open or save show up here."));
        } else {
            let mut list = div().flex().flex_col();
            for (i, p) in self.recent.iter().take(6).enumerate() {
                let name = p
                    .file_stem()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string());
                let folder = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
                // Show the home folder as ~ so the path fits.
                let folder = match std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
                    Ok(home) if !home.is_empty() && folder.starts_with(&home) => format!("~{}", &folder[home.len()..]),
                    _ => folder,
                };
                let path = p.clone();
                list = list.child(
                    div()
                        .id(("start-recent", i))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .mx(px(-8.))
                        .px_2()
                        .h(px(30.))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(UI_HOVER)))
                        .child(div().flex_none().size(px(6.)).rounded_sm().bg(rgb(ACCENT)))
                        .child(div().flex_none().max_w(px(260.)).truncate().text_sm().text_color(rgb(TEXT)).child(name))
                        .child(div().flex_1().min_w(px(0.)).truncate().text_xs().text_color(rgb(UI_FAINT)).child(folder))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.load(path.clone());
                            cx.notify();
                        })),
                );
            }
            card = card.child(list);
        }
        card = card.child(
            div()
                .mt_1()
                .pt_3()
                .border_t_1()
                .border_color(rgb(UI_LINE))
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .text_xs()
                .text_color(rgb(UI_FAINT))
                .child(kbd(format!("{SHORTCUT_KEY}K")))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .child("finds and adds a step by name. Drag from a dot into empty space to add a connected one."),
                ),
        );

        div()
            .id("start-screen")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba((CANVAS_BG << 8) | 0xcc))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(card)
    }

    /// The quick-add box: type to find a step, Enter adds it.
    fn render_quick(&self, vw: f32, vh: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(q) = &self.quick else { return div().id("quick-add") };
        let matches = self.quick_matches();
        let palette = NodeKind::palette();
        let w = 400.0;
        let shown = matches.len().min(8);
        let h = 44.0 + shown.max(1) as f32 * 46.0 + 8.0 + 30.0;
        let (x, y) = match q.screen {
            Some((sx, sy)) => ((sx + 8.0).min(vw - w - 8.0).max(8.0), (sy - 20.0).min(vh - h - 8.0).max(8.0)),
            None => (((vw - w) / 2.0).max(8.0), 72.0),
        };
        let caret = div().flex_none().w(px(1.5)).h(px(16.)).bg(rgb(TEXT));
        let input = if q.query.is_empty() {
            div()
                .flex()
                .items_center()
                .child(caret)
                .child(div().pl_1().text_color(rgb(MUTED)).child("Type to find a step…"))
        } else {
            div().flex().items_center().child(q.query.clone()).child(caret)
        };
        let mut list = div().flex().flex_col().p_1().gap_0p5();
        if matches.is_empty() {
            list = list.child(
                div()
                    .h(px(44.))
                    .px_2()
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(rgb(UI_FAINT))
                    .child(format!("No steps match “{}”", q.query)),
            );
        }
        let sel = q.sel.min(matches.len().saturating_sub(1));
        for (row, &pi) in matches.iter().take(8).enumerate() {
            let k = &palette[pi];
            let on = row == sel;
            list = list.child(
                div()
                    .id(("quick-item", pi))
                    .h(px(44.))
                    .px_2()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2p5()
                    .rounded_md()
                    .cursor_pointer()
                    .when(on, |d| d.bg(rgba((ACCENT << 8) | 0x33)))
                    .when(!on, |d| d.hover(|s| s.bg(rgb(UI_HOVER))))
                    .child(
                        div()
                            .size(px(26.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(rgba((k.color() << 8) | 0x1f))
                            .text_size(px(12.))
                            .text_color(rgb(k.color()))
                            .font_weight(FontWeight::BOLD)
                            .child(k.icon()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(div().line_height(px(17.)).text_color(rgb(TEXT)).child(k.title()))
                            .child(
                                div()
                                    .text_xs()
                                    .line_height(px(15.))
                                    .text_color(rgb(UI_FAINT))
                                    .truncate()
                                    .child(step_blurb(k)),
                            ),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.quick_pick(pi);
                            cx.notify();
                        }),
                    ),
            );
        }
        div()
            .id("quick-add")
            .occlude()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .flex()
            .flex_col()
            .rounded_xl()
            .overflow_hidden()
            .bg(rgb(UI_RAISED))
            .border_1()
            .border_color(rgb(UI_EDGE))
            .shadow_lg()
            .text_sm()
            .text_color(rgb(TEXT))
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                if this.quick.is_some() {
                    this.quick = None;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .h(px(44.))
                    .px_3()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(UI_LINE))
                    .child(
                        div()
                            .flex_none()
                            .px_1p5()
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .rounded_sm()
                            .bg(rgba((ACCENT << 8) | 0x2e))
                            .text_xs()
                            .text_color(rgb(0xc9bdff))
                            .child(if q.link.is_some() { "Add & connect" } else { "Add step" }),
                    )
                    .child(input),
            )
            .child(list)
            .child(
                div()
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(UI_LINE))
                    .text_xs()
                    .text_color(rgb(UI_FAINT))
                    .child("↑ ↓ choose · Enter adds · Esc closes"),
            )
    }

    /// "These steps would go wrong – run anyway?"
    fn render_run_check(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(c) = &self.run_check else { return div().id("run-check") };
        let n = c.problems.len();
        let mut list = div().flex().flex_col().gap_1();
        for (i, p) in c.problems.iter().take(6).enumerate() {
            let label = self.doc.node(p.step).map(|n| n.label()).unwrap_or_else(|| "A step".into());
            let step = p.step;
            list = list.child(
                div()
                    .id(("run-check-item", i))
                    .flex()
                    .flex_row()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgba((RED << 8) | 0x14))
                    .border_1()
                    .border_color(rgba((RED << 8) | 0x55))
                    .cursor_pointer()
                    .hover(|s| s.border_color(rgb(RED)))
                    .child(div().flex_none().text_color(rgb(RED)).child("⚠"))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                            .child(div().text_xs().text_color(rgb(MUTED)).child(p.text.clone())),
                    )
                    .child(div().flex_none().text_xs().text_color(rgb(MUTED)).child("Show"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.run_check = None;
                        this.jump_to_step(step);
                        cx.notify();
                    })),
            );
        }
        if n > 6 {
            list = list.child(help(&format!("…and {} more.", n - 6)));
        }
        div()
            .id("run-check")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x0000_0088))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .w(px(460.))
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .rounded_xl()
                    .bg(rgb(UI_RAISED))
                    .border_1()
                    .border_color(rgb(UI_EDGE))
                    .shadow_lg()
                    .text_sm()
                    .text_color(rgb(TEXT))
                    .child(div().text_base().font_weight(FontWeight::BOLD).child(format!(
                        "{n} step{} would go wrong",
                        if n == 1 { "" } else { "s" }
                    )))
                    .child(help("Fix them first (click one to go to it), or run anyway."))
                    .child(list)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .justify_end()
                            .child(small_btn("run-check-cancel", "Cancel  (Esc)").on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.run_check = None;
                                    cx.notify();
                                },
                            )))
                            .child(
                                small_btn("run-check-go", "Run anyway  (Enter)")
                                    .bg(rgb(ACCENT))
                                    .text_color(rgb(0xffffff))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        if let Some(c) = this.run_check.take() {
                                            this.run_now(c.from, c.stepping);
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }

    /// − 100% + Fit, bottom right of the canvas.
    fn render_zoom_controls(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("zoom-controls")
            .occlude()
            .absolute()
            .bottom(px(16.))
            .right(px(16.))
            .flex()
            .items_center()
            .gap_0p5()
            .p(px(3.))
            .rounded_lg()
            .bg(rgba((UI_RAISED << 8) | 0xf2))
            .border_1()
            .border_color(rgb(UI_EDGE))
            .shadow_md()
            .text_xs()
            .child(zoom_btn("zoom-out", "−").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.zoom_by(1.0 / 1.2);
                cx.notify();
            })))
            .child(
                zoom_btn("zoom-pct", format!("{:.0}%", self.zoom * 100.0))
                    .w(px(46.))
                    .justify_center()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.zoom_reset();
                        cx.notify();
                    })),
            )
            .child(zoom_btn("zoom-in", "+").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.zoom_by(1.2);
                cx.notify();
            })))
            .child(div().w(px(1.)).h(px(16.)).mx_0p5().bg(rgb(UI_EDGE)))
            .child(zoom_btn("zoom-fit", "Fit").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.zoom_fit();
                cx.notify();
            })))
    }

    /// A small floating bar at the bottom of the canvas with Group / Ungroup for
    /// whatever is selected, so grouping existing steps is always one click away.
    fn render_selection_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.idle() {
            return None;
        }
        let ids = self.selection();
        let first = *ids.first()?;
        let mut actions: Vec<Stateful<Div>> = Vec::new();
        let label;
        if ids.len() > 1 {
            label = format!("{} steps selected", ids.len());
            actions.push(bar_btn("sel-group", "▤ Group", true).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.group_selection();
                    cx.notify();
                },
            )));
            actions.push(bar_btn("sel-delete", "Delete", false).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.delete_selected();
                    cx.notify();
                },
            )));
        } else {
            let node = self.doc.node(first)?;
            if node.kind.is_group() {
                label = self.group_name(first);
                actions.push(bar_btn("sel-open", "Open", true).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.enter_group(first);
                        cx.notify();
                    },
                )));
                actions.push(bar_btn("sel-ungroup", "Ungroup", false).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.ungroup_selected();
                        cx.notify();
                    },
                )));
            } else {
                let run = if node.kind.is_fixed() { 0 } else { self.doc.straight_run(first).len() };
                if run < 2 {
                    return None;
                }
                label = format!("{} + next {} step{}", node.kind.title(), run - 1, if run == 2 { "" } else { "s" });
                actions.push(bar_btn("sel-group", "▤ Group these", true).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.group_selection();
                        cx.notify();
                    },
                )));
            }
        }
        actions.push(bar_btn("sel-clear", "✕", false).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.clear_selection();
                cx.notify();
            },
        )));
        let mut bar = div()
            .id("selection-bar")
            .occlude()
            .flex()
            .items_center()
            .gap_1()
            .pl_3()
            .pr_1()
            .py_1()
            .rounded_lg()
            .bg(rgba((UI_RAISED << 8) | 0xf2))
            .border_1()
            .border_color(rgb(UI_EDGE))
            .shadow_lg()
            .child(div().pr_2().text_xs().text_color(rgb(MUTED)).child(label));
        for a in actions {
            bar = bar.child(a);
        }
        Some(
            div()
                .absolute()
                .bottom(px(16.))
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .child(bar),
        )
    }

    /// Did this step end the last run (for a group: did one of its steps)?
    fn is_culprit(&self, n: &Node) -> bool {
        match (self.last_end.as_ref().and_then(|e| e.step), &n.kind) {
            (Some(s), _) if s == n.id => true,
            (Some(s), NodeKind::Group { graph, .. }) => graph.contains(s),
            _ => false,
        }
    }

    /// Open whichever group a step is in, select it and scroll it into view.
    fn jump_to_step(&mut self, id: u64) {
        let Some(path) = self.doc.root.path_to(id) else {
            self.status = "That step no longer exists.".into();
            return;
        };
        self.commit_edit();
        self.view = path;
        self.clear_selection();
        self.selected = Some(id);
        self.side_tab = SideTab::Edit;
        self.sidebar_open = true;
        if let Some(n) = self.doc.node(id) {
            self.pan = (240.0 - n.x * self.zoom, 160.0 - n.y * self.zoom);
        }
    }

    /// Is this step running right now (for a group: is one of its steps running)?
    fn is_active(&self, n: &Node) -> bool {
        let shown = self.active_node.or(if self.idle() { self.replay_node } else { None });
        match (shown, &n.kind) {
            (Some(a), _) if a == n.id => true,
            (Some(a), NodeKind::Group { graph, .. }) => graph.contains(a),
            _ => false,
        }
    }

    /// "Macro › Group 1 › Group 2" across the top of the canvas when you're inside a group.
    fn render_breadcrumb(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut crumbs: Vec<(usize, String)> = vec![(0, self.doc.name.clone())];
        for (i, gid) in self.view.iter().enumerate() {
            crumbs.push((i + 1, self.group_name(*gid)));
        }
        let last = crumbs.len() - 1;
        let mut bar = div()
            .id("breadcrumb")
            .occlude()
            .absolute()
            .top(px(12.))
            .left(px(12.))
            .flex()
            .items_center()
            .gap_1()
            .pl_1()
            .pr_2()
            .py_1()
            .rounded_lg()
            .bg(rgba((UI_RAISED << 8) | 0xf0))
            .border_1()
            .border_color(rgba((GROUP_COLOR << 8) | 0xaa))
            .shadow_md()
            .child(
                small_btn("crumb-back", "← Back")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            let depth = this.view.len().saturating_sub(1);
                            this.leave_group_to(depth);
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(GROUP_COLOR))
                    .child("INSIDE GROUP"),
            );
        for (depth, label) in crumbs {
            if depth > 0 {
                bar = bar.child(div().text_color(rgb(MUTED)).child("›"));
            }
            let is_last = depth == last;
            bar = bar.child(
                div()
                    .id(("crumb", depth))
                    .px_1p5()
                    .rounded_md()
                    .max_w(px(180.))
                    .truncate()
                    .when(is_last, |d| d.text_color(rgb(TEXT)).font_weight(FontWeight::SEMIBOLD))
                    .when(!is_last, |d| {
                        d.text_color(rgb(MUTED))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT)))
                    })
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.leave_group_to(depth);
                            cx.notify();
                        }),
                    ),
            );
        }
        bar
    }

    fn render_node(&self, n: &Node, pan_x: f32, pan_y: f32, z: f32, cx: &mut Context<Self>) -> Div {
        // Every size is scaled by the zoom (at 100% these are the original sizes).
        let s = |v: f32| px(v * z);
        let line = |v: f32| px((v * z).max(1.0));
        let id = n.id;
        let selected = self.selected == Some(id) || self.extra.contains(&id);
        let active = self.is_active(n);
        let color = n.kind.color();
        let culprit = !active && self.is_culprit(n);
        // The exact output that led nowhere (only on the step itself, not its group).
        let culprit_port = self
            .last_end
            .as_ref()
            .filter(|e| culprit && e.step == Some(id))
            .and_then(|e| e.port);
        let border = if active {
            rgb(YELLOW)
        } else if culprit {
            rgb(RED)
        } else if selected {
            rgb(ACCENT)
        } else {
            rgb(BORDER)
        };
        let outputs = n.kind.outputs();
        let subtitle = if n.note.is_empty() {
            String::new()
        } else {
            n.note.clone()
        };
        // Your own name for the step, otherwise its type.
        let heading = if n.name.trim().is_empty() {
            n.kind.title().to_string()
        } else {
            n.name.trim().to_string()
        };
        let text_pr = if outputs.len() > 1 { 64. } else { 12. };
        // Disabled itself, or inside a disabled group: runs skip it.
        let skipped = self.doc.is_skipped(id);

        let mut el = div()
            .absolute()
            .left(px(n.x * z + pan_x))
            .top(px(n.y * z + pan_y))
            .w(s(NODE_W))
            .h(s(NODE_H))
            .flex()
            .flex_col()
            .bg(rgb(NODE_BG))
            .rounded(s(8.))
            .border(line(2.))
            .border_color(border)
            .shadow_md()
            .text_size(s(14.))
            .cursor_move()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.node_mouse_down(id, ev, window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_step_menu(id, ev.position);
                    this.focus.focus(window);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseUpEvent, _, cx| this.node_mouse_up(id, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(s(8.))
                    .px(s(8.))
                    .h(s(26.))
                    .flex_none()
                    .rounded_t(s(6.))
                    .bg(if skipped { rgba((MUTED << 8) | 0x1c) } else { rgba((color << 8) | 0x2e) })
                    .child(
                        div()
                            .flex_none()
                            .text_color(rgb(if skipped { MUTED } else { color }))
                            .child(if n.disabled { "⊘" } else { n.kind.icon() }),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(if skipped { MUTED } else { TEXT }))
                            .when(n.disabled, |d| d.line_through())
                            .child(heading),
                    )
                    .child(div().flex_1())
                    .when(n.disabled, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .px(s(5.))
                                .rounded(s(4.))
                                .bg(rgb(BTN))
                                .text_size(s(11.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(MUTED))
                                .child("⊘ OFF"),
                        )
                    })
                    .when(n.delay_ms > 0 && !n.disabled, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(s(12.))
                                .text_color(rgb(MUTED))
                                .child(format!("+{} ms", n.delay_ms)),
                        )
                    }),
            )
            .child(
                div()
                    .px(s(8.))
                    .pt(s(4.))
                    .pr(s(text_pr))
                    .text_size(s(12.))
                    .text_color(rgb(TEXT))
                    .truncate()
                    .child(self.summary_view(&n.kind, z)),
            )
            .child(
                div()
                    .px(s(8.))
                    .pr(s(text_pr))
                    .text_size(s(12.))
                    .italic()
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(subtitle),
            );

        if n.kind.has_input() {
            el = el.child(
                div()
                    .absolute()
                    .left(s(-8.))
                    .top(s(NODE_H / 2. - 8.))
                    .size(s(14.))
                    .rounded_full()
                    .bg(rgb(PORT_BG))
                    .border(line(2.))
                    .border_color(rgb(color)),
            );
        }

        for (i, label) in outputs.iter().enumerate() {
            let y = port_offset(outputs.len(), i);
            let connected = self.doc.next(id, i).is_some();
            let dead_end = culprit_port == Some(i);
            el = el.child(
                div()
                    .absolute()
                    .right(s(-8.))
                    .top(s(y - 8.))
                    .size(s(14.))
                    .rounded_full()
                    .bg(if dead_end {
                        rgb(RED)
                    } else if connected {
                        rgb(color)
                    } else {
                        rgb(PORT_BG)
                    })
                    .border(line(2.))
                    .border_color(if dead_end { rgb(RED) } else { rgb(color) })
                    .cursor_crosshair()
                    .hover(|s| s.border_color(rgb(0xffffff)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.menu = None;
                            this.commit_edit();
                            this.drag = Drag::Connect { from: id, port: i };
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.menu = None;
                            this.doc.disconnect(id, i);
                            this.dirty = true;
                            cx.notify();
                        }),
                    ),
            );
            if outputs.len() > 1 {
                el = el.child(
                    div()
                        .absolute()
                        .right(s(12.))
                        .top(s(y - 8.))
                        .text_size(s(12.))
                        .text_color(rgb(MUTED))
                        .child(*label),
                );
            }
        }
        if n.breakpoint {
            el = el.child(
                div()
                    .absolute()
                    .left(s(-6.))
                    .top(s(-6.))
                    .size(s(13.))
                    .rounded_full()
                    .bg(rgb(RED))
                    .border(line(2.))
                    .border_color(rgb(NODE_BG))
                    .shadow_sm(),
            );
        }
        // ⚠ in the corner when something about the step looks wrong.
        if let Some(p) = self.problems.get(&id) {
            let total = p.serious + p.minor + p.inside;
            if total > 0 {
                let col = if p.serious > 0 || p.inside_serious { RED } else { VAR_WARN };
                el = el.child(
                    div()
                        .absolute()
                        .right(s(-7.))
                        .top(s(-8.))
                        .h(s(17.))
                        .px(s(4.))
                        .flex()
                        .items_center()
                        .gap(s(2.))
                        .rounded(s(5.))
                        .bg(rgb(col))
                        .border(line(2.))
                        .border_color(rgb(NODE_BG))
                        .text_size(s(10.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(if col == RED { 0xffffff } else { 0x1a1400 }))
                        .child("⚠")
                        .when(total > 1, |d| d.child(total.to_string())),
                );
            }
        }
        // What the step did on the last run, under the card.
        if let Some(r) = self.results.get(&id) {
            let col = if r.ok { GREEN } else { VAR_WARN };
            let text = if r.count > 1 { format!("{}  ×{}", r.text, r.count) } else { r.text.clone() };
            el = el.child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(s(NODE_H + 5.))
                    .max_w(s(NODE_W))
                    .h(s(18.))
                    .px(s(6.))
                    .flex()
                    .items_center()
                    .rounded(s(5.))
                    .bg(rgba((col << 8) | 0x1f))
                    .border(line(1.))
                    .border_color(rgba((col << 8) | 0x66))
                    .text_size(s(11.))
                    .text_color(rgb(col))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(text),
            );
        }
        if culprit {
            let (tag, why) = match self.last_end.as_ref().map(|e| e.kind) {
                Some(EndKind::Error) => ("✕", "went wrong here"),
                Some(EndKind::Stopped) => ("■", "stopped here"),
                _ if n.kind.is_group() => ("■", "ended inside"),
                _ if culprit_port.is_some() => ("■", "ended here – nothing connected"),
                _ => ("■", "ended here"),
            };
            el = el.child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(s(-24.))
                    .flex()
                    .items_center()
                    .gap(s(4.))
                    .px(s(6.))
                    .h(s(18.))
                    .rounded(s(6.))
                    .bg(rgb(RED))
                    .text_size(s(12.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0xffffff))
                    .whitespace_nowrap()
                    .child(tag)
                    .child(why),
            );
        }
        if skipped && !active {
            el = el.opacity(0.5);
        }
        el
    }

    // -----------------------------------------------------------------------
    // Rendering: inspector
    // -----------------------------------------------------------------------

    fn macro_settings(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![
            section_head("Macro").into_any_element(),
            self.text_field("Name", Field::MacroName, cx).into_any_element(),
            self.num_field("Start delay (ms)", Field::StartDelay, 250.0, cx).into_any_element(),
            self.num_field("Gap between steps (ms)", Field::StepDelay, 10.0, cx).into_any_element(),
            self.num_field("Speed (%)", Field::Speed, 10.0, cx).into_any_element(),
            self.num_field("Repeat whole macro", Field::Repeat, 1.0, cx).into_any_element(),
            help("Repeat 0 = keep going until you press Esc. Speed 200% halves every delay.").into_any_element(),
            section_title("Change every delay at once").into_any_element(),
            div()
                .flex()
                .gap_1()
                .child(
                    small_btn("all-half", "× 0.5").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.doc.scale_delays(0.5);
                        this.dirty = true;
                        this.status = "Halved every delay.".into();
                        cx.notify();
                    })),
                )
                .child(
                    small_btn("all-075", "× 0.75").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.doc.scale_delays(0.75);
                        this.dirty = true;
                        this.status = "Shortened every delay by 25%.".into();
                        cx.notify();
                    })),
                )
                .child(
                    small_btn("all-15", "× 1.5").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.doc.scale_delays(1.5);
                        this.dirty = true;
                        this.status = "Lengthened every delay by 50%.".into();
                        cx.notify();
                    })),
                )
                .child(
                    small_btn("all-double", "× 2").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.doc.scale_delays(2.0);
                        this.dirty = true;
                        this.status = "Doubled every delay.".into();
                        cx.notify();
                    })),
                )
                .into_any_element(),
            self.num_field("Fixed delay (ms)", Field::BulkDelay, 10.0, cx).into_any_element(),
            small_btn("all-set", "Set every recorded & step delay to this")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.commit_edit();
                    let v = this.bulk_delay;
                    this.doc.set_all_delays(v);
                    this.dirty = true;
                    this.status = format!("Every recorded delay and step delay is now {v} ms.");
                    cx.notify();
                }))
                .into_any_element(),
            help("Scaling affects waits, recordings and step delays. “Set every…” leaves Wait steps alone.").into_any_element(),
        ];

        // Variables.
        out.push(section_title("Variables").into_any_element());
        {
            let set = self.doc.variables_set();
            let never_set: Vec<String> = self
                .doc
                .variables_used()
                .into_iter()
                .filter(|n| !self.is_known_var(n))
                .collect();
            let pills = |names: Vec<String>, known: bool| {
                names.into_iter().fold(
                    div().flex().flex_row().flex_wrap().gap_y_1().text_xs(),
                    |row, n| row.child(var_pill(&n, known, false, 1.0)),
                )
            };
            if set.is_empty() {
                out.push(help("No steps set a variable yet. Add a Set variable step, or a Read text step that saves what it reads.").into_any_element());
            } else {
                out.push(pills(set.into_iter().collect(), true).into_any_element());
            }
            out.push(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_y_1()
                    .text_xs()
                    .child(div().text_color(rgb(MUTED)).mr_1().child("Always there:"))
                    .child(pills(BUILTIN_VARS.iter().map(|v| v.to_string()).collect(), true))
                    .into_any_element(),
            );
            if !never_set.is_empty() {
                out.push(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_y_1()
                        .text_xs()
                        .child(div().text_color(rgb(VAR_WARN)).mr_1().child("Used but never set:"))
                        .child(pills(never_set, false))
                        .into_any_element(),
                );
            }
            out.push(help("Use a variable in text as {name}: it turns into a tag like the ones above once you type the closing }. Amber tags are variables no step sets, which is usually a typo.").into_any_element());
        }

        // Scaling to the screen size.
        out.push(section_title("Screen size").into_any_element());
        let scaled = self.doc.reference.is_some();
        out.push(
            toggle("scale-positions", "Scale positions to the screen size", scaled)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_scaling(!scaled);
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(
            help(&match self.doc.reference {
                Some(r) if self.doc.anchor.is_some() => format!(
                    "On: positions are saved as a place within the followed window (made at {} × {}), so they still land in the same spot if the window is bigger, smaller or on another monitor.",
                    r.w, r.h
                ),
                Some(r) => format!(
                    "On: positions are saved as a place on the screen (made on a {} × {} screen), not exact pixels, so they land in the same spot on a bigger or smaller monitor or another computer. Find image pictures are resized to match.",
                    r.w, r.h
                ),
                None => "Off: positions are exact screen pixels, so they only line up on the same screen size. Turn this on to make the macro work on other monitors and computers (nothing moves on this screen when you do).".into(),
            })
            .into_any_element(),
        );

        // Following a window.
        out.push(section_title("Positions follow a window").into_any_element());
        match &self.doc.anchor {
            Some(a) => {
                let name = if a.title.is_empty() { a.app.clone() } else { format!("{} – {}", a.app, a.title) };
                out.push(
                    div()
                        .px_2()
                        .py_1p5()
                        .rounded_md()
                        .bg(rgba((GREEN << 8) | 0x1a))
                        .border_1()
                        .border_color(rgba((GREEN << 8) | 0x55))
                        .text_xs()
                        .child(format!("Following “{name}”"))
                        .into_any_element(),
                );
                out.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(small_btn("anchor-change", "Change window").on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.picking_anchor = true;
                                this.start_list_windows();
                                cx.notify();
                            },
                        )))
                        .child(small_btn("anchor-off", "Use screen positions").on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.set_anchor(None);
                                cx.notify();
                            },
                        )))
                        .into_any_element(),
                );
            }
            None => {
                out.push(help("Off: clicks, areas and colours are exact screen positions, so moving the window you automate breaks them.").into_any_element());
                out.push(
                    small_btn("anchor-pick", "Follow a window…")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.picking_anchor = true;
                            this.start_list_windows();
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
        }
        if self.picking_anchor && !self.windows.is_empty() {
            out.push(help("Click the window the macro works in. Its positions will move with it.").into_any_element());
            for (i, w) in self.windows.iter().take(25).enumerate() {
                let label = format!(
                    "{} — {}",
                    w.app,
                    if w.title.is_empty() { "(no title)" } else { w.title.as_str() }
                );
                let pick = w.clone();
                out.push(
                    div()
                        .id(("anchor-win", i))
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .text_xs()
                        .truncate()
                        .bg(rgb(UI_RAISED))
                        .hover(|s| s.bg(rgb(UI_HOVER)))
                        .cursor_pointer()
                        .child(label)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.set_anchor(Some(pick.clone()));
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
        }
        out.push(help("When followed, every saved position is relative to that window: if it moves (or you use a different monitor), clicks, read areas, search areas and colour checks move with it. The window is found by app name (and title if it matches).").into_any_element());

        // Mouse movement.
        out.push(section_title("Mouse movement").into_any_element());
        out.push(
            toggle("human-mouse", "Human-like mouse movement", self.doc.human_mouse)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.doc.human_mouse = !this.doc.human_mouse;
                    this.dirty = true;
                    cx.notify();
                }))
                .into_any_element(),
        );
        if self.doc.human_mouse {
            out.push(self.num_field("Mouse speed (%)", Field::MouseSpeed, 10.0, cx).into_any_element());
        }
        out.push(help(if self.doc.human_mouse {
            "On: the mouse glides to each spot along a slightly curved path, starting slow, speeding up and slowing down at the end. Every move is a bit different. Long moves take longer. 200% is twice as fast. Recordings still play back exactly as recorded."
        } else {
            "Off: the mouse jumps straight to each spot. Turn this on for curved, natural-looking moves."
        })
        .into_any_element());
        out.push(
            toggle("show-clicks", "Show a green dot where it clicks", self.doc.show_clicks)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.doc.show_clicks = !this.doc.show_clicks;
                    this.dirty = true;
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(help("While it runs, a green dot appears where it's about to click (a ring for a plain mouse move, and where a drag lets go), with a ripple when it clicks. Clicks go straight through it, and it's hidden from MacroFlow's own screenshots, so Find image, Read text and Pixel colour never see it.").into_any_element());

        // Alerts and hotkeys.
        out.push(section_title("When it stops").into_any_element());
        out.push(
            toggle("alert-on-stop", "Red pop-up if it stops unexpectedly", self.doc.alert_on_stop)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.doc.alert_on_stop = !this.doc.alert_on_stop;
                    this.dirty = true;
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(help("Shows when a step goes wrong (including an internal crash) or the macro runs into a path with nothing connected.").into_any_element());
        out.push(
            toggle("alert-on-finish", "Green pop-up when it finishes", self.doc.alert_on_finish)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.doc.alert_on_finish = !this.doc.alert_on_finish;
                    this.dirty = true;
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(help("Shows when it reaches an End step or has done every repeat. Pressing Stop / Esc never shows a pop-up.").into_any_element());
        out.push(
            toggle("alert-on-breakpoint", "Pop-up when it pauses at a breakpoint", self.doc.alert_on_breakpoint)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.doc.alert_on_breakpoint = !this.doc.alert_on_breakpoint;
                    this.dirty = true;
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(
            toggle("failsafe", "Failsafe: stop when the mouse hits a screen corner", self.doc.failsafe)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.doc.failsafe = !this.doc.failsafe;
                    this.dirty = true;
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(help("Throw the mouse into any outer corner of the screen to stop the macro instantly, even if keys aren't getting through. Corners where two monitors meet don't count, and the macro moving the mouse there itself doesn't either.").into_any_element());
        out.push(section_title("Hotkeys (work in any app)").into_any_element());
        for (k, what) in [
            ("F8", "Start, or stop while it runs"),
            ("F9", "Pause / resume"),
            ("F10", "Run the next step"),
            ("Esc", "Stop"),
        ] {
            out.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(div().w(px(40.)).flex().child(kbd(k)))
                    .child(what)
                    .into_any_element(),
            );
        }
        out
    }

    fn node_settings(&self, n: &Node, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let id = n.id;
        let mut out: Vec<AnyElement> = Vec::new();
        out.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().size(px(10.)).rounded_full().bg(rgb(n.kind.color())))
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .child(n.kind.title()),
                )
                .child(div().flex_1())
                .when(!n.kind.is_fixed(), |d| {
                    d.child(small_btn("n-dup", "Duplicate").on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.duplicate_selected();
                            cx.notify();
                        },
                    )))
                    .child(
                        small_btn("n-del", "Delete")
                            .text_color(rgb(RED))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.delete_selected();
                                cx.notify();
                            })),
                    )
                })
                .into_any_element(),
        );
        if let Some(p) = self.problems.get(&id) {
            let mut items: Vec<(bool, String)> = p.texts.clone();
            if p.inside > 0 {
                items.push((
                    p.inside_serious,
                    format!(
                        "{} problem{} with steps inside this group. Open it to see.",
                        p.inside,
                        if p.inside == 1 { "" } else { "s" }
                    ),
                ));
            }
            for (serious, text) in items {
                let col = if serious { RED } else { VAR_WARN };
                out.push(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(rgba((col << 8) | 0x18))
                        .border_1()
                        .border_color(rgba((col << 8) | 0x66))
                        .text_xs()
                        .child(div().flex_none().text_color(rgb(col)).child("⚠"))
                        .child(div().flex_1().min_w(px(0.)).line_height(px(17.)).text_color(rgb(TEXT)).child(text))
                        .into_any_element(),
                );
            }
        }
        if !matches!(n.kind, NodeKind::GroupIn | NodeKind::GroupOut | NodeKind::Group { .. }) {
            out.push(self.text_field("Name (shown on the step)", Field::StepName, cx).into_any_element());
        }
        // Options every step has go last, under their own heading.
        let mut general: Vec<AnyElement> = Vec::new();
        let groupable = !n.kind.is_fixed() && !matches!(n.kind, NodeKind::End) && self.doc.straight_run(id).len() >= 2;
        if groupable {
            let run = self.doc.straight_run(id).len();
            general.push(
                small_btn("n-group-run", "Group the straight line from here")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.group_selection();
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            general.push(
                help(&format!(
                    "Turns this step and the {} after it into one Group step. Shift-click steps to pick exactly which ones.",
                    run - 1
                ))
                .into_any_element(),
            );
        }
        if !matches!(n.kind, NodeKind::GroupIn | NodeKind::GroupOut) {
            general.push(self.text_field("Note", Field::Note, cx).into_any_element());
            general.push(self.num_field("Delay before (ms)", Field::Delay, 50.0, cx).into_any_element());
            general.push(
                toggle("n-breakpoint", "Breakpoint: pause before this step", n.breakpoint)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle_breakpoint(id);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        if !n.kind.is_fixed() {
            let label = if n.kind.is_group() {
                "Disabled: runs skip this whole group"
            } else {
                "Disabled: runs skip this step"
            };
            general.push(
                toggle("n-disabled", label, n.disabled)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        // Act on this step only, even if others are selected too.
                        this.clear_selection();
                        this.selected = Some(id);
                        this.toggle_disabled();
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            if n.disabled {
                let multi = n.kind.outputs().len() > 1;
                general.push(help(if multi {
                    "Runs skip it and carry on along its first output that's connected (normally the first one)."
                } else {
                    "Runs skip it and carry on with whatever its output is connected to."
                })
                .into_any_element());
            }
        }

        match &n.kind {
            NodeKind::Start => {
                out.push(help("Every run begins here. Connect its dot to your first step.").into_any_element());
            }
            NodeKind::End => {
                out.push(help("The macro stops when it reaches this step (or any step with nothing connected). Inside a group, an End step stops the whole macro too.").into_any_element());
            }
            NodeKind::GroupIn => {
                out.push(help("The group starts here. Connect it to the group's first step.").into_any_element());
            }
            NodeKind::GroupOut => {
                out.push(help("When the macro reaches this step it leaves the group and carries on from the Group step's output. A step inside the group with nothing connected does the same.").into_any_element());
            }
            NodeKind::Group { graph, .. } => {
                out.extend(self.group_settings(id, graph, cx));
            }
            NodeKind::Click { target, button, .. } => {
                out.push(self.target_row(id, *target, cx).into_any_element());
                out.extend(self.position_fields(id, *target, cx));
                let b = *button;
                out.push(
                    labeled(
                        "Button",
                        div()
                            .flex()
                            .gap_1()
                            .children([MouseBtn::Left, MouseBtn::Right, MouseBtn::Middle].into_iter().map(|opt| {
                                chip(("btn", opt as usize), opt.label(), b == opt).on_click(cx.listener(
                                    move |this, _: &ClickEvent, _, cx| {
                                        this.update_node(id, |k| {
                                            if let NodeKind::Click { button, .. } = k {
                                                *button = opt;
                                            }
                                        });
                                        cx.notify();
                                    },
                                ))
                            })),
                    )
                    .into_any_element(),
                );
                out.push(self.num_field("Clicks", Field::Clicks, 1.0, cx).into_any_element());
            }
            NodeKind::MoveMouse { target, .. } => {
                out.push(self.target_row(id, *target, cx).into_any_element());
                out.extend(self.position_fields(id, *target, cx));
            }
            NodeKind::Drag { button, taps, tap_in_area, .. } => {
                let b = *button;
                let tapping = *taps > 0;
                let in_area = tapping && *tap_in_area;
                if !in_area {
                out.push(section_title("Start").into_any_element());
                out.push(self.num_field("X", Field::DragFromX, 1.0, cx).into_any_element());
                out.push(self.num_field("Y", Field::DragFromY, 1.0, cx).into_any_element());
                out.push(
                    small_btn("drag-pick-from", "Pick start on screen (3 s)")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.start_drag_pick(id, false);
                            cx.notify();
                        }))
                        .into_any_element(),
                );
                out.push(section_title("End").into_any_element());
                out.push(self.num_field("X", Field::DragToX, 1.0, cx).into_any_element());
                out.push(self.num_field("Y", Field::DragToY, 1.0, cx).into_any_element());
                out.push(
                    small_btn("drag-pick-to", "Pick end on screen (3 s)")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.start_drag_pick(id, true);
                            cx.notify();
                        }))
                        .into_any_element(),
                );
                out.push(div().h(px(4.)).into_any_element());
                }
                out.push(
                    labeled(
                        "Button",
                        div().flex().gap_1().children([MouseBtn::Left, MouseBtn::Right, MouseBtn::Middle].into_iter().map(|opt| {
                            chip(("drag-btn", opt as usize), opt.label(), b == opt).on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.update_node(id, |k| {
                                        if let NodeKind::Drag { button, .. } = k {
                                            *button = opt;
                                        }
                                    });
                                    cx.notify();
                                },
                            ))
                        })),
                    )
                    .into_any_element(),
                );
                out.push(self.num_field("Tap instead of dragging (taps, 0 = drag)", Field::DragTaps, 1.0, cx).into_any_element());
                if tapping {
                    out.push(
                        labeled(
                            "Spread the taps",
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .child(chip("tap-line", "Along the line", !in_area).on_click(cx.listener(
                                    move |this, _: &ClickEvent, _, cx| {
                                        this.update_node(id, |k| {
                                            if let NodeKind::Drag { tap_in_area, .. } = k {
                                                *tap_in_area = false;
                                            }
                                        });
                                        cx.notify();
                                    },
                                )))
                                .child(chip("tap-area", "Round a shape", in_area).on_click(cx.listener(
                                    move |this, ev: &ClickEvent, window, cx| {
                                        let mut has_area = false;
                                        this.update_node(id, |k| {
                                            if let NodeKind::Drag { tap_in_area, tap_area, .. } = k {
                                                *tap_in_area = true;
                                                has_area = tap_area.is_some();
                                            }
                                        });
                                        // No area yet: go straight to dragging a box for it.
                                        if !has_area {
                                            this.start_area(AreaPurpose::ClickArea(id), ev.modifiers().shift, window, cx);
                                        }
                                        cx.notify();
                                    },
                                ))),
                        )
                        .into_any_element(),
                    );
                }
                if in_area {
                    out.extend(self.area_block(id, cx));
                }
                out.push(self.num_field(if tapping { "All the taps take (ms)" } else { "Move takes (ms)" }, Field::DragMs, 50.0, cx).into_any_element());
                if !tapping {
                    out.push(self.num_field("Hold still before moving (ms)", Field::DragHold, 50.0, cx).into_any_element());
                    out.push(help("After pressing, it keeps the mouse still this long before it starts moving (80 ms is a quick press). Raise it to 300–800 ms for apps that only pick something up after you've held it still – otherwise they pan the view instead.").into_any_element());
                }
                out.push(help(if in_area {
                    "Taps that many times round the shape below instead of dragging, spread over the time above."
                } else if tapping {
                    "Taps that many times, evenly spaced from the start to the end, spread over the time above (a few pixels off each time). Use this where dragging moves the view instead, e.g. placing troops along the edge of a base in a game. “Round a shape” spreads them round an area instead."
                } else {
                    "Presses the button at the start, holds still, moves smoothly to the end (curved if human-like mouse movement is on in the Macro tab), then lets go. Good for sliders, drag-and-drop and selecting text. The macro's Speed setting changes how long the move takes (not the hold). Set taps above 0 to tap instead of dragging."
                }).into_any_element());
            }
            NodeKind::KeyPress { key, mods } => {
                let capturing = self.capturing_key;
                out.push(
                    labeled(
                        "Key",
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(UI_INPUT))
                            .border_1()
                            .border_color(if capturing { rgb(ACCENT) } else { rgb(UI_EDGE) })
                            .child(if capturing {
                                "Press a key combination…".to_string()
                            } else {
                                keys::combo_label(key, mods)
                            }),
                    )
                    .into_any_element(),
                );
                out.push(
                    small_btn("cap-key", if capturing { "Cancel" } else { "Capture key from keyboard" })
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.commit_edit();
                            this.capturing_key = !this.capturing_key;
                            if this.capturing_key {
                                this.status = "Press the key combination you want (Esc cancels).".into();
                            }
                            this.focus.focus(window);
                            cx.notify();
                        }))
                        .into_any_element(),
                );
                let md = *mods;
                #[cfg(target_os = "macos")]
                let toggles: [(&'static str, &'static str, bool); 4] = [
                    ("m-ctrl", "⌃ Ctrl", md.ctrl),
                    ("m-alt", "⌥ Opt", md.alt),
                    ("m-shift", "⇧ Shift", md.shift),
                    ("m-cmd", "⌘ Cmd", md.cmd),
                ];
                #[cfg(not(target_os = "macos"))]
                let toggles: [(&'static str, &'static str, bool); 4] = [
                    ("m-ctrl", "Ctrl", md.ctrl),
                    ("m-alt", "Alt", md.alt),
                    ("m-shift", "Shift", md.shift),
                    ("m-cmd", "⊞ Win", md.cmd),
                ];
                out.push(
                    labeled(
                        "Hold",
                        div().flex().gap_1().children(toggles.into_iter().map(|(cid, label, on)| {
                            chip(cid, label, on).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.update_node(id, |k| {
                                    if let NodeKind::KeyPress { mods, .. } = k {
                                        match cid {
                                            "m-ctrl" => mods.ctrl = !mods.ctrl,
                                            "m-alt" => mods.alt = !mods.alt,
                                            "m-shift" => mods.shift = !mods.shift,
                                            _ => mods.cmd = !mods.cmd,
                                        }
                                    }
                                });
                                cx.notify();
                            }))
                        })),
                    )
                    .into_any_element(),
                );
            }
            NodeKind::TypeText { press_enter, .. } => {
                out.push(self.text_field("Text to type", Field::Text, cx).into_any_element());
                out.push(help("Put {text} anywhere to type what the last Read text step read.").into_any_element());
                out.push(
                    toggle("type-enter", "Press Return afterwards", *press_enter)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.update_node(id, |k| {
                                if let NodeKind::TypeText { press_enter, .. } = k {
                                    *press_enter = !*press_enter;
                                }
                            });
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
            NodeKind::Wait { .. } => {
                out.push(self.num_field("Wait (ms)", Field::WaitMs, 100.0, cx).into_any_element());
                out.push(self.num_field("Random extra (ms)", Field::Jitter, 50.0, cx).into_any_element());
                out.push(help("Random extra adds 0…N ms each time, so the macro looks less robotic.").into_any_element());
            }
            NodeKind::Scroll { horizontal, .. } => {
                out.push(self.num_field("Wheel clicks", Field::ScrollAmt, 1.0, cx).into_any_element());
                out.push(self.num_field("Strength (× per click)", Field::ScrollStrength, 1.0, cx).into_any_element());
                out.push(
                    toggle("scroll-h", "Sideways", *horizontal)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.update_node(id, |k| {
                                if let NodeKind::Scroll { horizontal, .. } = k {
                                    *horizontal = !*horizontal;
                                }
                            });
                            cx.notify();
                        }))
                        .into_any_element(),
                );
                out.push(help("Positive scrolls down/right, negative scrolls up/left. Each click is sent separately, like turning the wheel by hand. If it still scrolls less than your mouse does, raise the strength.").into_any_element());
                out.push(
                    small_btn("scroll-test", "Test (scrolls in 3 s)")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.start_scroll_test(id);
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
            NodeKind::Recording { events, .. } => {
                out.extend(self.recording_settings(id, events, cx));
            }
            NodeKind::FindImage {
                template,
                click_when_found,
                ..
            } => {
                out.extend(self.find_image_settings(id, template.clone(), *click_when_found, cx));
            }
            NodeKind::WindowCheck {
                scope,
                field,
                mode,
                case_sensitive,
                bring_to_front,
                ..
            } => {
                out.extend(self.window_check_settings(
                    id,
                    *scope,
                    *field,
                    *mode,
                    *case_sensitive,
                    *bring_to_front,
                    cx,
                ));
            }
            NodeKind::ReadText {
                compare,
                case_sensitive,
                preview,
                w,
                h,
                numbers_only,
                ..
            } => {
                out.extend(self.read_text_settings(
                    id,
                    *compare,
                    *case_sensitive,
                    *numbers_only,
                    preview.clone(),
                    *w,
                    *h,
                    cx,
                ));
            }
            NodeKind::Loop { .. } => {
                out.push(self.num_field("Times (0 = forever)", Field::LoopCount, 1.0, cx).into_any_element());
                out.push(help("Connect “repeat” to the steps you want repeated, and connect the last of them back into this Loop. When the count runs out it follows “done”.").into_any_element());
            }
            NodeKind::SetVar { .. } => {
                out.extend(self.set_var_settings(cx));
            }
            NodeKind::IfCompare { op, .. } => {
                out.extend(self.if_settings(id, *op, cx));
            }
            NodeKind::PixelColor { color, .. } => {
                out.extend(self.pixel_settings(id, *color, cx));
            }
            NodeKind::Notify {
                style,
                position,
                wait_for_click,
                sound,
                ..
            } => {
                out.extend(self.notify_settings(id, *style, *position, *wait_for_click, *sound, cx));
            }
        }
        if !general.is_empty() {
            out.push(section_title("Step options").into_any_element());
            out.extend(general);
        }
        out
    }

    fn target_row(&self, id: u64, target: Target, cx: &mut Context<Self>) -> Div {
        let opts = [
            (Target::Fixed, "Fixed spot"),
            (Target::Area, "Random in area"),
            (Target::FoundImage, "Found image"),
            (Target::Current, "Where it is"),
        ];
        let has_area = matches!(
            self.doc.node(id).map(|n| &n.kind),
            Some(NodeKind::Click { area: Some(_), .. } | NodeKind::MoveMouse { area: Some(_), .. })
        );
        labeled(
            "Where",
            div().flex().flex_wrap().gap_1().children(opts.into_iter().map(|(t, label)| {
                chip(("tgt", t as usize), label, t == target).on_click(cx.listener(
                    move |this, ev: &ClickEvent, window, cx| {
                        this.update_node(id, |k| match k {
                            NodeKind::Click { target, .. } | NodeKind::MoveMouse { target, .. } => *target = t,
                            _ => {}
                        });
                        // No area yet: go straight to dragging a box for it.
                        if t == Target::Area && !has_area {
                            this.start_area(AreaPurpose::ClickArea(id), ev.modifiers().shift, window, cx);
                        }
                        cx.notify();
                    },
                ))
            })),
        )
    }

    /// The random-area settings shared by Click / Move mouse ("Random in area") and
    /// Drag ("taps round a shape"): select / edit / show buttons, shape, turn,
    /// hollow band and a preview.
    fn area_block(&self, id: u64, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(n) = self.doc.node(id) else { return Vec::new() };
        let Some((area, shape)) = n.kind.area_parts() else { return Vec::new() };
        let shape = shape.clone();
        let taps = match &n.kind {
            NodeKind::Drag { taps, .. } => *taps,
            _ => 0,
        };
        let what = if taps > 0 { "Spreads the taps" } else { "Picks a different random spot" };
        let mut v = vec![match area {
            Some(r) if shape.is_plain() => {
                help(&format!("{what} inside {} × {} at ({}, {}).", r.w, r.h, r.x, r.y)).into_any_element()
            }
            Some(r) => help(&format!(
                "{what} inside a {} in the {} × {} box at ({}, {}).",
                shape.describe(),
                r.w,
                r.h,
                r.x,
                r.y
            ))
            .into_any_element(),
            None => help(if taps > 0 {
                "No area yet – drag a box round where to tap, or draw the shape with “Edit on screen”."
            } else {
                "No area yet – drag a box around where it should click, or draw the shape with “Edit on screen”."
            })
            .into_any_element(),
        }];
        let select_label: &'static str = match (area.is_some(), taps > 0) {
            (false, true) => "Select tap area",
            (true, true) => "Select tap area again",
            (false, false) => "Select click area",
            (true, false) => "Select click area again",
        };
        v.push(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(small_btn("click-area", select_label).on_click(cx.listener(
                    move |this, ev: &ClickEvent, window, cx| {
                        this.start_area(AreaPurpose::ClickArea(id), ev.modifiers().shift, window, cx);
                        cx.notify();
                    },
                )))
                .child(small_btn("area-edit", "Edit on screen…").on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.start_shape_editor(id, window);
                        cx.notify();
                    },
                )))
                .when(area.is_some(), |d| {
                    d.child(small_btn("area-tour", "Show on screen").on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.show_area_on_screen(id);
                            cx.notify();
                        },
                    )))
                })
                .into_any_element(),
        );
        let Some(r) = area else { return v };
        v.push(div().h(px(4.)).into_any_element());
        v.push(
            labeled(
                "Shape",
                div().flex().flex_wrap().gap_1().children(ShapeKind::ALL.into_iter().map(|k| {
                    chip(("area-shape", k as usize), k.label(), shape.kind == k).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            let mut new_outline = false;
                            this.update_node(id, |n| {
                                if let Some((_, sh)) = n.area_parts_mut() {
                                    sh.kind = k;
                                    if k == ShapeKind::Polygon && sh.points.len() < 3 {
                                        sh.points = default_polygon_points();
                                        new_outline = true;
                                    }
                                }
                            });
                            // A new freeform outline: straight to drawing it on screen.
                            if new_outline {
                                this.start_shape_editor(id, window);
                            }
                            cx.notify();
                        },
                    ))
                })),
            )
            .into_any_element(),
        );
        if shape.kind == ShapeKind::Polygon {
            v.push(
                help(&format!(
                    "{} corners. Use “Edit on screen…” to move them, add more (click outside the outline) or draw a new outline.",
                    shape.points.len()
                ))
                .into_any_element(),
            );
        }
        v.push(self.num_field("Turn (degrees, clockwise)", Field::AreaRotation, 5.0, cx).into_any_element());
        v.push(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children([0i32, 30, 45, 60, 90].into_iter().map(|deg| {
                    chip(("area-turn", deg as usize), turn_label(deg), shape.rotation.round() as i32 == deg).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.update_node(id, |n| {
                                if let Some((_, sh)) = n.area_parts_mut() {
                                    sh.rotation = deg as f32;
                                }
                            });
                            cx.notify();
                        }),
                    )
                }))
                .into_any_element(),
        );
        v.push(
            self.num_field("Hollow: only the edge band (px, 0 = whole shape)", Field::AreaHollow, 5.0, cx)
                .into_any_element(),
        );
        v.push(area_preview(r, &shape, taps).into_any_element());
        v.push(
            help(if taps > 0 {
                if shape.hollow_px > 0 {
                    "Green dots: where the taps go. With a hollow shape they go round the edge band in order, from a random starting point, either way round – e.g. troops round the outside of a base. Every run is a little different."
                } else {
                    "Green dots: where the taps go – spread evenly over the whole shape, a little different every run. Make it hollow to tap round its edge instead."
                }
            } else {
                "Green dots: where clicks can land (a sample). The faint box is the area you dragged; the shape fits inside it and turns around its middle, so a turned shape can stick out of the box. Hollow keeps only a band along the edge, e.g. a hollow diamond for the outside edge of a base."
            })
            .into_any_element(),
        );
        v
    }

    fn position_fields(&self, id: u64, target: Target, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = match target {
            Target::Fixed => vec![
                self.num_field("X", Field::X, 1.0, cx).into_any_element(),
                self.num_field("Y", Field::Y, 1.0, cx).into_any_element(),
                small_btn("pick", "Pick position on screen (3 s)")
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.start_pick(id);
                        cx.notify();
                    }))
                    .into_any_element(),
            ],
            Target::Area => self.area_block(id, cx),
            Target::FoundImage => vec![
                self.num_field("Offset X", Field::OffX, 1.0, cx).into_any_element(),
                self.num_field("Offset Y", Field::OffY, 1.0, cx).into_any_element(),
                help("Uses the centre of whatever the last Find image step found, moved by the offset.").into_any_element(),
            ],
            Target::Current => vec![help("Acts wherever the mouse pointer already is.").into_any_element()],
        };
        out.push(self.num_field("Random offset (px)", Field::ClickJitter, 1.0, cx).into_any_element());
        out.push(
            help("0 = exactly on the spot. Otherwise it moves up to this many pixels in a random direction each time, and never lands on exactly the same spot twice in a row.")
                .into_any_element(),
        );
        // How fast the mouse gets there in this step (human-like movement only).
        let step_speed = match self.doc.node(id).map(|n| &n.kind) {
            Some(NodeKind::Click { move_speed, .. } | NodeKind::MoveMouse { move_speed, .. }) => *move_speed,
            _ => 0,
        };
        out.push(self.num_field("Mouse speed for this step (%, 0 = macro's)", Field::StepMouseSpeed, 25.0, cx).into_any_element());
        out.push(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children([(0u32, "Macro's"), (50, "50%"), (100, "100%"), (200, "200%"), (400, "400%")].into_iter().map(
                    |(pct, label)| {
                        chip(("step-speed", pct as usize), label, step_speed == pct).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.update_node(id, |k| match k {
                                    NodeKind::Click { move_speed, .. } | NodeKind::MoveMouse { move_speed, .. } => {
                                        *move_speed = pct
                                    }
                                    _ => {}
                                });
                                cx.notify();
                            },
                        ))
                    },
                ))
                .into_any_element(),
        );
        let macro_speed = self.doc.mouse_speed;
        out.push(
            help(&if !self.doc.human_mouse {
                "Only used when “Human-like mouse movement” is on in the Macro tab. Right now the mouse jumps straight there.".to_string()
            } else if step_speed == 0 {
                format!("Uses the macro's mouse speed ({macro_speed}%). Set a number to make just this step faster (e.g. 300) or slower (e.g. 50).")
            } else {
                format!("This step moves at {step_speed}% instead of the macro's {macro_speed}%. 200% gets there in half the time.")
            })
            .into_any_element(),
        );
        out
    }

    fn recording_settings(&self, id: u64, events: &[RecEvent], cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let total: u64 = events.iter().map(|e| e.delay_ms).sum();
        let moves = events.iter().filter(|e| e.action.is_move()).count();
        out.push(self.num_field("Playback speed (×)", Field::RecSpeed, 0.25, cx).into_any_element());
        out.push(
            help(&format!(
                "{} events ({} mouse moves) · {:.1}s at 1×",
                events.len(),
                moves,
                total as f32 / 1000.0
            ))
            .into_any_element(),
        );
        out.push(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(small_btn("rec-half", "Delays × 0.5").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| k.scale_recording(0.5));
                        cx.notify();
                    },
                )))
                .child(small_btn("rec-double", "Delays × 2").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| k.scale_recording(2.0));
                        cx.notify();
                    },
                )))
                .child(small_btn("rec-set", "Set all to fixed delay").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.commit_edit();
                        let v = this.bulk_delay;
                        this.update_node(id, |k| {
                            if let NodeKind::Recording { events, .. } = k {
                                for e in events.iter_mut() {
                                    e.delay_ms = v;
                                }
                            }
                        });
                        this.status = format!("Every delay in this recording is now {v} ms.");
                        cx.notify();
                    },
                )))
                .child(small_btn("rec-nomoves", "Remove mouse moves").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| {
                            if let NodeKind::Recording { events, .. } = k {
                                let mut carry = 0;
                                let mut kept = Vec::with_capacity(events.len());
                                for mut e in events.drain(..) {
                                    if e.action.is_move() {
                                        carry += e.delay_ms;
                                    } else {
                                        e.delay_ms += carry;
                                        carry = 0;
                                        kept.push(e);
                                    }
                                }
                                *events = kept;
                            }
                        });
                        cx.notify();
                    },
                )))
                .child(small_btn("rec-trim", "No wait at start").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| {
                            if let NodeKind::Recording { events, .. } = k {
                                if let Some(e) = events.first_mut() {
                                    e.delay_ms = 0;
                                }
                            }
                        });
                        cx.notify();
                    },
                )))
                .into_any_element(),
        );
        out.push(self.num_field("Fixed delay (ms)", Field::BulkDelay, 10.0, cx).into_any_element());
        out.push(div().h(px(4.)).into_any_element());
        out.push(section_title("Events  (delay before each one)").into_any_element());

        const MAX_ROWS: usize = 400;
        for (i, e) in events.iter().enumerate().take(MAX_ROWS) {
            let field = Field::RecDelay(i);
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .py_0p5()
                    .child(
                        div()
                            .w(px(26.))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!("{}", i + 1)),
                    )
                    .child(self.value_box(field, 56.0, cx))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .truncate()
                            .child(e.action.describe()),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("rec-del-{i}")))
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .hover(|s| s.bg(rgb(RED)).text_color(rgb(0xffffff)))
                            .cursor_pointer()
                            .child("×")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.editing = None;
                                this.update_node(id, |k| {
                                    if let NodeKind::Recording { events, .. } = k {
                                        if i < events.len() {
                                            let removed = events.remove(i);
                                            if let Some(next) = events.get_mut(i) {
                                                next.delay_ms += removed.delay_ms;
                                            }
                                        }
                                    }
                                });
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }
        if events.len() > MAX_ROWS {
            out.push(
                help(&format!(
                    "…and {} more. Use “Remove mouse moves” to shorten the list.",
                    events.len() - MAX_ROWS
                ))
                .into_any_element(),
            );
        }
        out
    }

    fn find_image_settings(
        &self,
        id: u64,
        template: Option<PathBuf>,
        click_when_found: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let gone = matches!(
            self.doc.node(id).map(|n| &n.kind),
            Some(NodeKind::FindImage { until_gone: true, .. })
        );
        // Hide the preview while searching, so the search can't find our own preview.
        let show_preview = matches!(self.mode, Mode::Idle | Mode::Snipping(_) | Mode::Picking(_));
        let preview: AnyElement = match (&template, show_preview) {
            (Some(p), true) => img(p.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            (Some(_), false) => div()
                .text_color(rgb(MUTED))
                .child("(preview hidden while searching)")
                .into_any_element(),
            (None, _) => div()
                .text_color(rgb(MUTED))
                .child("No image yet")
                .into_any_element(),
        };
        if show_preview {
            out.extend(self.picture_box(id, 110., preview, "Image to find", cx));
        } else {
            // While searching, show nothing that could be found on screen.
            out.push(
                div()
                    .h(px(110.))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(rgb(UI_INPUT))
                    .border_1()
                    .border_color(rgb(UI_EDGE))
                    .p_1()
                    .child(preview)
                    .into_any_element(),
            );
        }
        if let Some(p) = &template {
            out.push(help(&p.display().to_string()).into_any_element());
        }
        out.push(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(small_btn("snip-box", "Drag a box on screen").on_click(cx.listener(
                    move |this, ev: &ClickEvent, window, cx| {
                        this.start_area(AreaPurpose::Template(id), ev.modifiers().shift, window, cx);
                        cx.notify();
                    },
                )))
                .child(small_btn("snip", "Snip around mouse (3 s)").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.start_snip(id);
                        cx.notify();
                    },
                )))
                .child(small_btn("choose", "Choose image…").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| this.choose_template(id, cx),
                )))
                .child(small_btn("test", "Test now").on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.start_test(id, window);
                        cx.notify();
                    },
                )))
                .into_any_element(),
        );
        out.push(self.num_field("Snip size (pt)", Field::SnipSize, 8.0, cx).into_any_element());

        // Where to look.
        let area = match self.doc.node(id).map(|n| &n.kind) {
            Some(NodeKind::FindImage { search_area, .. }) => *search_area,
            _ => None,
        };
        out.push(div().h(px(4.)).into_any_element());
        out.push(
            labeled(
                "Look in",
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(chip("area-whole", "Whole screen", area.is_none()).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.update_node(id, |k| {
                                if let NodeKind::FindImage { search_area, .. } = k {
                                    *search_area = None;
                                }
                            });
                            cx.notify();
                        },
                    )))
                    .child(chip("area-part", "One area", area.is_some()).on_click(cx.listener(
                        move |this, ev: &ClickEvent, window, cx| {
                            this.start_area(AreaPurpose::SearchArea(id), ev.modifiers().shift, window, cx);
                            cx.notify();
                        },
                    ))),
            )
            .into_any_element(),
        );
        match area {
            Some(r) => {
                out.push(
                    help(&format!(
                        "Only looking inside {} × {} at ({}, {}). Much faster than the whole screen.",
                        r.w, r.h, r.x, r.y
                    ))
                    .into_any_element(),
                );
                out.push(
                    small_btn("area-reselect", "Select search area again")
                        .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                            this.start_area(AreaPurpose::SearchArea(id), ev.modifiers().shift, window, cx);
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
            None => out.push(
                help("Tip: if the image always appears in the same part of the screen, choose “One area” and drag a box around that part. Searching is much faster and won't match things elsewhere.")
                    .into_any_element(),
            ),
        }
        out.push(div().h(px(4.)).into_any_element());
        out.push(self.num_field("Match threshold", Field::Threshold, 0.01, cx).into_any_element());
        out.push(
            self.num_field(if gone { "Wait up to (ms)" } else { "Keep looking for (ms)" }, Field::Timeout, 500.0, cx)
                .into_any_element(),
        );
        out.push(self.num_field("Check every (ms)", Field::Interval, 50.0, cx).into_any_element());
        out.push(
            toggle("find-gone", "Wait until it's gone", gone)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.update_node(id, |k| {
                        if let NodeKind::FindImage { until_gone, .. } = k {
                            *until_gone = !*until_gone;
                        }
                    });
                    cx.notify();
                }))
                .into_any_element(),
        );
        if gone {
            out.push(help("Follows “gone” as soon as the image disappears (e.g. a loading screen), or “still there” if it's still showing when the time runs out.").into_any_element());
            return out;
        }
        out.push(
            toggle("find-click", "Click it when found", click_when_found)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.update_node(id, |k| {
                        if let NodeKind::FindImage { click_when_found, .. } = k {
                            *click_when_found = !*click_when_found;
                        }
                    });
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(help("Follows “found” as soon as the image appears, or “not found” after the time runs out. Later Click / Move steps can aim at the found image (set Where → Found image). Threshold 0.85 is a good start; lower it if it misses, raise it if it finds the wrong thing.").into_any_element());
        out
    }

    fn set_var_settings(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            self.text_field("Variable name", Field::VarName, cx).into_any_element(),
            self.text_field("Value", Field::VarExpr, cx).into_any_element(),
            help("The value can be:").into_any_element(),
            help("• a number: 0").into_any_element(),
            help("• a sum using other variables: gold + 1000, (a + b) / 2, count * 3, total % 10").into_any_element(),
            help("• text, with {name} for a variable: Gold is {gold}").into_any_element(),
            help("• another variable: gold_before").into_any_element(),
            help("A variable that hasn't been set yet counts as 0 in sums, so “count + 1” works straight away. Built-in: text (the last Read text), run (which repeat this is), found_x / found_y (the last found image).").into_any_element(),
            help("Use variables in If steps, and as {name} in Type text and Pop-up steps.").into_any_element(),
        ]
    }

    fn if_settings(&self, id: u64, op: CmpOp, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        out.push(self.text_field("Check", Field::IfLeft, cx).into_any_element());
        out.push(
            labeled(
                "Is",
                div().flex().flex_wrap().gap_1().children(CmpOp::ALL.into_iter().enumerate().map(|(i, v)| {
                    chip(("cmp-op", i), v.label(), v == op).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.update_node(id, |k| {
                                if let NodeKind::IfCompare { op, .. } = k {
                                    *op = v;
                                }
                            });
                            cx.notify();
                        },
                    ))
                })),
            )
            .into_any_element(),
        );
        out.push(self.text_field("Than / to", Field::IfRight, cx).into_any_element());
        out.push(help("Both sides can be numbers, variables, sums or text, e.g. “gold” ≥ “gold_before + 1000”, or “text” contains “Complete”. Numbers compare as numbers; text ignores upper / lower case.").into_any_element());
        out.push(help("Goes out “yes” if it's true, otherwise “no”.").into_any_element());
        out
    }

    fn pixel_settings(&self, id: u64, color: u32, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            labeled(
                "Colour",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(22.))
                            .flex_none()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(UI_EDGE))
                            .bg(rgb(color)),
                    )
                    .child(div().flex_1().child(self.value_box(Field::PixelHex, 0.0, cx))),
            )
            .into_any_element(),
            self.num_field("X", Field::X, 1.0, cx).into_any_element(),
            self.num_field("Y", Field::Y, 1.0, cx).into_any_element(),
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(small_btn("pixel-pick", "Pick spot & colour (3 s)").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.start_pixel_pick(id);
                        cx.notify();
                    },
                )))
                .child(small_btn("pixel-test", "Test now").on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.start_pixel_test(id, window);
                        cx.notify();
                    },
                )))
                .into_any_element(),
            self.num_field("Tolerance (0–255)", Field::PixelTol, 4.0, cx).into_any_element(),
            self.num_field("Keep checking for (ms)", Field::Timeout, 500.0, cx).into_any_element(),
            self.num_field("Check every (ms)", Field::Interval, 20.0, cx).into_any_element(),
            help("Goes out “match” if the spot is this colour (each of red / green / blue within the tolerance), otherwise “no match”. Much faster than Find image, e.g. for a health bar or a button turning green. Keep checking 0 = check once; set it higher to wait for the colour.").into_any_element(),
        ]
    }

    fn notify_settings(
        &self,
        id: u64,
        style: NoticeStyle,
        position: NoticePos,
        wait_for_click: bool,
        sound: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        out.push(self.text_field("Title", Field::ToastTitle, cx).into_any_element());
        out.push(self.text_field("Message", Field::ToastText, cx).into_any_element());
        out.push(help("Use {text} to show what the last Read text step read, e.g. “Gold is now {text}”.").into_any_element());
        out.push(
            labeled(
                "Style",
                div().flex().flex_wrap().gap_1().children(NoticeStyle::ALL.into_iter().enumerate().map(
                    |(i, v)| {
                        chip(("nstyle", i), v.label(), v == style).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.update_node(id, |k| {
                                    if let NodeKind::Notify { style, .. } = k {
                                        *style = v;
                                    }
                                });
                                cx.notify();
                            },
                        ))
                    },
                )),
            )
            .into_any_element(),
        );
        let places = [(NoticePos::TopCenter, "Top centre"), (NoticePos::TopRight, "Top right")];
        out.push(
            labeled(
                "Where",
                div().flex().flex_wrap().gap_1().children(places.into_iter().enumerate().map(
                    |(i, (v, label))| {
                        chip(("npos", i), label, v == position).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.update_node(id, |k| {
                                    if let NodeKind::Notify { position, .. } = k {
                                        *position = v;
                                    }
                                });
                                cx.notify();
                            },
                        ))
                    },
                )),
            )
            .into_any_element(),
        );
        out.push(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(toggle("n-wait", "Pause until I click it", wait_for_click).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| {
                            if let NodeKind::Notify { wait_for_click, .. } = k {
                                *wait_for_click = !*wait_for_click;
                            }
                        });
                        cx.notify();
                    },
                )))
                .child(toggle("n-sound", "Play a sound", sound).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| {
                            if let NodeKind::Notify { sound, .. } = k {
                                *sound = !*sound;
                            }
                        });
                        cx.notify();
                    },
                )))
                .into_any_element(),
        );
        if !wait_for_click {
            out.push(self.num_field("Show for (ms)", Field::ToastMs, 500.0, cx).into_any_element());
        }
        out.push(
            small_btn("n-preview", "Preview pop-up")
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.preview_notification(id);
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(help(if wait_for_click {
            "The macro pauses until you click the pop-up (Esc still stops it). Handy for “check this, then carry on”."
        } else {
            "The macro carries on straight away; the pop-up fades out by itself, or click it to close it. A new pop-up replaces the old one."
        })
        .into_any_element());
        out.push(help("Pop-ups never take focus, so they won't interrupt typing or clicks the macro is doing.").into_any_element());
        out
    }

    /// Show what a Pop-up step looks like, without running the macro.
    fn preview_notification(&mut self, id: u64) {
        self.commit_edit();
        let Some(NodeKind::Notify {
            title,
            message,
            style,
            position,
            duration_ms,
            sound,
            ..
        }) = self.doc.node(id).map(|n| n.kind.clone())
        else {
            return;
        };
        if let Some(mut old) = self.preview_toast.take() {
            let _ = old.kill();
            let _ = old.wait();
        }
        let toast = vision::Toast {
            title: title.replace("{text}", "12345"),
            message: message.replace("{text}", "12345"),
            style: style.key(),
            position: position.key(),
            // A preview never pauses anything, so it always closes by itself.
            duration_ms: duration_ms.max(2500),
            wait: false,
            sound,
        };
        match vision::show_notification(&toast) {
            Ok(child) => {
                self.preview_toast = Some(child);
                self.status = if cfg!(target_os = "windows") {
                    "Showing a preview pop-up ({text} shows as 12345). The very first one can take a second to appear.".into()
                } else {
                    "Showing a preview pop-up ({text} shows as 12345).".into()
                };
            }
            Err(e) => self.status = e,
        }
    }

    fn group_settings(&self, id: u64, graph: &Graph, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        out.push(self.text_field("Group name", Field::GroupName, cx).into_any_element());
        out.push(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(small_btn("grp-open", "Open group").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.enter_group(id);
                        cx.notify();
                    },
                )))
                .child(small_btn("grp-ungroup", "Ungroup").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.ungroup_selected();
                        cx.notify();
                    },
                )))
                .into_any_element(),
        );
        out.push(help("Double-click the group (or press Enter) to open it. Ungroup puts the steps back where the group is.").into_any_element());
        out.push(div().h(px(4.)).into_any_element());
        out.push(section_title("Steps inside").into_any_element());
        let start = graph
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::GroupIn))
            .map(|n| n.id);
        let mut listed: Vec<u64> = Vec::new();
        if let Some(s) = start {
            for sid in graph.chain_from(s) {
                if let Some(n) = graph.nodes.iter().find(|n| n.id == sid) {
                    if !matches!(n.kind, NodeKind::GroupIn | NodeKind::GroupOut) {
                        listed.push(sid);
                    }
                }
            }
        }
        // Anything not on the main line (e.g. branches) goes at the end.
        for n in &graph.nodes {
            if !listed.contains(&n.id) && !matches!(n.kind, NodeKind::GroupIn | NodeKind::GroupOut) {
                listed.push(n.id);
            }
        }
        if listed.is_empty() {
            out.push(help("This group is empty. Open it and add steps.").into_any_element());
        }
        for (i, sid) in listed.iter().enumerate() {
            let Some(n) = graph.nodes.iter().find(|n| n.id == *sid) else { continue };
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_0p5()
                    .child(
                        div()
                            .w(px(18.))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!("{}", i + 1)),
                    )
                    .child(div().size(px(8.)).flex_none().rounded_full().bg(rgb(n.kind.color())))
                    .child(div().flex_none().text_xs().child(n.kind.title()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(self.summary_view(&n.kind, 1.0)),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    fn multi_settings(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let ids = self.selection();
        let mut out: Vec<AnyElement> = vec![
            div()
                .text_base()
                .font_weight(FontWeight::BOLD)
                .child(format!("{} steps selected", ids.len()))
                .into_any_element(),
            small_btn("multi-group", "Group these steps")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.group_selection();
                    cx.notify();
                }))
                .into_any_element(),
            small_btn("multi-delete", "Delete these steps")
                .text_color(rgb(RED))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.delete_selected();
                    cx.notify();
                }))
                .into_any_element(),
            help(&format!(
                "Grouping needs the steps to be one straight line, each connected to the next. Shift-click or Shift-drag to add steps · {}G groups · Shift+{}G ungroups.",
                SHORTCUT_KEY, SHORTCUT_KEY
            ))
            .into_any_element(),
            div().h(px(4.)).into_any_element(),
        ];
        for id in ids {
            if let Some(n) = self.doc.node(id) {
                out.push(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().size(px(8.)).flex_none().rounded_full().bg(rgb(n.kind.color())))
                        .child(div().text_xs().child(n.kind.title()))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .truncate()
                                .child(self.summary_view(&n.kind, 1.0)),
                        )
                        .into_any_element(),
                );
            }
        }
        out
    }

    fn window_check_settings(
        &self,
        id: u64,
        scope: WinScope,
        field: WinField,
        mode: WinMatch,
        case_sensitive: bool,
        bring_to_front: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let scopes = [(WinScope::Frontmost, "Current window"), (WinScope::AnyOpen, "Any open window")];
        out.push(
            labeled(
                "Look at",
                div().flex().flex_wrap().gap_1().children(scopes.into_iter().map(|(v, label)| {
                    chip(("wscope", v as usize), label, v == scope).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.update_node(id, |k| {
                                if let NodeKind::WindowCheck { scope, .. } = k {
                                    *scope = v;
                                }
                            });
                            cx.notify();
                        },
                    ))
                })),
            )
            .into_any_element(),
        );
        let fields = [(WinField::Title, "Title"), (WinField::App, "App name"), (WinField::Either, "Either")];
        out.push(
            labeled(
                "Check its",
                div().flex().flex_wrap().gap_1().children(fields.into_iter().map(|(v, label)| {
                    chip(("wfield", v as usize), label, v == field).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.update_node(id, |k| {
                                if let NodeKind::WindowCheck { field, .. } = k {
                                    *field = v;
                                }
                            });
                            cx.notify();
                        },
                    ))
                })),
            )
            .into_any_element(),
        );
        let modes = [WinMatch::Contains, WinMatch::Equals, WinMatch::StartsWith];
        out.push(
            labeled(
                "Which",
                div().flex().flex_wrap().gap_1().children(modes.into_iter().map(|v| {
                    chip(("wmode", v as usize), v.label(), v == mode).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.update_node(id, |k| {
                                if let NodeKind::WindowCheck { mode, .. } = k {
                                    *mode = v;
                                }
                            });
                            cx.notify();
                        },
                    ))
                })),
            )
            .into_any_element(),
        );
        out.push(self.text_field("Window text", Field::WinText, cx).into_any_element());
        out.push(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(toggle("win-case", "Match case", case_sensitive).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| {
                            if let NodeKind::WindowCheck { case_sensitive, .. } = k {
                                *case_sensitive = !*case_sensitive;
                            }
                        });
                        cx.notify();
                    },
                )))
                .child(toggle("win-front", "Bring it to the front if found", bring_to_front).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| {
                            if let NodeKind::WindowCheck { bring_to_front, .. } = k {
                                *bring_to_front = !*bring_to_front;
                            }
                        });
                        cx.notify();
                    }),
                ))
                .into_any_element(),
        );
        out.push(self.num_field("Keep checking for (ms)", Field::Timeout, 500.0, cx).into_any_element());
        out.push(self.num_field("Check every (ms)", Field::Interval, 50.0, cx).into_any_element());
        out.push(help("“yes” if a window matches, otherwise “no”. Keep checking 0 = check once; set it higher to wait for a window to open or come to the front.").into_any_element());

        out.push(div().h(px(4.)).into_any_element());
        out.push(
            small_btn("win-list", "Show open windows")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.start_list_windows();
                    cx.notify();
                }))
                .into_any_element(),
        );
        for (i, w) in self.windows.iter().take(25).enumerate() {
            let (app, title) = (w.app.clone(), w.title.clone());
            let label = format!(
                "{}{} — {}",
                if w.focused { "★ " } else { "" },
                w.app,
                if w.title.is_empty() { "(no title)" } else { w.title.as_str() }
            );
            out.push(
                div()
                    .id(("win-row", i))
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .text_xs()
                    .truncate()
                    .bg(rgb(UI_RAISED))
                    .hover(|s| s.bg(rgb(UI_HOVER)))
                    .cursor_pointer()
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let (app, title) = (app.clone(), title.clone());
                        this.editing = None;
                        this.update_node(id, |k| {
                            if let NodeKind::WindowCheck { text, field, .. } = k {
                                if title.is_empty() {
                                    *text = app;
                                    *field = WinField::App;
                                } else {
                                    *text = title;
                                    *field = WinField::Title;
                                }
                            }
                        });
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        if !self.windows.is_empty() {
            out.push(help(WINDOW_LIST_NOTE).into_any_element());
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn read_text_settings(
        &self,
        id: u64,
        compare: TextCompare,
        case_sensitive: bool,
        numbers_only: bool,
        preview: Option<PathBuf>,
        w: i32,
        h: i32,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let has_area = w > 0 && h > 0;
        let preview_el: AnyElement = match (&preview, has_area) {
            (Some(p), true) => img(p.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            _ => div()
                .text_color(rgb(MUTED))
                .child(if has_area { "Area set" } else { "No area yet" })
                .into_any_element(),
        };
        out.extend(self.picture_box(id, if self.seen.contains_key(&id) { 150. } else { 80. }, preview_el, "When selected", cx));
        out.push(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(small_btn("area-pick", "Select area (drag a box)").on_click(cx.listener(
                    move |this, ev: &ClickEvent, window, cx| {
                        this.start_area(AreaPurpose::ReadText(id), ev.modifiers().shift, window, cx);
                        cx.notify();
                    },
                )))
                .child(small_btn("area-read", "Read now").on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.start_read_test(id, window);
                        cx.notify();
                    },
                )))
                .into_any_element(),
        );
        out.push(help("Shift-click “Select area” to get 3 seconds to switch to another window first.").into_any_element());
        let gone = matches!(
            self.doc.node(id).map(|n| &n.kind),
            Some(NodeKind::ReadText { until_gone: true, .. })
        );
        out.push(
            toggle("read-gone", "Wait until it stops matching (gone)", gone)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.update_node(id, |k| {
                        if let NodeKind::ReadText { until_gone, .. } = k {
                            *until_gone = !*until_gone;
                        }
                    });
                    cx.notify();
                }))
                .into_any_element(),
        );
        if gone {
            out.push(help("Keeps reading until the comparison below is no longer true (e.g. until “Loading” disappears), then follows “gone”. If it's still true when the time runs out, it follows “still there”.").into_any_element());
        }
        out.push(self.num_field("X", Field::X, 1.0, cx).into_any_element());
        out.push(self.num_field("Y", Field::Y, 1.0, cx).into_any_element());
        out.push(self.num_field("Width", Field::W, 1.0, cx).into_any_element());
        out.push(self.num_field("Height", Field::H, 1.0, cx).into_any_element());
        out.push(div().h(px(4.)).into_any_element());

        let row = move |title: &'static str, opts: [TextCompare; 6], cx: &mut Context<Self>| {
            labeled(
                title,
                div().flex().flex_wrap().gap_1().children(opts.into_iter().map(|v| {
                    chip(("cmp", v as usize), v.label(), v == compare).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.update_node(id, |k| {
                                if let NodeKind::ReadText { compare, .. } = k {
                                    *compare = v;
                                }
                            });
                            cx.notify();
                        },
                    ))
                })),
            )
            .into_any_element()
        };
        out.push(row("Text", TextCompare::TEXT, cx));
        out.push(row("Number", TextCompare::NUMBER, cx));
        if compare.needs_value() {
            let label = if compare.is_number() { "Compare with (number)" } else { "Compare with" };
            out.push(self.text_field(label, Field::CompareValue, cx).into_any_element());
        }
        if compare.is_number() {
            out.push(
                help("Reading as a number: misread digits like O→0 and l→1 are fixed, and split digits are joined.")
                    .into_any_element(),
            );
        } else {
            out.push(
                toggle("rt-numbers", "Only read a number", numbers_only)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| {
                            if let NodeKind::ReadText { numbers_only, .. } = k {
                                *numbers_only = !*numbers_only;
                            }
                        });
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        if !compare.is_number() && compare.needs_value() {
            out.push(
                toggle("rt-case", "Match case", case_sensitive)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| {
                            if let NodeKind::ReadText { case_sensitive, .. } = k {
                                *case_sensitive = !*case_sensitive;
                            }
                        });
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        out.push(self.text_field("Save what's read into variable (optional)", Field::ReadSaveTo, cx).into_any_element());
        out.push(self.num_field("Keep reading for (ms)", Field::Timeout, 500.0, cx).into_any_element());
        out.push(self.num_field("Read every (ms)", Field::Interval, 100.0, cx).into_any_element());
        let explain = if compare.is_number() {
            "Uses the first number in the area (commas, £/$ and minus signs are handled). “match” if the comparison is true, otherwise “no match”."
        } else if compare == TextCompare::Changed {
            "“match” once the text differs from the previous reading (from the last time this step ran, or from the first read now). Set “Keep reading” to wait for a change."
        } else {
            "“match” if the text passes, otherwise “no match”. Keep reading 0 = read once; set it higher to wait for the text to appear."
        };
        out.push(help(explain).into_any_element());
        out.push(help("For the best results, draw the box tightly around just the text or number, with a little space round it. Leave out icons and labels. If “Read now” says the reads disagreed, try a slightly bigger or tighter box.").into_any_element());
        out.push(help("Type text steps can type what was read with {text}. Make sure MacroFlow isn't covering the area when the macro runs.").into_any_element());
        out
    }

    // -----------------------------------------------------------------------
    // Field widgets
    // -----------------------------------------------------------------------

    fn value_box(&self, field: Field, width: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let editing_buf = match &self.editing {
            Some((f, buf)) if *f == field => Some(buf.clone()),
            _ => None,
        };
        let editing = editing_buf.is_some();
        // Measures where the text starts (8 px of padding in from the box's edge) and
        // which font it uses, so clicks can be turned into a cursor position.
        let geom = self.field_geom.clone();
        let probe = canvas(
            move |bounds, window, _| {
                let style = window.text_style();
                let size = f32::from(style.font_size.to_pixels(window.rem_size()));
                geom.borrow_mut()
                    .insert(field, (f32::from(bounds.origin.x) + 8.0, style.font(), size));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let mut base = div()
            .id(SharedString::from(format!("val-{field:?}")))
            .relative()
            .flex()
            .items_center()
            .min_w(px(0.))
            .min_h(px(28.))
            .px_2()
            .rounded_md()
            .bg(rgb(UI_INPUT))
            .border_1()
            .border_color(if editing { rgb(ACCENT) } else { rgb(UI_EDGE) })
            .cursor_text()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(probe)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let already = matches!(&this.editing, Some((f, _)) if *f == field);
                    if !already {
                        this.start_edit(field, window);
                    }
                    let len = this.editing.as_ref().map_or(0, |(_, b)| b.chars().count());
                    if ev.click_count >= 2 {
                        // Double-click selects everything.
                        this.edit_anchor = Some(0);
                        this.edit_cursor = len;
                    } else {
                        let at = this.char_index_at(field, f32::from(ev.position.x), window);
                        if ev.modifiers.shift && already {
                            if this.edit_anchor.is_none() {
                                this.edit_anchor = Some(this.edit_cursor);
                            }
                        } else {
                            this.edit_anchor = Some(at);
                        }
                        this.edit_cursor = at;
                        this.field_drag = true;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, window, cx| {
                let editing_this = matches!(&this.editing, Some((f, _)) if *f == field);
                if this.field_drag && editing_this && ev.pressed_button == Some(MouseButton::Left) {
                    let at = this.char_index_at(field, f32::from(ev.position.x), window);
                    if at != this.edit_cursor {
                        this.edit_cursor = at;
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.field_drag = false),
            );
        base = match editing_buf {
            None => {
                let text = self.get_field(field);
                let known = |n: &str| self.is_known_var(n);
                match field.var_mode() {
                    Some(mode) if var_ranges(&text, mode, &known).len() > 0 => {
                        base.child(spans_view(&var_spans(&text, mode, &known), &known, 1.0))
                    }
                    _ => base.child(div().truncate().child(text)),
                }
            }
            Some(buf) => {
                let pills = field.edit_pills(&buf);
                base.child(self.render_edit_text(&buf, &pills))
            }
        };
        if width > 0.0 {
            base.w(px(width)).flex_none()
        } else {
            base.flex_1()
        }
    }

    /// The text being edited, with the cursor and any selection.
    fn render_edit_text(&self, buf: &str, pills: &[VarRange]) -> Div {
        let chars: Vec<char> = buf.chars().collect();
        let len = chars.len();
        let cur = self.edit_cursor.min(len);
        let caret = || div().flex_none().w(px(1.5)).h(px(15.)).mx(px(-0.75)).bg(rgb(TEXT));
        let sel = match self.edit_anchor {
            Some(a) if a.min(len) != cur => Some((a.min(len).min(cur), a.min(len).max(cur))),
            _ => None,
        };
        let in_sel = |p: usize| sel.map_or(false, |(a, b)| p >= a && p < b);
        let text_piece = |text: String, selected: bool| {
            let d = div().flex_none().child(text);
            if selected {
                d.rounded_sm().bg(rgba((ACCENT << 8) | 0x77)).text_color(rgb(0xffffff))
            } else {
                d
            }
        };
        let mut row = div().flex().flex_row().items_center().whitespace_nowrap();
        for seg in edit_segments(len, pills, cur, sel) {
            match seg {
                EditSeg::Pill(a, _, name) => {
                    if cur == a {
                        row = row.child(caret());
                    }
                    row = row.child(var_pill(&name, self.is_known_var(&name), in_sel(a), 1.0));
                }
                EditSeg::Plain(a, b) => {
                    // Split the run where the caret and the selection's ends fall.
                    let mut cuts = vec![a, b, cur];
                    if let Some((x, y)) = sel {
                        cuts.push(x);
                        cuts.push(y);
                    }
                    cuts.retain(|c| *c >= a && *c <= b);
                    cuts.sort();
                    cuts.dedup();
                    for w in cuts.windows(2) {
                        if w[0] == cur {
                            row = row.child(caret());
                        }
                        let text: String = chars[w[0]..w[1]].iter().collect();
                        row = row.child(text_piece(text, in_sel(w[0])));
                    }
                }
            }
        }
        if cur == len {
            row = row.child(caret());
        }
        row
    }

    /// Is this a variable some step sets (or a built-in one)?
    fn is_known_var(&self, name: &str) -> bool {
        self.vars_known.contains(name)
    }

    /// A step's summary, with any variables shown as pills.
    fn summary_view(&self, kind: &NodeKind, z: f32) -> AnyElement {
        let known = |n: &str| self.is_known_var(n);
        match kind.summary_spans(&known) {
            Some(spans) => spans_view(&spans, &known, z).into_any_element(),
            None => kind.summary().into_any_element(),
        }
    }

    /// Put a variable into the box being edited: `{name}` at the cursor, or (for a
    /// box that holds a variable's name) the name itself.
    fn insert_var(&mut self, field: Field, name: &str) {
        let Some((f, buf)) = self.editing.take() else { return };
        if f != field {
            self.editing = Some((f, buf));
            return;
        }
        let mut ed = TextEdit::new(&buf, self.edit_cursor, self.edit_anchor);
        if field.var_mode() == Some(VarMode::Name) {
            ed.anchor = Some(0);
            ed.cur = ed.chars.len();
            ed.insert(name);
        } else {
            ed.insert(&format!("{{{name}}}"));
        }
        self.edit_cursor = ed.cur;
        self.edit_anchor = ed.anchor;
        self.editing = Some((f, ed.text()));
    }

    /// Which character a click at window x lands on, in the field being edited.
    fn char_index_at(&self, field: Field, x: f32, window: &mut Window) -> usize {
        let Some((_, text)) = self.editing.as_ref().filter(|(f, _)| *f == field) else {
            return 0;
        };
        let len = text.chars().count();
        if text.is_empty() {
            return 0;
        }
        let Some((origin, font, size)) = self.field_geom.borrow().get(&field).cloned() else {
            return len;
        };
        let ts = window.text_system().clone();
        let shape = |t: &str| {
            let run = TextRun {
                len: t.len(),
                font: font.clone(),
                color: hsla(TEXT),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            ts.shape_line(SharedString::from(t.to_string()), px(size), &[run], None)
        };
        // Walk along the text: plain runs are measured letter by letter, a pill
        // counts as one block (a click on it goes to whichever end is nearer).
        let chars: Vec<char> = text.chars().collect();
        let pills = field.edit_pills(text);
        let sel = self
            .edit_anchor
            .filter(|a| *a != self.edit_cursor)
            .map(|a| (a.min(self.edit_cursor), a.max(self.edit_cursor)));
        let mut left = origin;
        for seg in edit_segments(len, &pills, self.edit_cursor.min(len), sel) {
            match seg {
                EditSeg::Pill(a, b, name) => {
                    let w = f32::from(shape(&format!("{VAR_MARK}{name}")).width) + PILL_EXTRA;
                    if x < left + w {
                        return if x < left + w / 2.0 { a } else { b };
                    }
                    left += w;
                }
                EditSeg::Plain(a, b) => {
                    let piece: String = chars[a..b].iter().collect();
                    let line = shape(&piece);
                    let w = f32::from(line.width);
                    if x < left + w {
                        let byte = line.closest_index_for_x(px(x - left)).min(piece.len());
                        return a + piece.get(..byte).map_or(0, |s| s.chars().count());
                    }
                    left += w;
                }
            }
        }
        len
    }

    fn num_field(&self, label: &str, field: Field, step: f64, cx: &mut Context<Self>) -> Div {
        // − and + sit inside one joined control with the value between them.
        let step_btn = |id: String, text: &'static str| {
            div()
                .id(SharedString::from(id))
                .flex_none()
                .w(px(24.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(MUTED))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT)))
                .child(text)
        };
        let minus = step_btn(format!("minus-{field:?}"), "−").on_click(cx.listener(
            move |this, ev: &ClickEvent, _, cx| {
                let mult = if ev.modifiers().shift { 10.0 } else { 1.0 };
                this.bump(field, -step * mult);
                cx.notify();
            },
        ));
        let plus = step_btn(format!("plus-{field:?}"), "+").on_click(cx.listener(
            move |this, ev: &ClickEvent, _, cx| {
                let mult = if ev.modifiers().shift { 10.0 } else { 1.0 };
                this.bump(field, step * mult);
                cx.notify();
            },
        ));
        let editing = matches!(&self.editing, Some((f, _)) if *f == field);
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .min_h(px(28.))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(label.to_string()),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(132.))
                    .h(px(28.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .rounded_md()
                    .overflow_hidden()
                    .bg(rgb(UI_INPUT))
                    .border_1()
                    .border_color(rgb(if editing { ACCENT } else { UI_EDGE }))
                    .child(minus)
                    .child(
                        self.value_box(field, 0.0, cx)
                            .h_full()
                            .rounded_none()
                            .border_0()
                            .border_l_1()
                            .border_r_1()
                            .border_color(rgb(UI_LINE))
                            .bg(rgb(UI_INPUT)),
                    )
                    .child(plus),
            )
    }

    fn text_field(&self, label: &str, field: Field, cx: &mut Context<Self>) -> Div {
        debug_assert!(field.is_text());
        let editing = matches!(&self.editing, Some((f, _)) if *f == field);
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(field_label(label))
            .child(div().flex().child(self.value_box(field, 0.0, cx)))
            .when(editing && field.var_mode().is_some(), |d| d.child(self.var_picker(field, cx)))
    }

    /// While a box that can use variables is being edited: the variables you can put in.
    fn var_picker(&self, field: Field, cx: &mut Context<Self>) -> Div {
        let name_box = field.var_mode() == Some(VarMode::Name);
        let mut row = div().flex().flex_row().flex_wrap().items_center().gap_y_1().text_xs();
        row = row.child(div().text_color(rgb(MUTED)).mr_1().child(if name_box {
            "Use an existing name:"
        } else {
            "Insert (or type {name}):"
        }));
        let names: Vec<String> = self
            .vars_known
            .iter()
            .filter(|n| !(name_box && BUILTIN_VARS.contains(&n.as_str())))
            .cloned()
            .collect();
        if names.is_empty() {
            return row.child(div().text_color(rgb(MUTED)).child("no variables yet"));
        }
        for name in names {
            let n = name.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("insert-var-{field:?}-{name}")))
                    .cursor_pointer()
                    .hover(|d| d.opacity(0.8))
                    .child(var_pill(&name, true, false, 1.0))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            // Keep editing (a click elsewhere would finish the edit).
                            cx.stop_propagation();
                            this.insert_var(field, &n);
                            cx.notify();
                        }),
                    ),
            );
        }
        row
    }
}

impl Render for MacroApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Remember the macro for undo whenever it has changed.
        self.checkpoint();
        self.vars_known = self.doc.variables_set();
        self.vars_known.extend(BUILTIN_VARS.iter().map(|v| v.to_string()));
        // What's wrong with each step (badges on the cards, notes in the Edit tab).
        self.problems.clear();
        for p in self.doc.problems() {
            let e = self.problems.entry(p.step).or_default();
            if p.serious {
                e.serious += 1;
            } else {
                e.minor += 1;
            }
            e.texts.push((p.serious, p.text.clone()));
            for g in self.doc.root.path_to(p.step).unwrap_or_default() {
                let e = self.problems.entry(g).or_default();
                e.inside += 1;
                e.inside_serious |= p.serious;
            }
        }
        // The start screen goes away once you start working.
        if self.dirty || !self.idle() {
            self.show_start = false;
        }
        let menu = self.menu.clone().map(|m| self.render_menu(&m, window, cx));
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let quick = self.quick.as_ref().map(|_| self.render_quick(vw, vh, cx));
        // The shape editor fits its screenshot into the window.
        if let Some(ed) = self.shape_edit.as_mut() {
            let (aw, ah) = ((vw - 32.0).max(200.0), (vh - 130.0).max(150.0));
            ed.k = (aw / ed.shot.w.max(1) as f32).min(ah / ed.shot.h.max(1) as f32).max(0.05);
        }
        let shape_edit = self.shape_edit.as_ref().map(|_| self.render_shape_edit(cx));
        let check = self.run_check.as_ref().map(|_| self.render_run_check(cx));
        div()
            .relative()
            .on_action(cx.listener(Self::on_quit))
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, _, cx| this.finish_drag(ev.position, cx)),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(UI_INPUT))
            .text_color(rgb(TEXT))
            .text_sm()
            .child(self.render_titlebar(window, cx))
            .child(
                // Keyboard focus lives here, not on the whole window: an element that
                // takes focus claims every mouse press inside it, and on Windows that
                // stopped the title bar from dragging the window.
                div()
                    .track_focus(&self.focus)
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .when(self.sidebar_open, |d| d.child(self.render_sidebar(cx)))
                    .child(self.render_graph(cx)),
            )
            .child(self.render_statusbar(cx))
            .children(menu)
            .children(quick)
            .children(check)
            .children(shape_edit)
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn bezier_ctrl(x0: f32, x1: f32, zoom: f32) -> f32 {
    ((x1 - x0).abs() * 0.5).max(50.0 * zoom)
}

fn bezier_mid(x0: f32, y0: f32, x1: f32, y1: f32, zoom: f32) -> (f32, f32) {
    let d = bezier_ctrl(x0, x1, zoom);
    let (ax, ay) = (x0 + d, y0);
    let (bx, by) = (x1 - d, y1);
    (
        (x0 + 3.0 * ax + 3.0 * bx + x1) / 8.0,
        (y0 + 3.0 * ay + 3.0 * by + y1) / 8.0,
    )
}

/// Draw straight lines through `pts` (closed back to the first with `closed`).
fn stroke_poly(window: &mut Window, pts: &[Point<Pixels>], width: f32, color: Hsla, closed: bool, dashed: bool) {
    if pts.len() < 2 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width));
    if dashed {
        b = b.dash_array(&[px(5.), px(4.)]);
    }
    b.move_to(pts[0]);
    for p in &pts[1..] {
        b.line_to(*p);
    }
    if closed {
        b.close();
    }
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

#[allow(clippy::too_many_arguments)]
fn stroke_curve(
    window: &mut Window,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    width: f32,
    color: Hsla,
    dashed: bool,
    zoom: f32,
) {
    let d = bezier_ctrl(x0, x1, zoom);
    let mut b = PathBuilder::stroke(px(width));
    if dashed {
        b = b.dash_array(&[px(6. * zoom), px(4. * zoom)]);
    }
    b.move_to(point(px(x0), px(y0)));
    b.cubic_bezier_to(
        point(px(x1), px(y1)),
        point(px(x0 + d), px(y0)),
        point(px(x1 - d), px(y1)),
    );
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

fn zoom_btn(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    let label: SharedString = label.into();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .px_2()
        .h(px(24.))
        .rounded_md()
        .text_color(rgb(MUTED))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT)))
        .child(label)
}

/// A solid button. (GPUI only allows one hover style per element, so the hover
/// colour is chosen here rather than added afterwards.)
fn tool_btn_colored(
    id: impl Into<ElementId>,
    label: &'static str,
    enabled: bool,
    bg: u32,
    hover_bg: u32,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .px_3()
        .h(px(28.))
        .rounded_md()
        .bg(rgb(bg))
        .text_color(rgb(TEXT))
        .when(enabled, |d| d.hover(move |s| s.bg(rgb(hover_bg))).cursor_pointer())
        .when(!enabled, |d| d.opacity(0.4))
        .child(label)
}

/// A secondary button: quiet fill, hairline border.
fn small_btn(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .px_2p5()
        .h(px(26.))
        .rounded_md()
        .bg(rgb(UI_RAISED))
        .border_1()
        .border_color(rgb(UI_EDGE))
        .text_xs()
        .text_color(rgb(TEXT))
        .whitespace_nowrap()
        .hover(|s| s.bg(rgb(UI_HOVER)))
        .cursor_pointer()
        .child(label)
}

/// One option in a set (Left / Right / Middle, Whole screen / One area …).
fn chip(id: impl Into<ElementId>, label: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .px_2p5()
        .h(px(26.))
        .rounded_md()
        .text_xs()
        .border_1()
        .whitespace_nowrap()
        .cursor_pointer()
        .when(on, |d| {
            d.bg(rgba((ACCENT << 8) | 0x2e))
                .border_color(rgba((ACCENT << 8) | 0xcc))
                .text_color(rgb(0xffffff))
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!on, |d| {
            d.bg(rgb(UI_INPUT))
                .border_color(rgb(UI_EDGE))
                .text_color(rgb(MUTED))
                .hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT)))
        })
        .child(label)
}

/// An on/off setting: the label on the left, a switch on the right. The whole row
/// is clickable.
fn toggle(id: impl Into<ElementId>, label: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        // Bleeds into the panel padding so the label lines up with other labels
        // while the hover fill still has room around it.
        .mx(px(-8.))
        .px_2()
        .py(px(6.))
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(UI_HOVER)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_xs()
                .text_color(rgb(if on { TEXT } else { MUTED }))
                .child(label),
        )
        .child(switch(on))
}

/// The little pill switch used by [`toggle`].
fn switch(on: bool) -> Div {
    switch_sized(on, 28.0, 16.0)
}

fn switch_sized(on: bool, w: f32, h: f32) -> Div {
    let knob = h - 4.0;
    div()
        .flex_none()
        .relative()
        .w(px(w))
        .h(px(h))
        .rounded_full()
        .bg(rgb(if on { ACCENT } else { UI_EDGE }))
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { w - knob - 2.0 } else { 2. }))
                .size(px(knob))
                .rounded_full()
                .bg(rgb(if on { 0xffffff } else { 0x9aa0b0 })),
        )
}

/// A label above its control (option sets need the full width).
fn labeled(label: &str, content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(field_label(label))
        .child(div().w_full().child(content))
}

fn field_label(label: &str) -> Div {
    div()
        .text_xs()
        .text_color(rgb(MUTED))
        .child(label.to_string())
}

/// A section heading. Sections after the first get a hairline above them.
fn section_title(text: &'static str) -> Div {
    div()
        .mt_2()
        .pt_3()
        .border_t_1()
        .border_color(rgb(UI_LINE))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(UI_FAINT))
        .child(text.to_uppercase())
}

/// A section heading with no line above it (the first one in a panel).
fn section_head(text: &'static str) -> Div {
    div()
        .pt_1()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(UI_FAINT))
        .child(text.to_uppercase())
}

/// A keyboard key, e.g. in the shortcut list.
fn kbd(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .px_1p5()
        .min_w(px(20.))
        .h(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .bg(rgb(UI_INPUT))
        .border_1()
        .border_color(rgb(UI_EDGE))
        .text_size(px(10.5))
        .text_color(rgb(MUTED))
        .child(text.into())
}

/// The hover pop-up behind [`help`] and title-bar buttons.
struct TipView {
    text: SharedString,
}

impl Render for TipView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(320.))
            .px_2p5()
            .py_1p5()
            .rounded_md()
            .bg(rgb(UI_RAISED))
            .border_1()
            .border_color(rgb(UI_EDGE))
            .shadow_lg()
            .text_xs()
            .text_color(rgb(TEXT))
            .child(self.text.clone())
    }
}

/// Builds a tooltip for `.tooltip(...)`.
fn tip_view(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| cx.new(|_| TipView { text: text.clone() }).into()
}

// ---------------------------------------------------------------------------
// Variable pills: wherever a variable is used it's shown as a small tinted tag
// ---------------------------------------------------------------------------

fn turn_label(deg: i32) -> &'static str {
    match deg {
        0 => "0°",
        30 => "30°",
        45 => "45°",
        60 => "60°",
        _ => "90°",
    }
}

/// A small picture of a random click area: green dots where clicks can land
/// (a fixed sample, so it doesn't flicker) inside the faint box that was dragged.
fn area_preview(r: ScreenRect, shape: &AreaShape, taps: u32) -> Div {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut unit = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 1_000_000) as f64 / 1_000_000.0
    };
    let local = ScreenRect { x: 0, y: 0, w: r.w.max(2), h: r.h.max(2) };
    let (w, h) = (local.w as f32, local.h as f32);
    // Taps: exactly where this many taps would go; otherwise a sample of spots.
    let spots: Vec<(i32, i32)> = if taps > 0 {
        spread_taps(local, shape, taps, &mut unit)
    } else {
        (0..320).map(|_| area_point(local, shape, &mut unit)).collect()
    };
    let dots: Vec<(f32, f32)> = spots.into_iter().map(|(x, y)| (x as f32 - w / 2.0, y as f32 - h / 2.0)).collect();
    let dot_size = if taps > 0 { 5.0 } else { 3.0 };
    let outline: Vec<(f32, f32)> = area_outline(local, shape)
        .into_iter()
        .map(|(x, y)| (x as f32 - w / 2.0, y as f32 - h / 2.0))
        .collect();
    // Fit the box and the turned shape.
    let th = shape.rotation.to_radians();
    let tw = (w * th.cos()).abs() + (h * th.sin()).abs();
    let thh = (w * th.sin()).abs() + (h * th.cos()).abs();
    let (fw, fh) = (tw.max(w), thh.max(h));
    div()
        .h(px(130.))
        .w_full()
        .rounded_md()
        .bg(rgb(BG))
        .border_1()
        .border_color(rgb(BORDER))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let bx = f32::from(bounds.origin.x);
                    let by = f32::from(bounds.origin.y);
                    let bw = f32::from(bounds.size.width);
                    let bh = f32::from(bounds.size.height);
                    let k = ((bw - 16.0) / fw).min((bh - 16.0) / fh).max(0.0001);
                    let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
                    let (ow, oh) = (w * k, h * k);
                    let edge = hsla(BORDER);
                    for (x, y, ew, eh) in [
                        (cx - ow / 2.0, cy - oh / 2.0, ow, 1.0),
                        (cx - ow / 2.0, cy + oh / 2.0 - 1.0, ow, 1.0),
                        (cx - ow / 2.0, cy - oh / 2.0, 1.0, oh),
                        (cx + ow / 2.0 - 1.0, cy - oh / 2.0, 1.0, oh),
                    ] {
                        window.paint_quad(fill(
                            Bounds { origin: point(px(x), px(y)), size: gpui::size(px(ew), px(eh)) },
                            edge,
                        ));
                    }
                    let pts: Vec<Point<Pixels>> =
                        outline.iter().map(|(x, y)| point(px(cx + x * k), px(cy + y * k))).collect();
                    stroke_poly(window, &pts, 1.5, rgba((ACCENT << 8) | 0xcc).into(), true, false);
                    let dot = hsla(GREEN);
                    for (dx, dy) in &dots {
                        window.paint_quad(
                            fill(
                                Bounds {
                                    origin: point(px(cx + dx * k - dot_size / 2.0), px(cy + dy * k - dot_size / 2.0)),
                                    size: gpui::size(px(dot_size), px(dot_size)),
                                },
                                dot,
                            )
                            .corner_radii(px(dot_size / 2.0)),
                        );
                    }
                },
            )
            .size_full(),
        )
}

// ---------------------------------------------------------------------------
// Editing a random area's shape on top of a screenshot
// ---------------------------------------------------------------------------

/// What's being dragged in the shape editor.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EditDrag {
    None,
    /// The whole shape; (dx, dy) from the mouse to the middle.
    Move { dx: f64, dy: f64 },
    /// A corner of the box: 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left.
    Corner(usize),
    /// The middle of a side: 0 top, 1 right, 2 bottom, 3 left.
    Edge(usize),
    /// The round handle above the shape.
    Rotate,
    /// A corner of a freeform outline.
    Point(usize),
    /// Drawing a new box from (x0, y0).
    NewBox { x0: f64, y0: f64 },
}

const CORNER_SIGNS: [(f64, f64); 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
const EDGE_SIGNS: [(f64, f64); 4] = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)];

/// The shape editor: a screenshot of one display with the area drawn on it.
/// Everything is in screen coordinates (the ones the mouse uses).
#[derive(Clone)]
struct ShapeEdit {
    step: u64,
    shot: vision::Shot,
    /// The box: its middle, size and turn (degrees, clockwise).
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    shape: AreaShape,
    /// Taps to show (Drag steps); 0 = show where clicks can land.
    taps: u32,
    drag: EditDrag,
    /// Clicks add corners one after another (drawing a new freeform outline).
    drawing: bool,
    /// Window pixels per screen point, and where the picture is in the window.
    k: f32,
    origin: Rc<Cell<(f32, f32)>>,
}

impl ShapeEdit {
    fn rot(&self) -> f64 {
        (self.shape.rotation as f64).to_radians()
    }

    /// Measured from the middle of the box (before turning) -> on screen.
    fn to_screen(&self, lx: f64, ly: f64) -> (f64, f64) {
        let t = self.rot();
        (self.cx + lx * t.cos() - ly * t.sin(), self.cy + lx * t.sin() + ly * t.cos())
    }

    /// On screen -> measured from the middle of the box (before turning).
    fn to_local(&self, sx: f64, sy: f64) -> (f64, f64) {
        let t = -self.rot();
        let (dx, dy) = (sx - self.cx, sy - self.cy);
        (dx * t.cos() - dy * t.sin(), dx * t.sin() + dy * t.cos())
    }

    fn corner(&self, i: usize) -> (f64, f64) {
        let (sx, sy) = CORNER_SIGNS[i];
        self.to_screen(sx * self.w / 2.0, sy * self.h / 2.0)
    }

    fn edge(&self, i: usize) -> (f64, f64) {
        let (sx, sy) = EDGE_SIGNS[i];
        self.to_screen(sx * self.w / 2.0, sy * self.h / 2.0)
    }

    /// The round turn handle, a little above the top of the box.
    fn rot_handle(&self) -> (f64, f64) {
        self.to_screen(0.0, -self.h / 2.0 - 28.0 / self.k.max(0.05) as f64)
    }

    /// The freeform corners on screen.
    fn poly(&self) -> Vec<(f64, f64)> {
        self.shape
            .points
            .iter()
            .map(|p| self.to_screen((p[0] as f64 - 0.5) * self.w, (p[1] as f64 - 0.5) * self.h))
            .collect()
    }

    fn is_poly(&self) -> bool {
        self.shape.kind == ShapeKind::Polygon
    }

    /// The box as a screen rectangle (what gets saved).
    fn rect(&self) -> ScreenRect {
        ScreenRect {
            x: (self.cx - self.w / 2.0).round() as i32,
            y: (self.cy - self.h / 2.0).round() as i32,
            w: self.w.round().max(2.0) as i32,
            h: self.h.round().max(2.0) as i32,
        }
    }

    /// After moving freeform corners: make the box fit them snugly again.
    fn refit(&mut self) {
        if self.shape.points.is_empty() {
            return;
        }
        let local: Vec<(f64, f64)> = self
            .shape
            .points
            .iter()
            .map(|p| ((p[0] as f64 - 0.5) * self.w, (p[1] as f64 - 0.5) * self.h))
            .collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in &local {
            x0 = x0.min(*x);
            y0 = y0.min(*y);
            x1 = x1.max(*x);
            y1 = y1.max(*y);
        }
        let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (ncx, ncy) = self.to_screen(mx, my);
        self.shape.points = local
            .iter()
            .map(|(x, y)| [((x - x0) / w) as f32, ((y - y0) / h) as f32])
            .collect();
        self.cx = ncx;
        self.cy = ncy;
        self.w = w;
        self.h = h;
    }

    /// Put a freeform corner at a screen point (index i), then refit the box.
    fn set_point(&mut self, i: usize, sx: f64, sy: f64) {
        let (lx, ly) = self.to_local(sx, sy);
        let f = [(lx / self.w.max(1.0) + 0.5) as f32, (ly / self.h.max(1.0) + 0.5) as f32];
        if i < self.shape.points.len() {
            self.shape.points[i] = f;
        } else {
            self.shape.points.push(f);
        }
        self.refit();
    }

    /// Window position -> screen position.
    fn screen_at(&self, wx: f32, wy: f32) -> (f64, f64) {
        let (ox, oy) = self.origin.get();
        let k = self.k.max(0.05) as f64;
        (
            self.shot.ox as f64 + (wx - ox) as f64 / k,
            self.shot.oy as f64 + (wy - oy) as f64 / k,
        )
    }

    /// How close (in screen points) counts as "on" a handle.
    fn tolerance(&self) -> f64 {
        14.0 / self.k.max(0.05) as f64
    }
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Distance from p to the segment a–b.
fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

fn inside_poly(poly: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    for i in 0..poly.len() {
        let (x1, y1) = poly[i];
        let (x2, y2) = poly[(i + 1) % poly.len()];
        if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
            inside = !inside;
        }
    }
    inside
}

/// Draw the shape editor's overlay: where clicks / taps land, the outline, the box
/// and its handles.
fn paint_shape_edit(ed: &ShapeEdit, bounds: Bounds<Pixels>, window: &mut Window) {
    let (bx, by) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let k = ed.k as f64;
    let (sox, soy) = (ed.shot.ox as f64, ed.shot.oy as f64);
    let at = |p: (f64, f64)| point(px(bx + ((p.0 - sox) * k) as f32), px(by + ((p.1 - soy) * k) as f32));
    let accent: Hsla = hsla(ACCENT);
    let white: Hsla = hsla(0xffffff);
    let shade: Hsla = rgba(0x0000_0099).into();
    let poly_ok = !ed.is_poly() || ed.shape.points.len() >= 3;

    // Where clicks / taps land.
    if poly_ok && !ed.drawing {
        let r = ed.rect();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut unit = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0
        };
        let (spots, size) = if ed.taps > 0 {
            (spread_taps(r, &ed.shape, ed.taps, &mut unit), 7.0f32)
        } else {
            ((0..700).map(|_| area_point(r, &ed.shape, &mut unit)).collect::<Vec<_>>(), 3.0f32)
        };
        let green: Hsla = hsla(GREEN);
        for (x, y) in spots {
            let p = at((x as f64, y as f64));
            if ed.taps > 0 {
                window.paint_quad(
                    fill(
                        Bounds { origin: point(p.x - px(size / 2.0 + 1.5), p.y - px(size / 2.0 + 1.5)), size: gpui::size(px(size + 3.0), px(size + 3.0)) },
                        shade,
                    )
                    .corner_radii(px(size)),
                );
            }
            window.paint_quad(
                fill(
                    Bounds { origin: point(p.x - px(size / 2.0), p.y - px(size / 2.0)), size: gpui::size(px(size), px(size)) },
                    green,
                )
                .corner_radii(px(size / 2.0)),
            );
        }
        // The outline.
        let outline: Vec<Point<Pixels>> = area_outline(r, &ed.shape).into_iter().map(at).collect();
        stroke_poly(window, &outline, 4.0, shade, true, false);
        stroke_poly(window, &outline, 2.0, accent, true, false);
    }

    // The box (dashed) and its handles.
    if !ed.drawing {
        let corners: Vec<Point<Pixels>> = (0..4).map(|i| at(ed.corner(i))).collect();
        stroke_poly(window, &corners, 1.0, rgba(0xffffffaa).into(), true, true);
        let top = at(ed.edge(0));
        let rh = at(ed.rot_handle());
        stroke_poly(window, &[top, rh], 1.5, white, false, false);
        let square = |window: &mut Window, p: Point<Pixels>| {
            window.paint_quad(fill(Bounds { origin: point(p.x - px(6.), p.y - px(6.)), size: gpui::size(px(12.), px(12.)) }, accent));
            window.paint_quad(fill(Bounds { origin: point(p.x - px(3.5), p.y - px(3.5)), size: gpui::size(px(7.), px(7.)) }, white));
        };
        for i in 0..4 {
            square(window, at(ed.corner(i)));
            square(window, at(ed.edge(i)));
        }
        window.paint_quad(
            fill(Bounds { origin: point(rh.x - px(8.), rh.y - px(8.)), size: gpui::size(px(16.), px(16.)) }, accent)
                .corner_radii(px(8.)),
        );
        window.paint_quad(
            fill(Bounds { origin: point(rh.x - px(4.), rh.y - px(4.)), size: gpui::size(px(8.), px(8.)) }, white)
                .corner_radii(px(4.)),
        );
    }

    // Freeform corners.
    if ed.is_poly() {
        let pts: Vec<Point<Pixels>> = ed.poly().into_iter().map(at).collect();
        if ed.drawing {
            stroke_poly(window, &pts, 4.0, shade, false, false);
            stroke_poly(window, &pts, 2.0, accent, false, false);
        }
        for (i, p) in pts.iter().enumerate() {
            let big = ed.drawing && i == 0 && pts.len() >= 3;
            let r = if big { 9.0 } else { 6.0 };
            window.paint_quad(
                fill(Bounds { origin: point(p.x - px(r + 1.5), p.y - px(r + 1.5)), size: gpui::size(px(2.0 * r + 3.0), px(2.0 * r + 3.0)) }, accent)
                    .corner_radii(px(r + 1.5)),
            );
            window.paint_quad(
                fill(Bounds { origin: point(p.x - px(r - 1.0), p.y - px(r - 1.0)), size: gpui::size(px(2.0 * r - 2.0), px(2.0 * r - 2.0)) }, white)
                    .corner_radii(px(r - 1.0)),
            );
        }
    }
}

/// What a step did the last time the macro ran (the tag under its card).
struct CardResult {
    text: String,
    ok: bool,
    /// How many times it ran.
    count: u32,
}

/// The quick-add box (Ctrl/⌘ + K, or letting go of a line on empty canvas).
struct QuickAdd {
    query: String,
    /// The highlighted match.
    sel: usize,
    /// Connect this output to the new step.
    link: Option<(u64, usize)>,
    /// Put the new step here (canvas coordinates) and show the box at `screen`.
    at: Option<(f32, f32)>,
    screen: Option<(f32, f32)>,
}

/// "Run anyway?" when the check before running found problems.
struct RunCheck {
    from: Option<u64>,
    stepping: bool,
    problems: Vec<Problem>,
}

/// What's wrong with one step (or, for a group, with the steps inside it).
#[derive(Default, Clone)]
struct StepProblems {
    serious: usize,
    minor: usize,
    texts: Vec<(bool, String)>,
    /// Problems with steps inside this group.
    inside: usize,
    inside_serious: bool,
}

/// What a Read text / Find image step saw when it last ran.
struct SeenShot {
    path: PathBuf,
    caption: String,
    /// It matched / found the image.
    ok: bool,
}

/// Text colour of a variable pill.
const VAR_TEXT: u32 = 0xc9bdff;
/// A variable that no step sets (probably a typo): amber.
const VAR_WARN: u32 = 0xf0b429;
/// The space a pill adds around its text: 1 px margin, 1 px border and 4 px padding
/// on each side (at 100% zoom). Used to work out where clicks land in a text box.
const PILL_EXTRA: f32 = 12.0;
/// The marker in front of a variable's name.
const VAR_MARK: &str = "#";

/// A variable shown as a pill: `# gold`. `known` = some step sets it (else amber);
/// `selected` = it's inside the selected text of a box being edited.
fn var_pill(name: &str, known: bool, selected: bool, z: f32) -> Div {
    let color = if known { ACCENT } else { VAR_WARN };
    let bg = if selected { (ACCENT << 8) | 0x99 } else { (color << 8) | 0x24 };
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .mx(px(1. * z))
        .px(px(4. * z))
        .rounded(px(4. * z))
        .border_1()
        .border_color(rgba((color << 8) | 0x55))
        .bg(rgba(bg))
        .text_color(rgb(if selected { 0xffffff } else if known { VAR_TEXT } else { VAR_WARN }))
        .child(div().text_color(rgba((color << 8) | 0xcc)).child(VAR_MARK))
        .child(name.to_string())
}

/// Text with its variables shown as pills (all on one line, cut off if too long).
fn spans_view(spans: &[Span], known: &dyn Fn(&str) -> bool, z: f32) -> Div {
    let mut row = div().flex().flex_row().items_center().whitespace_nowrap().overflow_hidden().min_w(px(0.));
    for sp in spans {
        row = match sp {
            Span::Text(t) if t.is_empty() => row,
            Span::Text(t) => row.child(div().flex_none().child(t.clone())),
            Span::Var(n) => row.child(var_pill(n, known(n), false, z)),
        };
    }
    row
}

/// A piece of the text box being edited: plain letters, or a `{name}` pill.
#[derive(Clone, Debug)]
enum EditSeg {
    Plain(usize, usize),
    Pill(usize, usize, String),
}

/// Split the text being edited into plain runs and pills. A pill the cursor is inside,
/// or that the selection only partly covers, is shown as its plain `{name}` text.
fn edit_segments(len: usize, pills: &[VarRange], cur: usize, sel: Option<(usize, usize)>) -> Vec<EditSeg> {
    let pills: Vec<&VarRange> = pills
        .iter()
        .filter(|r| !(cur > r.start && cur < r.end))
        .filter(|r| match sel {
            Some((a, b)) => (a <= r.start && b >= r.end) || b <= r.start || a >= r.end,
            None => true,
        })
        .collect();
    let mut out = Vec::new();
    let mut at = 0;
    for r in pills {
        if r.start > at {
            out.push(EditSeg::Plain(at, r.start));
        }
        out.push(EditSeg::Pill(r.start, r.end, r.name.clone()));
        at = r.end;
    }
    if at < len {
        out.push(EditSeg::Plain(at, len));
    }
    out
}

impl Field {
    /// How this box uses variables (None = it doesn't).
    fn var_mode(self) -> Option<VarMode> {
        match self {
            Field::Text | Field::ToastTitle | Field::ToastText | Field::CompareValue => Some(VarMode::Text),
            Field::VarExpr | Field::IfLeft | Field::IfRight => Some(VarMode::Expr),
            Field::VarName | Field::ReadSaveTo => Some(VarMode::Name),
            _ => None,
        }
    }

    /// The `{name}` pills in this box's text while it's being edited.
    fn edit_pills(self, text: &str) -> Vec<VarRange> {
        match self.var_mode() {
            Some(VarMode::Text) | Some(VarMode::Expr) => var_ranges(text, VarMode::Text, &|_| true),
            _ => Vec::new(),
        }
    }
}

/// Longer explanations are cut to this many characters' worth of lines (two lines in
/// the sidebar); hovering shows the whole text.
const HELP_SHORT: usize = 96;

/// A quiet explanation under a setting. Short ones show in full. Long ones show
/// their first two lines with an ⓘ, and the full text appears on hover, so the
/// panel stays readable without losing anything.
fn help(text: &str) -> Stateful<Div> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    let id = ElementId::Name(SharedString::from(format!("help-{:x}", h.finish())));
    let long = text.chars().count() > HELP_SHORT;
    let base = div()
        .id(id)
        .text_xs()
        .line_height(px(17.))
        .text_color(rgb(UI_FAINT));
    if !long {
        return base.child(text.to_string());
    }
    base.flex()
        .flex_row()
        .items_start()
        .gap_1p5()
        .hover(|s| s.text_color(rgb(MUTED)))
        .child(div().flex_1().min_w(px(0.)).line_clamp(2).text_ellipsis().child(text.to_string()))
        .child(
            div()
                .flex_none()
                .mt(px(1.))
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(rgb(UI_EDGE))
                .text_size(px(9.))
                .text_color(rgb(MUTED))
                .child("i"),
        )
        .tooltip(tip_view(text.to_string()))
}

/// One line in the Tips list.
fn tip(text: &'static str) -> Div {
    div()
        .flex()
        .flex_row()
        .gap_2()
        .text_xs()
        .line_height(px(17.))
        .text_color(rgb(MUTED))
        .child(div().flex_none().text_color(rgb(UI_FAINT)).child("•"))
        .child(div().flex_1().min_w(px(0.)).child(text))
}

fn divider() -> Div {
    div().flex_none().w(px(1.)).h(px(18.)).bg(rgb(UI_LINE)).mx_1p5()
}

#[cfg(target_os = "macos")]
const SHORTCUT_KEY: &str = "⌘";
#[cfg(not(target_os = "macos"))]
const SHORTCUT_KEY: &str = "Ctrl+";


/// Bring MacroFlow back (un-minimise it) after selecting an area.
fn show_again(window: &mut Window, cx: &mut Context<MacroApp>) {
    window.activate_window();
    if IS_MACOS {
        cx.activate(true);
    }
}

#[cfg(target_os = "macos")]
const WINDOW_LIST_NOTE: &str = "★ = the app you were using. Window titles need Screen Recording permission. Minimised windows and windows on other Spaces aren't listed.";
#[cfg(not(target_os = "macos"))]
const WINDOW_LIST_NOTE: &str = "★ = the window you were using. Minimised windows aren't listed.";

// ---------------------------------------------------------------------------
// Title bar and sidebar pieces
// ---------------------------------------------------------------------------

fn step_group(k: &NodeKind) -> usize {
    match k {
        NodeKind::Click { .. } | NodeKind::MoveMouse { .. } | NodeKind::Drag { .. } | NodeKind::Scroll { .. } => 0,
        NodeKind::KeyPress { .. } | NodeKind::TypeText { .. } => 1,
        NodeKind::FindImage { .. } | NodeKind::WindowCheck { .. } | NodeKind::ReadText { .. } => 2,
        NodeKind::Recording { .. } => 4,
        NodeKind::Notify { .. } => 5,
        NodeKind::SetVar { .. } | NodeKind::IfCompare { .. } => 6,
        NodeKind::PixelColor { .. } => 2,
        _ => 3,
    }
}

fn step_blurb(k: &NodeKind) -> &'static str {
    match k {
        NodeKind::Start => "Where every run begins",
        NodeKind::Click { .. } => "Click at a spot or on a found image",
        NodeKind::MoveMouse { .. } => "Move the pointer somewhere",
        NodeKind::Drag { .. } => "Hold at one spot, let go at another",
        NodeKind::Scroll { .. } => "Scroll up, down or sideways",
        NodeKind::KeyPress { .. } => "Press a key or shortcut",
        NodeKind::TypeText { .. } => "Type some text",
        NodeKind::FindImage { until_gone: true, .. } => "Wait until an image disappears",
        NodeKind::FindImage { .. } => "Branch on an image on screen",
        NodeKind::WindowCheck { .. } => "Branch on a window's title or app",
        NodeKind::ReadText { .. } => "Read text or a number and compare",
        NodeKind::Wait { .. } => "Pause, with optional randomness",
        NodeKind::Loop { .. } => "Repeat a branch N times",
        NodeKind::End => "Stop the macro",
        NodeKind::Group { .. } => "An empty group to fill with steps",
        NodeKind::GroupIn => "Where a group starts",
        NodeKind::GroupOut => "Leave the group and carry on",
        NodeKind::Recording { .. } => "Empty recording to fill or edit",
        NodeKind::Notify { .. } => "Show a pop-up at the top of the screen",
        NodeKind::SetVar { .. } => "Store a number, text or sum",
        NodeKind::IfCompare { .. } => "Branch on a comparison",
        NodeKind::PixelColor { .. } => "Branch on the colour of one spot",
    }
}

/// A small accent tile with a flowchart mark, shown before the app name.
fn logo_mark() -> Div {
    div()
        .size(px(20.))
        .flex_none()
        .relative()
        .rounded_md()
        .bg(rgb(ACCENT))
        .child(
            div()
                .absolute()
                .left(px(4.))
                .top(px(4.))
                .size(px(5.))
                .rounded_sm()
                .bg(rgb(0xffffff)),
        )
        .child(
            div()
                .absolute()
                .left(px(11.))
                .top(px(11.))
                .size(px(5.))
                .rounded_sm()
                .bg(rgba(0xffffffcc)),
        )
        .child(
            div()
                .absolute()
                .left(px(6.))
                .top(px(9.))
                .w(px(1.5))
                .h(px(4.5))
                .bg(rgba(0xffffffcc)),
        )
        .child(
            div()
                .absolute()
                .left(px(6.))
                .top(px(13.))
                .w(px(5.))
                .h(px(1.5))
                .bg(rgba(0xffffffcc)),
        )
}

/// "Show/hide panel" icon: a window outline with a left column.
fn sidebar_icon(open: bool) -> Div {
    div()
        .relative()
        .w(px(16.))
        .h(px(13.))
        .rounded_sm()
        .border_1()
        .border_color(rgb(if open { TEXT } else { MUTED }))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .w(px(5.))
                .h_full()
                .when(open, |d| d.bg(rgb(TEXT)))
                .when(!open, |d| d.border_r_1().border_color(rgb(MUTED))),
        )
}

/// A button for the floating selection bar.
fn bar_btn(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    let (bg, hover, fg) = if primary {
        (GROUP_COLOR, 0x8aa3ff, 0x0b1020)
    } else {
        (BTN, BTN_HOVER, TEXT)
    };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .px_2p5()
        .h(px(26.))
        .rounded_md()
        .bg(rgb(bg))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(fg))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(hover)))
        .child(label)
}

/// A quiet, borderless title bar button.
fn ghost_btn(id: impl Into<ElementId>, label: &'static str, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .px_2()
        .h(px(28.))
        .rounded_md()
        .whitespace_nowrap()
        .text_color(rgb(MUTED))
        .when(enabled, |d| d.hover(|s| s.bg(rgb(UI_HOVER)).text_color(rgb(TEXT))).cursor_pointer())
        .when(!enabled, |d| d.opacity(0.35))
        .child(label)
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterSave {
    CloseWindow,
    Quit,
}

fn safe_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "macro".into()
    } else {
        s
    }
}

/// Maximise the window, or restore it if it's already maximised.
#[cfg(target_os = "windows")]
fn toggle_maximize(window: &mut Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsZoomed, ShowWindowAsync, SW_MAXIMIZE, SW_RESTORE};
    // MacroFlow's own window, not whichever window happens to be in front.
    let Ok(handle) = HasWindowHandle::window_handle(&*window) else {
        return;
    };
    let RawWindowHandle::Win32(h) = handle.as_raw() else {
        return;
    };
    let hwnd = h.hwnd.get() as *mut core::ffi::c_void;
    // Asynchronous on purpose: a direct resize while MacroFlow is still handling the
    // click is missed by GPUI, so the window changed size but the app didn't follow.
    unsafe {
        let cmd = if IsZoomed(hwnd) != 0 { SW_RESTORE } else { SW_MAXIMIZE };
        ShowWindowAsync(hwnd, cmd);
    }
}

#[cfg(not(target_os = "windows"))]
fn toggle_maximize(window: &mut Window) {
    window.zoom_window();
}

/// A new, empty macro. Its positions scale with the screen size, so it works the
/// same on another monitor or computer.
fn fresh_macro() -> Macro {
    let mut m = Macro::default();
    m.reference = vision::primary_screen().ok();
    m
}

// ---------------------------------------------------------------------------
// Editing text in a box
// ---------------------------------------------------------------------------

/// The text in the box being edited, its cursor and selection (all in characters).
struct TextEdit {
    chars: Vec<char>,
    cur: usize,
    /// The other end of the selection; no selection when it's None or equals `cur`.
    anchor: Option<usize>,
}

impl TextEdit {
    fn new(text: &str, cur: usize, anchor: Option<usize>) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        TextEdit {
            cur: cur.min(n),
            anchor: anchor.map(|a| a.min(n)),
            chars,
        }
    }

    fn text(&self) -> String {
        self.chars.iter().collect()
    }

    fn selection(&self) -> Option<(usize, usize)> {
        match self.anchor {
            Some(a) if a != self.cur => Some((a.min(self.cur), a.max(self.cur))),
            _ => None,
        }
    }

    fn selected_text(&self) -> String {
        self.selection()
            .map(|(a, b)| self.chars[a..b].iter().collect())
            .unwrap_or_default()
    }

    /// Move the cursor; with `extend` (Shift held) the selection grows or shrinks.
    fn move_to(&mut self, to: usize, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cur);
            }
        } else {
            self.anchor = None;
        }
        self.cur = to.min(self.chars.len());
    }

    fn delete(&mut self, from: usize, to: usize) {
        let (a, b) = (from.min(to), from.max(to).min(self.chars.len()));
        if a < b {
            self.chars.drain(a..b);
        }
        self.cur = a;
        self.anchor = None;
    }

    fn delete_selection(&mut self) -> bool {
        match self.selection() {
            Some((a, b)) => {
                self.delete(a, b);
                true
            }
            None => false,
        }
    }

    /// Type or paste text, replacing the selection.
    fn insert(&mut self, text: &str) {
        self.delete_selection();
        let new: Vec<char> = text.chars().collect();
        let n = new.len();
        self.chars.splice(self.cur..self.cur, new);
        self.cur += n;
        self.anchor = None;
    }
}

/// Where Ctrl / ⌥ + ← lands: the start of the previous word.
fn prev_word(chars: &[char], from: usize) -> usize {
    let mut i = from.min(chars.len());
    while i > 0 && !chars[i - 1].is_alphanumeric() {
        i -= 1;
    }
    while i > 0 && chars[i - 1].is_alphanumeric() {
        i -= 1;
    }
    i
}

/// Where Ctrl / ⌥ + → lands: the end of the next word.
fn next_word(chars: &[char], from: usize) -> usize {
    let n = chars.len();
    let mut i = from.min(n);
    while i < n && !chars[i].is_alphanumeric() {
        i += 1;
    }
    while i < n && chars[i].is_alphanumeric() {
        i += 1;
    }
    i
}
