//! The MacroFlow window: toolbar, step palette, flowchart canvas and inspector.

use crate::engine::{self, AreaPurpose, WorkerMsg};
use crate::keys;
use crate::model::*;
use crate::vision;
use gpui::{
    canvas, div, fill, img, point, prelude::*, px, rgb, rgba, AnyElement, Bounds, ClickEvent,
    Context, Div, ElementId, FocusHandle, FontWeight, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, PathBuilder, PathPromptOptions,
    Pixels, Point, PromptLevel, ScrollDelta, ScrollWheelEvent, SharedString, Stateful, Task,
    Window, WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowKind, WindowOptions,
};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

const BG: u32 = 0x0f1117;
const PANEL: u32 = 0x161922;
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
const TITLEBAR: u32 = 0x13151d;

/// Height of the custom title bar (the system one is transparent, so we draw our own).
pub const TITLEBAR_H: f32 = 40.0;
const IS_MACOS: bool = cfg!(target_os = "macos");
/// Room for the macOS traffic lights before the app name.
const NAME_INSET: f32 = if IS_MACOS { 80.0 } else { 14.0 };
const SIDEBAR_W: f32 = 320.0;

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
}

/// Which page the left sidebar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideTab {
    Steps,
    Edit,
    Macro,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl Field {
    fn is_text(self) -> bool {
        matches!(
            self,
            Field::MacroName | Field::Note | Field::Text | Field::CompareValue | Field::WinText
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
    drag: Drag,
    mouse: (f32, f32),
    editing: Option<(Field, String)>,
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
    windows: Vec<vision::WinInfo>,
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
    stop: Arc<AtomicBool>,
    _poll: Task<()>,
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
        MacroApp {
            focus,
            doc: Macro::default(),
            path: None,
            dirty: false,
            selected: None,
            pan: (0.0, 0.0),
            canvas_origin: Rc::new(Cell::new((0.0, 0.0))),
            drag: Drag::None,
            mouse: (0.0, 0.0),
            editing: None,
            capturing_key: false,
            mode: Mode::Idle,
            status: "Welcome! Add steps from the left, or record a macro. Drag from a step's right-hand dot to connect it.".into(),
            active_node: None,
            record_moves: true,
            bulk_delay: 100,
            snip_size: 64,
            pending_area: None,
            close_confirmed: false,
            close_prompt_open: false,
            side_tab: SideTab::Steps,
            sidebar_open: true,
            windows: Vec::new(),
            tx,
            rx,
            stop: Arc::new(AtomicBool::new(false)),
            _poll: poll,
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
                WorkerMsg::RunFinished(s) => {
                    self.mode = Mode::Idle;
                    self.active_node = None;
                    self.status = s;
                }
                WorkerMsg::Recorded(events) => {
                    self.mode = Mode::Idle;
                    if events.is_empty() {
                        self.status = "Nothing was recorded.".into();
                    } else {
                        let n = events.len();
                        self.add_step(NodeKind::Recording { events, speed: 1.0 });
                        self.status = format!("Recorded {n} events. Edit delays in the panel on the right.");
                    }
                }
                WorkerMsg::StepRecorded(kind) => {
                    self.mode = Mode::Idle;
                    let title = kind.title();
                    self.add_step(kind);
                    self.status = format!("Added a {title} step.");
                }
                WorkerMsg::Position { x, y } => {
                    if let Mode::Picking(id) = self.mode {
                        if let Some(n) = self.doc.node_mut(id) {
                            match &mut n.kind {
                                NodeKind::Click { target, x: nx, y: ny, .. }
                                | NodeKind::MoveMouse { target, x: nx, y: ny, .. } => {
                                    *target = Target::Fixed;
                                    *nx = x;
                                    *ny = y;
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
                WorkerMsg::TestResult(s) => {
                    self.mode = Mode::Idle;
                    self.status = s;
                }
                WorkerMsg::AreaShot(shot) => match self.pending_area.take() {
                    Some(purpose) => {
                        if let Err(e) = open_area_overlay(shot, purpose, self.tx.clone(), cx) {
                            self.mode = Mode::Idle;
                            self.status = e;
                            show_again(window, cx);
                        } else {
                            self.status = "Drag a box anywhere on your screen…".into();
                        }
                    }
                    None => {
                        self.mode = Mode::Idle;
                        let _ = std::fs::remove_file(&shot.path);
                        show_again(window, cx);
                    }
                },
                WorkerMsg::AreaPicked { purpose, shot, sel } => {
                    self.mode = Mode::Idle;
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
                            self.update_node(id, |k| {
                                if let NodeKind::ReadText { preview, .. } = k {
                                    *preview = Some(path.clone());
                                }
                            });
                        }
                        AreaPurpose::Template(id) => {
                            self.update_node(id, |k| {
                                if let NodeKind::FindImage { template, .. } = k {
                                    *template = Some(path.clone());
                                }
                            });
                            self.status = format!("Saved {}. Use “Test now” to check it.", path.display());
                        }
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
                    if self.pending_area.take().is_some() {
                        show_again(window, cx);
                    }
                }
            }
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
        if self.begin_worker(Mode::Running) {
            self.status = "Starting…".into();
            engine::run_macro(self.doc.clone(), self.tx.clone(), self.stop.clone());
        }
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

    fn start_test(&mut self, id: u64) {
        let Some(NodeKind::FindImage { template, threshold, .. }) =
            self.doc.node(id).map(|n| n.kind.clone())
        else {
            return;
        };
        let Some(path) = template else {
            self.status = "Snip or choose an image first.".into();
            return;
        };
        if self.begin_worker(Mode::Testing) {
            self.status = "Searching the screen…".into();
            engine::test_find(self.tx.clone(), path, threshold);
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
        window.minimize_window();
        engine::capture_for_area(self.tx.clone(), self.stop.clone(), countdown);
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
        let (gx, gy) = (shot.ox + x0 as i32, shot.oy + y0 as i32);
        match purpose {
            AreaPurpose::ReadText(id) => {
                self.update_node(id, |k| {
                    if let NodeKind::ReadText { x, y, w: nw, h: nh, .. } = k {
                        *x = gx;
                        *y = gy;
                        *nw = w as i32;
                        *nh = h as i32;
                    }
                });
                self.status = format!("Area set to {w}×{h} at ({gx}, {gy}). Press “Read now” to test it.");
            }
            AreaPurpose::Template(_) => self.status = "Saving the picture…".into(),
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

    fn start_read_test(&mut self, id: u64) {
        self.commit_edit();
        let Some(NodeKind::ReadText {
            x,
            y,
            w,
            h,
            compare,
            value,
            case_sensitive,
            ..
        }) = self.doc.node(id).map(|n| n.kind.clone())
        else {
            return;
        };
        if w < 2 || h < 2 {
            self.status = "Select an area on screen first.".into();
            return;
        }
        if self.begin_worker(Mode::Working) {
            self.status = "Reading the screen…".into();
            engine::test_read(self.tx.clone(), (x, y, w, h), compare, value, case_sensitive);
        }
    }

    fn start_list_windows(&mut self) {
        if self.begin_worker(Mode::Working) {
            self.status = "Listing open windows…".into();
            engine::list_windows(self.tx.clone());
        }
    }

    fn stop_worker(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.status = "Stopping…".into();
    }

    /// Add a step. If a step is selected and its main output is free,
    /// the new step is placed after it and connected automatically.
    fn add_step(&mut self, kind: NodeKind) {
        self.commit_edit();
        let link = self
            .selected
            .and_then(|id| self.doc.node(id))
            .filter(|n| !n.kind.outputs().is_empty())
            .map(|n| (n.id, n.x + NODE_W + 70.0, n.y));
        let (x, y, from) = match link {
            Some((id, x, y)) => (x, y, Some(id)),
            None => {
                let k = (self.doc.nodes.len() % 6) as f32;
                (260.0 - self.pan.0 + k * 26.0, 120.0 - self.pan.1 + k * 26.0, None)
            }
        };
        let new_id = self.doc.add_node(kind, x, y);
        if let Some(from) = from {
            if self.doc.next(from, 0).is_none() {
                self.doc.connect(from, 0, new_id);
            }
        }
        self.selected = Some(new_id);
        self.dirty = true;
    }

    fn delete_selected(&mut self) {
        let Some(id) = self.selected else { return };
        if matches!(self.doc.node(id).map(|n| &n.kind), Some(NodeKind::Start)) {
            self.status = "The Start step can't be deleted.".into();
            return;
        }
        self.doc.remove_node(id);
        self.selected = None;
        self.dirty = true;
    }

    fn duplicate_selected(&mut self) {
        let Some(n) = self.selected.and_then(|id| self.doc.node(id)).cloned() else {
            return;
        };
        if matches!(n.kind, NodeKind::Start) {
            return;
        }
        let id = self.doc.add_node(n.kind, n.x + 30.0, n.y + 30.0);
        if let Some(copy) = self.doc.node_mut(id) {
            copy.delay_ms = n.delay_ms;
            copy.note = n.note;
        }
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
        self.doc = Macro::default();
        self.path = None;
        self.dirty = false;
        self.selected = None;
        self.editing = None;
        self.pan = (0.0, 0.0);
        self.status = "New macro.".into();
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
                self.doc = doc;
                self.path = Some(path.clone());
                self.dirty = false;
                self.selected = None;
                self.editing = None;
                self.pan = (0.0, 0.0);
                self.status = format!("Opened {}", path.display());
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
        self.editing = Some((field, self.get_field(field)));
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
            _ => {}
        }
        let Some(n) = self.selected.and_then(|id| d.node(id)) else {
            return String::new();
        };
        match (f, &n.kind) {
            (Field::Note, _) => n.note.clone(),
            (Field::Delay, _) => n.delay_ms.to_string(),
            (
                Field::X,
                NodeKind::Click { x, .. } | NodeKind::MoveMouse { x, .. } | NodeKind::ReadText { x, .. },
            ) => x.to_string(),
            (
                Field::Y,
                NodeKind::Click { y, .. } | NodeKind::MoveMouse { y, .. } | NodeKind::ReadText { y, .. },
            ) => y.to_string(),
            (Field::W, NodeKind::ReadText { w, .. }) => w.to_string(),
            (Field::H, NodeKind::ReadText { h, .. }) => h.to_string(),
            (Field::CompareValue, NodeKind::ReadText { value, .. }) => value.clone(),
            (Field::WinText, NodeKind::WindowCheck { text, .. }) => text.clone(),
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
                | NodeKind::WindowCheck { timeout_ms, .. },
            ) => timeout_ms.to_string(),
            (
                Field::Interval,
                NodeKind::FindImage { interval_ms, .. }
                | NodeKind::ReadText { interval_ms, .. }
                | NodeKind::WindowCheck { interval_ms, .. },
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
            _ => {}
        }
        let Some(id) = self.selected else { return Ok(()) };
        let Some(n) = self.doc.node_mut(id) else { return Ok(()) };
        match (f, &mut n.kind) {
            (Field::Note, _) => n.note = raw.to_string(),
            (Field::Delay, _) => n.delay_ms = int()?.max(0) as u64,
            (
                Field::X,
                NodeKind::Click { x, .. } | NodeKind::MoveMouse { x, .. } | NodeKind::ReadText { x, .. },
            ) => *x = int()? as i32,
            (
                Field::Y,
                NodeKind::Click { y, .. } | NodeKind::MoveMouse { y, .. } | NodeKind::ReadText { y, .. },
            ) => *y = int()? as i32,
            (Field::W, NodeKind::ReadText { w, .. }) => *w = int()?.clamp(0, 10_000) as i32,
            (Field::H, NodeKind::ReadText { h, .. }) => *h = int()?.clamp(0, 10_000) as i32,
            (Field::CompareValue, NodeKind::ReadText { value, .. }) => *value = raw.to_string(),
            (Field::WinText, NodeKind::WindowCheck { text, .. }) => *text = raw.to_string(),
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
                | NodeKind::WindowCheck { timeout_ms, .. },
            ) => *timeout_ms = int()?.max(0) as u64,
            (
                Field::Interval,
                NodeKind::FindImage { interval_ms, .. }
                | NodeKind::ReadText { interval_ms, .. }
                | NodeKind::WindowCheck { interval_ms, .. },
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

    fn to_local(&self, p: Point<Pixels>) -> (f32, f32) {
        let (ox, oy) = self.canvas_origin.get();
        (
            f32::from(p.x) - ox - self.pan.0,
            f32::from(p.y) - oy - self.pan.1,
        )
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let m = ks.modifiers;

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

        if let Some((f, mut buf)) = self.editing.take() {
            cx.stop_propagation();
            match key {
                "enter" | "return" | "tab" => {
                    self.editing = Some((f, buf));
                    self.commit_edit();
                }
                "escape" => {}
                "backspace" => {
                    if m.secondary() || m.alt {
                        buf.clear();
                    } else {
                        buf.pop();
                    }
                    self.editing = Some((f, buf));
                }
                _ => {
                    // Ctrl+Alt is AltGr on Windows, which types characters like @ and €.
                    let shortcut = if cfg!(target_os = "macos") {
                        m.platform || m.control
                    } else {
                        m.control && !m.alt
                    };
                    if m.secondary() && key == "v" {
                        if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            buf.push_str(&t.replace(['\n', '\r'], " "));
                        }
                    } else if !shortcut {
                        match &ks.key_char {
                            Some(ch) => buf.push_str(ch),
                            None if key == "space" => buf.push(' '),
                            None => {}
                        }
                    }
                    self.editing = Some((f, buf));
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
                "r" => self.start_run(),
                "b" => self.sidebar_open = !self.sidebar_open,
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        match key {
            "backspace" | "delete" => self.delete_selected(),
            "escape" => {
                self.selected = None;
                self.drag = Drag::None;
            }
            _ => return,
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let (wx, wy) = (f32::from(ev.position.x), f32::from(ev.position.y));
        self.mouse = (wx, wy);
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
                if let Some(n) = self.doc.node_mut(id) {
                    n.x = (lx - dx).round();
                    n.y = (ly - dy).round();
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
            Drag::Connect { .. } => cx.notify(),
            Drag::None => {}
        }
    }

    fn node_mouse_down(&mut self, id: u64, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.commit_edit();
        self.capturing_key = false;
        if self.selected != Some(id) {
            // Show the clicked step's settings.
            self.side_tab = SideTab::Edit;
            self.sidebar_open = true;
        }
        self.selected = Some(id);
        let (lx, ly) = self.to_local(ev.position);
        if let Some(n) = self.doc.node(id) {
            self.drag = Drag::Node {
                id,
                dx: lx - n.x,
                dy: ly - n.y,
            };
        }
        // Bring the step to the front.
        if let Some(i) = self.doc.nodes.iter().position(|n| n.id == id) {
            let n = self.doc.nodes.remove(i);
            self.doc.nodes.push(n);
        }
        self.focus.focus(window);
        cx.notify();
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

        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .w_full()
            .h(px(TITLEBAR_H))
            .bg(rgb(TITLEBAR))
            .border_b_1()
            .border_color(rgb(BORDER))
            // App name, top left (beside the traffic lights on macOS).
            .child(
                div()
                    .h_full()
                    .flex_none()
                    .pl(px(NAME_INSET))
                    .pr_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .window_control_area(WindowControlArea::Drag)
                    .child(logo_mark())
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
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
                    .hover(|s| s.bg(rgb(BTN_HOVER)))
                    .child(sidebar_icon(sidebar_open))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.sidebar_open = !this.sidebar_open;
                        cx.notify();
                    })),
            )
            .child(divider())
            .child(
                ghost_btn("tb-new", "New", idle).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.new_doc();
                    cx.notify();
                })),
            )
            .child(
                ghost_btn("tb-open", "Open", idle)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open(cx))),
            )
            .child(
                ghost_btn("tb-save", "Save", idle).on_click(cx.listener(
                    |this, ev: &ClickEvent, _, cx| {
                        this.save(ev.modifiers().shift, cx);
                        cx.notify();
                    },
                )),
            )
            .child(divider())
            // Current macro name (click to rename it in the Macro tab).
            .child(
                div()
                    .id("tb-name")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .h(px(26.))
                    .max_w(px(260.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(BTN_HOVER)))
                    .child(div().text_color(rgb(MUTED)).truncate().child(name))
                    .when(dirty, |d| {
                        d.child(div().flex_none().size(px(6.)).rounded_full().bg(rgb(YELLOW)))
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.selected = None;
                        this.side_tab = SideTab::Macro;
                        this.sidebar_open = true;
                        cx.notify();
                    })),
            )
            // Empty space you can drag to move the window.
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                chip("tb-moves", "Mouse moves", self.record_moves).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.record_moves = !this.record_moves;
                        cx.notify();
                    },
                )),
            )
            .child(div().w(px(6.)))
            .child(
                ghost_btn("tb-rec", "● Record", idle)
                    .text_color(rgb(RED))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.start_record();
                        cx.notify();
                    })),
            )
            .child(
                ghost_btn("tb-step", "◎ Record step", idle).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.start_record_step();
                        cx.notify();
                    },
                )),
            )
            .child(divider())
            .when(busy, |d| {
                d.child(
                    tool_btn_colored("tb-stop", "■ Stop", true, RED, 0xf47373)
                        .h(px(26.))
                        .text_color(rgb(0xffffff))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.stop_worker();
                            cx.notify();
                        })),
                )
            })
            .when(idle, |d| {
                d.child(
                    tool_btn_colored("tb-run", "▶ Run", true, GREEN, 0x45d98c)
                        .h(px(26.))
                        .text_color(rgb(0x0b1a10))
                        .font_weight(FontWeight::BOLD)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.start_run();
                            cx.notify();
                        })),
                )
            })
            .child(div().w(px(if IS_MACOS { 12. } else { 8. })))
            // Windows draws no title bar for us, so add minimise / maximise / close.
            .when(!IS_MACOS, |d| d.child(window_controls(is_max)))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let items: Vec<AnyElement> = match self.side_tab {
            SideTab::Steps => self.steps_tab(cx),
            SideTab::Edit => match self.selected.and_then(|id| self.doc.node(id)).cloned() {
                Some(n) => self.node_settings(&n, cx),
                None => self.edit_empty(cx),
            },
            SideTab::Macro => self.macro_settings(cx),
        };
        let mut content = div()
            .id("sidebar-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3()
            .pb_4()
            .flex()
            .flex_col()
            .gap_2();
        for el in items {
            content = content.child(el);
        }
        div()
            .w(px(SIDEBAR_W))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(BORDER))
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
        ];
        let current = self.side_tab;
        div().flex_none().px_3().pt_3().pb_3().child(
            div()
                .flex()
                .gap_1()
                .p(px(3.))
                .rounded_lg()
                .bg(rgb(BG))
                .border_1()
                .border_color(rgb(BORDER))
                .children(tabs.into_iter().map(|(tab, label)| {
                    let active = tab == current;
                    div()
                        .id(("side-tab", tab as usize))
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_1p5()
                        .h(px(28.))
                        .rounded_md()
                        .cursor_pointer()
                        .when(active, |d| {
                            d.bg(rgb(BTN))
                                .text_color(rgb(TEXT))
                                .font_weight(FontWeight::SEMIBOLD)
                                .shadow_sm()
                        })
                        .when(!active, |d| {
                            d.text_color(rgb(MUTED)).hover(|s| s.text_color(rgb(TEXT)).bg(rgb(PANEL2)))
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
        let hint = match self.selected.and_then(|id| self.doc.node(id)) {
            Some(n) if !n.kind.outputs().is_empty() => {
                format!("New steps attach after the selected “{}” step.", n.kind.title())
            }
            _ => "Select a step on the canvas to chain new steps after it.".to_string(),
        };
        out.push(
            div()
                .px_2()
                .py_1p5()
                .rounded_md()
                .bg(rgba((ACCENT << 8) | 0x1a))
                .border_1()
                .border_color(rgba((ACCENT << 8) | 0x40))
                .text_xs()
                .text_color(rgb(TEXT))
                .child(hint)
                .into_any_element(),
        );

        let groups: [(&'static str, usize); 5] = [
            ("Mouse", 0),
            ("Keyboard", 1),
            ("Smart checks", 2),
            ("Flow & timing", 3),
            ("Recording", 4),
        ];
        let palette: Vec<(usize, NodeKind)> = NodeKind::palette().into_iter().enumerate().collect();
        for (title, g) in groups {
            out.push(div().h(px(2.)).into_any_element());
            out.push(section_title(title).into_any_element());
            for (i, kind) in palette.iter().filter(|(_, k)| step_group(k) == g) {
                let i = *i;
                let kind = kind.clone();
                let color = kind.color();
                out.push(
                    div()
                        .id(("pal", i))
                        .group(SharedString::from(format!("pal-{i}")))
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_2()
                        .py_1p5()
                        .rounded_lg()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(PANEL2)))
                        .child(
                            div()
                                .size(px(30.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_md()
                                .bg(rgba((color << 8) | 0x24))
                                .border_1()
                                .border_color(rgba((color << 8) | 0x55))
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
                                .child(div().text_color(rgb(TEXT)).child(kind.title()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(MUTED))
                                        .truncate()
                                        .child(step_blurb(&kind)),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px_1p5()
                                .rounded_sm()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .invisible()
                                .group_hover(SharedString::from(format!("pal-{i}")), |s| {
                                    s.visible().text_color(rgb(TEXT)).bg(rgb(BTN_HOVER))
                                })
                                .child("Add"),
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
        }

        out.push(div().h(px(10.)).into_any_element());
        out.push(section_title("Tips").into_any_element());
        for t in [
            "Drag from a dot on the right of a step to another step to connect them.",
            "Right-click a dot, or click the × on a line, to remove a connection.",
            "Drag the empty grid (or scroll) to move around.",
            "Esc stops a recording or a running macro from anywhere.",
            "“Select area” minimises MacroFlow so you can drag a box anywhere on your screen. Shift-click it for a 3 s countdown first.",
            SHORTCUT_TIP,
        ] {
            out.push(tip(t).into_any_element());
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
            .border_color(rgb(BORDER))
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

    fn render_statusbar(&self) -> Div {
        let (badge, color) = match self.mode {
            Mode::Idle => ("READY", MUTED),
            Mode::Running => ("RUNNING", GREEN),
            Mode::Recording => ("● REC", RED),
            Mode::RecordingStep => ("◎ STEP", RED),
            Mode::Picking(_) => ("PICK", YELLOW),
            Mode::Snipping(_) => ("SNIP", YELLOW),
            Mode::Testing => ("SEARCH", YELLOW),
            Mode::Capturing => ("CAPTURE", YELLOW),
            Mode::Working => ("BUSY", YELLOW),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .px_3()
            .h(px(30.))
            .bg(rgb(PANEL))
            .border_t_1()
            .border_color(rgb(BORDER))
            .text_xs()
            .child(
                div()
                    .px_2()
                    .rounded_sm()
                    .bg(rgba((color << 8) | 0x33))
                    .text_color(rgb(color))
                    .font_weight(FontWeight::BOLD)
                    .child(badge),
            )
            .child(div().flex_1().truncate().text_color(rgb(TEXT)).child(self.status.clone()))
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .child(format!("{} steps · {} links", self.doc.nodes.len(), self.doc.edges.len())),
            )
    }

    // -----------------------------------------------------------------------
    // Rendering: flowchart
    // -----------------------------------------------------------------------

    fn render_graph(&self, cx: &mut Context<Self>) -> Div {
        let (pan_x, pan_y) = self.pan;
        let origin = self.canvas_origin.clone();
        let (ox, oy) = origin.get();

        // Connection geometry, in canvas-local coordinates.
        let mut lines: Vec<(f32, f32, f32, f32, Hsla, bool)> = Vec::new();
        let mut handles: Vec<(f32, f32, u64, usize)> = Vec::new();
        for e in &self.doc.edges {
            let (Some(a), Some(b)) = (self.doc.node(e.from), self.doc.node(e.to)) else {
                continue;
            };
            let (x0, y0) = out_port_pos(a, e.port);
            let (x1, y1) = in_port_pos(b);
            let (x0, y0, x1, y1) = (x0 + pan_x, y0 + pan_y, x1 + pan_x, y1 + pan_y);
            let active = self.active_node == Some(e.from);
            lines.push((x0, y0, x1, y1, hsla(a.kind.color()), active));
            let (mx, my) = bezier_mid(x0, y0, x1, y1);
            handles.push((mx, my, e.from, e.port));
        }
        let pending = match self.drag {
            Drag::Connect { from, port } => self.doc.node(from).map(|n| {
                let (x0, y0) = out_port_pos(n, port);
                (x0 + pan_x, y0 + pan_y, self.mouse.0 - ox, self.mouse.1 - oy)
            }),
            _ => None,
        };

        let painter = canvas(
            move |bounds, _, _| {
                origin.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y)));
            },
            move |bounds, _, window, _| {
                let bx = f32::from(bounds.origin.x);
                let by = f32::from(bounds.origin.y);
                let bw = f32::from(bounds.size.width);
                let bh = f32::from(bounds.size.height);
                let grid = hsla(GRID);
                let step = 24.0;
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
                    stroke_curve(window, bx + x0, by + y0, bx + x1, by + y1, width, color, false);
                }
                if let Some((x0, y0, x1, y1)) = pending {
                    stroke_curve(window, bx + x0, by + y0, bx + x1, by + y1, 2.0, hsla(ACCENT), true);
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
                    this.commit_edit();
                    this.capturing_key = false;
                    this.selected = None;
                    this.drag = Drag::Pan {
                        x: f32::from(ev.position.x),
                        y: f32::from(ev.position.y),
                    };
                    this.focus.focus(window);
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let (dx, dy) = match ev.delta {
                    ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                    ScrollDelta::Lines(l) => (l.x * 24.0, l.y * 24.0),
                };
                this.pan.0 += dx;
                this.pan.1 += dy;
                cx.notify();
            }))
            .child(painter);

        for n in &self.doc.nodes {
            graph = graph.child(self.render_node(n, pan_x, pan_y, cx));
        }

        for (mx, my, from, port) in handles {
            graph = graph.child(
                div()
                    .absolute()
                    .left(px(mx - 8.))
                    .top(px(my - 8.))
                    .size(px(16.))
                    .rounded_full()
                    .bg(rgb(PANEL2))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
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

        if self.doc.nodes.len() <= 1 {
            graph = graph.child(
                div()
                    .absolute()
                    .bottom(px(18.))
                    .left(px(18.))
                    .max_w(px(460.))
                    .p_3()
                    .rounded_lg()
                    .bg(rgba(0x1d212cdd))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .text_color(rgb(MUTED))
                    .child("Build a macro like a flowchart: add steps from the left (they chain after the selected step), record a whole macro with ● Record macro, or capture one click or key with ◎ Record one step. Find image steps branch into “found” and “not found”."),
            );
        }
        graph
    }

    fn render_node(&self, n: &Node, pan_x: f32, pan_y: f32, cx: &mut Context<Self>) -> Div {
        let id = n.id;
        let selected = self.selected == Some(id);
        let active = self.active_node == Some(id);
        let color = n.kind.color();
        let border = if active {
            rgb(YELLOW)
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

        let mut el = div()
            .absolute()
            .left(px(n.x + pan_x))
            .top(px(n.y + pan_y))
            .w(px(NODE_W))
            .h(px(NODE_H))
            .flex()
            .flex_col()
            .bg(rgb(NODE_BG))
            .rounded_lg()
            .border_2()
            .border_color(border)
            .shadow_md()
            .cursor_move()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.node_mouse_down(id, ev, window, cx)
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
                    .gap_2()
                    .px_2()
                    .h(px(26.))
                    .flex_none()
                    .rounded_t_md()
                    .bg(rgba((color << 8) | 0x2e))
                    .child(div().text_color(rgb(color)).child(n.kind.icon()))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT))
                            .child(n.kind.title()),
                    )
                    .child(div().flex_1())
                    .when(n.delay_ms > 0, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(format!("+{} ms", n.delay_ms)),
                        )
                    }),
            )
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pr(px(if outputs.len() > 1 { 64. } else { 12. }))
                    .text_xs()
                    .text_color(rgb(TEXT))
                    .truncate()
                    .child(n.kind.summary()),
            )
            .child(
                div()
                    .px_2()
                    .pr(px(if outputs.len() > 1 { 64. } else { 12. }))
                    .text_xs()
                    .italic()
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(subtitle),
            );

        if n.kind.has_input() {
            el = el.child(
                div()
                    .absolute()
                    .left(px(-8.))
                    .top(px(NODE_H / 2. - 8.))
                    .size(px(14.))
                    .rounded_full()
                    .bg(rgb(PORT_BG))
                    .border_2()
                    .border_color(rgb(color)),
            );
        }

        for (i, label) in outputs.iter().enumerate() {
            let y = port_offset(outputs.len(), i);
            let connected = self.doc.next(id, i).is_some();
            el = el.child(
                div()
                    .absolute()
                    .right(px(-8.))
                    .top(px(y - 8.))
                    .size(px(14.))
                    .rounded_full()
                    .bg(if connected { rgb(color) } else { rgb(PORT_BG) })
                    .border_2()
                    .border_color(rgb(color))
                    .cursor_crosshair()
                    .hover(|s| s.border_color(rgb(0xffffff)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.commit_edit();
                            this.drag = Drag::Connect { from: id, port: i };
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
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
                        .right(px(12.))
                        .top(px(y - 8.))
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(*label),
                );
            }
        }
        el
    }

    // -----------------------------------------------------------------------
    // Rendering: inspector
    // -----------------------------------------------------------------------

    fn macro_settings(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            section_title("Macro").into_any_element(),
            self.text_field("Name", Field::MacroName, cx).into_any_element(),
            self.num_field("Start delay (ms)", Field::StartDelay, 250.0, cx).into_any_element(),
            self.num_field("Gap between steps (ms)", Field::StepDelay, 10.0, cx).into_any_element(),
            self.num_field("Speed (%)", Field::Speed, 10.0, cx).into_any_element(),
            self.num_field("Repeat whole macro", Field::Repeat, 1.0, cx).into_any_element(),
            help("Repeat 0 = keep going until you press Esc. Speed 200% halves every delay.").into_any_element(),
            div().h(px(8.)).into_any_element(),
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
        ]
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
                .when(!matches!(n.kind, NodeKind::Start), |d| {
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
        out.push(self.text_field("Note", Field::Note, cx).into_any_element());
        out.push(self.num_field("Delay before (ms)", Field::Delay, 50.0, cx).into_any_element());
        out.push(div().h(px(6.)).into_any_element());

        match &n.kind {
            NodeKind::Start => {
                out.push(help("Every run begins here. Connect its dot to your first step.").into_any_element());
            }
            NodeKind::End => {
                out.push(help("The macro stops when it reaches this step (or any step with nothing connected).").into_any_element());
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
            NodeKind::KeyPress { key, mods } => {
                let capturing = self.capturing_key;
                out.push(
                    labeled(
                        "Key",
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(BG))
                            .border_1()
                            .border_color(if capturing { rgb(ACCENT) } else { rgb(BORDER) })
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
                    chip("type-enter", "Press Return afterwards", *press_enter)
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
                out.push(self.num_field("Lines", Field::ScrollAmt, 1.0, cx).into_any_element());
                out.push(
                    chip("scroll-h", "Sideways", *horizontal)
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
                out.push(help("Positive scrolls down/right, negative scrolls up/left.").into_any_element());
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
                ..
            } => {
                out.extend(self.read_text_settings(id, *compare, *case_sensitive, preview.clone(), *w, *h, cx));
            }
            NodeKind::Loop { .. } => {
                out.push(self.num_field("Times (0 = forever)", Field::LoopCount, 1.0, cx).into_any_element());
                out.push(help("Connect “repeat” to the steps you want repeated, and connect the last of them back into this Loop. When the count runs out it follows “done”.").into_any_element());
            }
        }
        out
    }

    fn target_row(&self, id: u64, target: Target, cx: &mut Context<Self>) -> Div {
        let opts = [
            (Target::Fixed, "Fixed spot"),
            (Target::FoundImage, "Found image"),
            (Target::Current, "Where it is"),
        ];
        labeled(
            "Where",
            div().flex().gap_1().children(opts.into_iter().map(|(t, label)| {
                chip(("tgt", t as usize), label, t == target).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| match k {
                            NodeKind::Click { target, .. } | NodeKind::MoveMouse { target, .. } => *target = t,
                            _ => {}
                        });
                        cx.notify();
                    },
                ))
            })),
        )
    }

    fn position_fields(&self, id: u64, target: Target, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match target {
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
            Target::FoundImage => vec![
                self.num_field("Offset X", Field::OffX, 1.0, cx).into_any_element(),
                self.num_field("Offset Y", Field::OffY, 1.0, cx).into_any_element(),
                help("Uses the centre of whatever the last Find image step found, moved by the offset.").into_any_element(),
            ],
            Target::Current => vec![help("Acts wherever the mouse pointer already is.").into_any_element()],
        }
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
        out.push(
            div()
                .h(px(110.))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(rgb(BG))
                .border_1()
                .border_color(rgb(BORDER))
                .p_1()
                .child(preview)
                .into_any_element(),
        );
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
                    move |this, _: &ClickEvent, _, cx| {
                        this.start_test(id);
                        cx.notify();
                    },
                )))
                .into_any_element(),
        );
        out.push(self.num_field("Snip size (pt)", Field::SnipSize, 8.0, cx).into_any_element());
        out.push(self.num_field("Match threshold", Field::Threshold, 0.01, cx).into_any_element());
        out.push(self.num_field("Keep looking for (ms)", Field::Timeout, 500.0, cx).into_any_element());
        out.push(self.num_field("Check every (ms)", Field::Interval, 50.0, cx).into_any_element());
        out.push(
            chip("find-click", "Click it when found", click_when_found)
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
                .flex_wrap()
                .gap_1()
                .child(chip("win-case", "Match case", case_sensitive).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.update_node(id, |k| {
                            if let NodeKind::WindowCheck { case_sensitive, .. } = k {
                                *case_sensitive = !*case_sensitive;
                            }
                        });
                        cx.notify();
                    },
                )))
                .child(chip("win-front", "Bring it to the front if found", bring_to_front).on_click(
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
                    .bg(rgb(PANEL2))
                    .hover(|s| s.bg(rgb(BTN_HOVER)))
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
        out.push(
            div()
                .h(px(80.))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(rgb(BG))
                .border_1()
                .border_color(rgb(BORDER))
                .p_1()
                .child(preview_el)
                .into_any_element(),
        );
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
                    move |this, _: &ClickEvent, _, cx| {
                        this.start_read_test(id);
                        cx.notify();
                    },
                )))
                .into_any_element(),
        );
        out.push(help("Shift-click “Select area” to get 3 seconds to switch to another window first.").into_any_element());
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
        if !compare.is_number() && compare.needs_value() {
            out.push(
                chip("rt-case", "Match case", case_sensitive)
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
        out.push(help("Type text steps can type what was read with {text}. Make sure MacroFlow isn't covering the area when the macro runs.").into_any_element());
        out
    }

    // -----------------------------------------------------------------------
    // Field widgets
    // -----------------------------------------------------------------------

    fn value_box(&self, field: Field, width: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let editing = matches!(&self.editing, Some((f, _)) if *f == field);
        let text = match &self.editing {
            Some((f, buf)) if *f == field => format!("{buf}▏"),
            _ => self.get_field(field),
        };
        let base = div()
            .id(SharedString::from(format!("val-{field:?}")))
            .px_2()
            .py_0p5()
            .rounded_md()
            .bg(rgb(BG))
            .border_1()
            .border_color(if editing { rgb(ACCENT) } else { rgb(BORDER) })
            .cursor_text()
            .truncate()
            .child(text)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.start_edit(field, window);
                cx.notify();
            }));
        if width > 0.0 {
            base.w(px(width)).flex_none()
        } else {
            base.flex_1()
        }
    }

    fn num_field(&self, label: &str, field: Field, step: f64, cx: &mut Context<Self>) -> Div {
        let minus = small_btn(SharedString::from(format!("minus-{field:?}")), "−").on_click(
            cx.listener(move |this, ev: &ClickEvent, _, cx| {
                let mult = if ev.modifiers().shift { 10.0 } else { 1.0 };
                this.bump(field, -step * mult);
                cx.notify();
            }),
        );
        let plus = small_btn(SharedString::from(format!("plus-{field:?}")), "+").on_click(
            cx.listener(move |this, ev: &ClickEvent, _, cx| {
                let mult = if ev.modifiers().shift { 10.0 } else { 1.0 };
                this.bump(field, step * mult);
                cx.notify();
            }),
        );
        labeled(
            label,
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(minus)
                .child(self.value_box(field, 0.0, cx))
                .child(plus),
        )
    }

    fn text_field(&self, label: &str, field: Field, cx: &mut Context<Self>) -> Div {
        debug_assert!(field.is_text());
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().text_color(rgb(MUTED)).child(label.to_string()))
            .child(div().flex().child(self.value_box(field, 0.0, cx)))
    }
}

impl Render for MacroApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_quit))
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if this.drag != Drag::None {
                        this.drag = Drag::None;
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_sm()
            .child(self.render_titlebar(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .when(self.sidebar_open, |d| d.child(self.render_sidebar(cx)))
                    .child(self.render_graph(cx)),
            )
            .child(self.render_statusbar())
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn bezier_ctrl(x0: f32, x1: f32) -> f32 {
    ((x1 - x0).abs() * 0.5).max(50.0)
}

fn bezier_mid(x0: f32, y0: f32, x1: f32, y1: f32) -> (f32, f32) {
    let d = bezier_ctrl(x0, x1);
    let (ax, ay) = (x0 + d, y0);
    let (bx, by) = (x1 - d, y1);
    (
        (x0 + 3.0 * ax + 3.0 * bx + x1) / 8.0,
        (y0 + 3.0 * ay + 3.0 * by + y1) / 8.0,
    )
}

#[allow(clippy::too_many_arguments)]
fn stroke_curve(window: &mut Window, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Hsla, dashed: bool) {
    let d = bezier_ctrl(x0, x1);
    let mut b = PathBuilder::stroke(px(width));
    if dashed {
        b = b.dash_array(&[px(6.), px(4.)]);
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

fn tool_btn(id: impl Into<ElementId>, label: &'static str, enabled: bool) -> Stateful<Div> {
    tool_btn_colored(id, label, enabled, BTN, BTN_HOVER)
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

fn small_btn(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .px_2()
        .py_0p5()
        .rounded_md()
        .bg(rgb(BTN))
        .text_xs()
        .text_color(rgb(TEXT))
        .hover(|s| s.bg(rgb(BTN_HOVER)))
        .cursor_pointer()
        .child(label)
}

fn chip(id: impl Into<ElementId>, label: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .border_1()
        .cursor_pointer()
        .when(on, |d| {
            d.bg(rgba((ACCENT << 8) | 0x40))
                .border_color(rgb(ACCENT))
                .text_color(rgb(0xffffff))
        })
        .when(!on, |d| {
            d.bg(rgb(BTN))
                .border_color(rgb(BORDER))
                .text_color(rgb(MUTED))
                .hover(|s| s.bg(rgb(BTN_HOVER)))
        })
        .child(label)
}

fn labeled(label: &str, content: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(118.))
                .flex_none()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label.to_string()),
        )
        .child(div().flex_1().child(content))
}

fn section_title(text: &'static str) -> Div {
    div()
        .pt_1()
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(MUTED))
        .child(text.to_uppercase())
}

fn help(text: &str) -> Div {
    div().text_xs().text_color(rgb(MUTED)).child(text.to_string())
}

fn tip(text: &'static str) -> Div {
    div().text_xs().text_color(rgb(MUTED)).pb_1().child(text)
}

fn divider() -> Div {
    div().w(px(1.)).h(px(22.)).bg(rgb(BORDER)).mx_1()
}

#[cfg(target_os = "macos")]
const SHORTCUT_TIP: &str = "⌘S save · ⌘O open · ⌘D duplicate · ⌫ delete step · ⌘R run · ⌘B show/hide panel";
#[cfg(not(target_os = "macos"))]
const SHORTCUT_TIP: &str = "Ctrl+S save · Ctrl+O open · Ctrl+D duplicate · Delete removes a step · Ctrl+R run · Ctrl+B show/hide panel";

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
        NodeKind::Click { .. } | NodeKind::MoveMouse { .. } | NodeKind::Scroll { .. } => 0,
        NodeKind::KeyPress { .. } | NodeKind::TypeText { .. } => 1,
        NodeKind::FindImage { .. } | NodeKind::WindowCheck { .. } | NodeKind::ReadText { .. } => 2,
        NodeKind::Recording { .. } => 4,
        _ => 3,
    }
}

fn step_blurb(k: &NodeKind) -> &'static str {
    match k {
        NodeKind::Start => "Where every run begins",
        NodeKind::Click { .. } => "Click at a spot or on a found image",
        NodeKind::MoveMouse { .. } => "Move the pointer somewhere",
        NodeKind::Scroll { .. } => "Scroll up, down or sideways",
        NodeKind::KeyPress { .. } => "Press a key or shortcut",
        NodeKind::TypeText { .. } => "Type some text",
        NodeKind::FindImage { .. } => "Branch on an image on screen",
        NodeKind::WindowCheck { .. } => "Branch on a window's title or app",
        NodeKind::ReadText { .. } => "Read text or a number and compare",
        NodeKind::Wait { .. } => "Pause, with optional randomness",
        NodeKind::Loop { .. } => "Repeat a branch N times",
        NodeKind::End => "Stop the macro",
        NodeKind::Recording { .. } => "Empty recording to fill or edit",
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

/// A quiet, borderless title bar button.
fn ghost_btn(id: impl Into<ElementId>, label: &'static str, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .px_2()
        .h(px(26.))
        .rounded_md()
        .text_color(rgb(TEXT))
        .when(enabled, |d| d.hover(|s| s.bg(rgb(BTN_HOVER))).cursor_pointer())
        .when(!enabled, |d| d.opacity(0.4))
        .child(label)
}

/// Minimise / maximise / close buttons for Windows (macOS has traffic lights).
fn window_controls(is_maximized: bool) -> Div {
    let cell = |id: &'static str, area: WindowControlArea| {
        div()
            .id(id)
            .w(px(46.))
            .h(px(TITLEBAR_H))
            .flex()
            .items_center()
            .justify_center()
            .window_control_area(area)
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
                    .bg(rgb(TITLEBAR))
                    .border_1()
                    .border_color(rgb(TEXT)),
            )
    } else {
        div().size(px(10.)).border_1().border_color(rgb(TEXT))
    };
    div()
        .flex()
        .flex_none()
        .h_full()
        .child(
            cell("win-min", WindowControlArea::Min)
                .hover(|s| s.bg(rgb(BTN_HOVER)))
                .child(div().w(px(10.)).h(px(1.)).bg(rgb(TEXT))),
        )
        .child(
            cell("win-max", WindowControlArea::Max)
                .hover(|s| s.bg(rgb(BTN_HOVER)))
                .child(max_icon),
        )
        .child(
            cell("win-close", WindowControlArea::Close)
                .hover(|s| s.bg(rgb(0xc42b1c)))
                .child(
                    canvas(
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
                    .size(px(10.)),
                ),
        )
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

// ---------------------------------------------------------------------------
// Full-screen area selection
// ---------------------------------------------------------------------------

/// Open a borderless window covering the whole display, showing a frozen screenshot
/// of it, so the user can drag a box anywhere on their screen.
fn open_area_overlay(
    shot: vision::Shot,
    purpose: AreaPurpose,
    tx: Sender<WorkerMsg>,
    cx: &mut Context<MacroApp>,
) -> Result<(), String> {
    let displays = cx.displays();
    let display = if IS_MACOS {
        displays.iter().find(|d| u32::from(d.id()) == shot.monitor_id).cloned()
    } else {
        displays.get(shot.monitor_index).cloned()
    }
    .or_else(|| cx.primary_display())
    .ok_or_else(|| "Couldn't find the display to select on.".to_string())?;

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(display.bounds())),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(display.id()),
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };
    cx.open_window(options, move |window, cx| {
        cx.new(|cx| AreaOverlay::new(shot, purpose, tx, window, cx))
    })
    .map_err(|e| format!("Couldn't open the selection screen: {e}"))?;
    cx.activate(true);
    Ok(())
}

pub struct AreaOverlay {
    focus: FocusHandle,
    shot: vision::Shot,
    purpose: AreaPurpose,
    /// Corners of the box, in this window's coordinates.
    start: Option<(f32, f32)>,
    end: Option<(f32, f32)>,
    dragging: bool,
    done: bool,
    tx: Sender<WorkerMsg>,
}

impl AreaOverlay {
    fn new(
        shot: vision::Shot,
        purpose: AreaPurpose,
        tx: Sender<WorkerMsg>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        // Closing this window any other way counts as cancelling.
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = this.update(cx, |o, _| o.report(None));
            true
        });
        AreaOverlay {
            focus,
            shot,
            purpose,
            start: None,
            end: None,
            dragging: false,
            done: false,
            tx,
        }
    }

    fn rect(&self) -> Option<(f32, f32, f32, f32)> {
        let (s, e) = (self.start?, self.end?);
        Some((s.0.min(e.0), s.1.min(e.1), s.0.max(e.0), s.1.max(e.1)))
    }

    /// Tell MacroFlow what was picked (in screenshot coordinates), once.
    fn report(&mut self, sel: Option<(f32, f32, f32, f32)>) {
        if self.done {
            return;
        }
        self.done = true;
        let _ = self.tx.send(WorkerMsg::AreaPicked {
            purpose: self.purpose,
            shot: self.shot.clone(),
            sel,
        });
    }

    fn finish(&mut self, confirm: bool, window: &mut Window) {
        let sel = if confirm {
            let vp = window.viewport_size();
            let sx = self.shot.w as f32 / f32::from(vp.width).max(1.0);
            let sy = self.shot.h as f32 / f32::from(vp.height).max(1.0);
            self.rect()
                .map(|(x0, y0, x1, y1)| (x0 * sx, y0 * sy, x1 * sx, y1 * sy))
        } else {
            None
        };
        self.report(sel);
        window.remove_window();
    }
}

impl Render for AreaOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let what = match self.purpose {
            AreaPurpose::ReadText(_) => "Drag a box around the text or number to read",
            AreaPurpose::Template(_) => "Drag a box around the thing to find",
        };
        let pos = |p: Point<Pixels>| (f32::from(p.x), f32::from(p.y));
        let dim = rgba(0x0000_0066);

        let mut root = div()
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(rgb(BG))
            .cursor_crosshair()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, _| {
                if ev.keystroke.key == "escape" {
                    this.finish(false, window);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    let p = pos(ev.position);
                    this.start = Some(p);
                    this.end = Some(p);
                    this.dragging = true;
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, window, _| this.finish(false, window)),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                if this.dragging && ev.pressed_button == Some(MouseButton::Left) {
                    this.end = Some(pos(ev.position));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseUpEvent, window, cx| {
                    if !this.dragging {
                        return;
                    }
                    this.end = Some(pos(ev.position));
                    this.dragging = false;
                    match this.rect() {
                        Some((x0, y0, x1, y1)) if x1 - x0 >= 4.0 && y1 - y0 >= 4.0 => {
                            this.finish(true, window)
                        }
                        _ => {
                            // A click rather than a drag – let them try again.
                            this.start = None;
                            this.end = None;
                            cx.notify();
                        }
                    }
                }),
            )
            // The frozen screen, at its real size, so it looks like your actual desktop.
            .child(
                img(self.shot.path.clone())
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            );

        match self.rect() {
            None => {
                root = root.child(div().absolute().top_0().left_0().size_full().bg(dim));
            }
            Some((x0, y0, x1, y1)) => {
                // Dim everything except the box.
                let shade = |l: f32, t: f32, w: f32, h: f32| {
                    div()
                        .absolute()
                        .left(px(l))
                        .top(px(t))
                        .w(px(w.max(0.0)))
                        .h(px(h.max(0.0)))
                        .bg(dim)
                };
                let vp_sx = self.shot.w as f32 / vw.max(1.0);
                let vp_sy = self.shot.h as f32 / vh.max(1.0);
                root = root
                    .child(shade(0.0, 0.0, vw, y0))
                    .child(shade(0.0, y1, vw, vh - y1))
                    .child(shade(0.0, y0, x0, y1 - y0))
                    .child(shade(x1, y0, vw - x1, y1 - y0))
                    .child(
                        div()
                            .absolute()
                            .left(px(x0))
                            .top(px(y0))
                            .w(px(x1 - x0))
                            .h(px(y1 - y0))
                            .border_2()
                            .border_color(rgb(ACCENT)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(x0))
                            .top(px(if y0 > 28.0 { y0 - 24.0 } else { y1 + 4.0 }))
                            .px_1p5()
                            .py_0p5()
                            .rounded_sm()
                            .bg(rgb(ACCENT))
                            .text_xs()
                            .text_color(rgb(0xffffff))
                            .child(format!(
                                "{:.0} × {:.0}",
                                (x1 - x0) * vp_sx,
                                (y1 - y0) * vp_sy
                            )),
                    );
            }
        }

        // Instructions, top centre.
        root.child(
            div()
                .absolute()
                .top(px(if IS_MACOS { 46.0 } else { 24.0 }))
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .child(
                    div()
                        .occlude()
                        .flex()
                        .items_center()
                        .gap_3()
                        .pl_4()
                        .pr_1p5()
                        .py_1p5()
                        .rounded_lg()
                        .bg(rgba(0x161922f2))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .shadow_lg()
                        .text_sm()
                        .text_color(rgb(TEXT))
                        .cursor_default()
                        .child(what)
                        .child(div().text_color(rgb(MUTED)).child("Let go to confirm · Esc or right-click to cancel"))
                        .child(
                            div()
                                .id("overlay-cancel")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(BTN))
                                .hover(|s| s.bg(rgb(BTN_HOVER)))
                                .cursor_pointer()
                                .child("Cancel")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.finish(false, window);
                                    }),
                                ),
                        ),
                ),
        )
    }
}
