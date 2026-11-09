//! Background work: playing macros, recording, picking positions and snipping images.
//! Everything here runs on worker threads and reports back to the UI over a channel.

use crate::keys;
use crate::model::*;
use crate::vision;
use device_query::{DeviceQuery, DeviceState, Keycode};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How a run of steps finished.
enum Exit {
    /// Reached an End step: this run of the macro is over.
    Ended,
    /// Reached the end of a group (or a step with nothing connected).
    LeftGroup,
}

pub enum WorkerMsg {
    Status(String),
    Active(Option<u64>),
    RunFinished(RunEnd),
    Recorded(Vec<RecEvent>),
    StepRecorded(NodeKind),
    Position { x: i32, y: i32 },
    Snipped(PathBuf),
    TestResult(String),
    /// A box cut out of a screenshot was saved.
    Cropped { purpose: AreaPurpose, path: PathBuf },
    Windows(Vec<vision::WinInfo>),
    /// F8 / F9 pressed anywhere.
    Hotkey(Hotkey),
    /// One line for the run log.
    Log(LogEntry),
    /// A position and the colour under it (for Pixel colour steps).
    Pixel { x: i32, y: i32, color: u32 },
    /// What a Read text / Find image step just looked at (a picture saved in
    /// `images/last-run/`), so the editor can show it in place of the original.
    Seen { step: u64, path: PathBuf, caption: String, ok: bool },
    /// The user finished (or cancelled) dragging a box on the screen.
    /// `sel` is (x0, y0, x1, y1) in the screenshot's display coordinates.
    AreaPicked {
        purpose: AreaPurpose,
        shot: vision::Shot,
        sel: Option<(f32, f32, f32, f32)>,
    },
    Failed(String),
}

/// What a drag-a-box selection is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaPurpose {
    /// The area a Read text step reads.
    ReadText(u64),
    /// The picture a Find image step looks for.
    Template(u64),
    /// The only part of the screen a Find image step searches.
    SearchArea(u64),
    /// Where a Click / Move mouse step picks a random spot.
    ClickArea(u64),
}

pub type Tx = Sender<WorkerMsg>;

/// Where the pictures of what steps saw are kept (emptied when MacroFlow starts).
pub fn seen_dir() -> PathBuf {
    images_dir().join("last-run")
}

/// Save what a step just looked at and tell the editor.
fn send_seen(tx: &Tx, step: u64, img: image::RgbaImage, caption: String, ok: bool) {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let dir = seen_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    // A new name every time: the editor keeps pictures it has shown by file name.
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("step{step}-{stamp}-{n}.png"));
    if vision::shrink_for_display(img).save(&path).is_ok() {
        send(tx, WorkerMsg::Seen { step, path, caption, ok });
    }
}

/// "Read “123”", plus what each read saw if they didn't all agree.
fn read_caption(r: &vision::OcrRead) -> String {
    let shown = if r.text.is_empty() {
        "nothing".to_string()
    } else {
        format!("“{}”", r.text.replace('\n', " ⏎ "))
    };
    if r.agreed < r.tries {
        let raw: Vec<String> = r
            .raw
            .iter()
            .map(|s| if s.is_empty() { "–".to_string() } else { format!("“{}”", s.replace('\n', " ⏎ ")) })
            .collect();
        format!("Read {shown} ({}/{} reads agreed: {})", r.agreed, r.tries, raw.join(", "))
    } else {
        format!("Read {shown}")
    }
}

const STOPPED: &str = "Stopped.";
#[cfg(target_os = "macos")]
pub const NEEDS_ACCESSIBILITY: &str = "MacroFlow needs Accessibility permission. Open System Settings → Privacy & Security → Accessibility, turn on the app you launched MacroFlow from (e.g. Terminal), then restart it.";
#[cfg(not(target_os = "macos"))]
pub const NEEDS_ACCESSIBILITY: &str = "MacroFlow couldn't read or send input. If the target app is running as administrator, run MacroFlow as administrator too.";

fn send(tx: &Tx, m: WorkerMsg) {
    let _ = tx.send(m);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn device() -> Result<DeviceState, String> {
    DeviceState::checked_new().ok_or_else(|| NEEDS_ACCESSIBILITY.to_string())
}

fn input_err<E: std::fmt::Debug>(e: E) -> String {
    format!("Couldn't send input: {e:?}. {NEEDS_ACCESSIBILITY}")
}

fn to_button(b: MouseBtn) -> Button {
    match b {
        MouseBtn::Left => Button::Left,
        MouseBtn::Right => Button::Right,
        MouseBtn::Middle => Button::Middle,
    }
}

fn sleep_checked(ms: u64, stop: &AtomicBool) -> Result<(), String> {
    let end = Instant::now() + Duration::from_millis(ms);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(STOPPED.into());
        }
        let now = Instant::now();
        if now >= end {
            return Ok(());
        }
        thread::sleep((end - now).min(Duration::from_millis(10)));
    }
}

/// Wait until the user has let go of every key and mouse button
/// (so the click on a MacroFlow button isn't recorded).
fn wait_all_released(ds: &DeviceState, stop: &AtomicBool) {
    let t0 = Instant::now();
    loop {
        let any_button = ds.get_mouse().button_pressed.iter().any(|b| *b);
        if (!any_button && ds.get_keys().is_empty())
            || stop.load(Ordering::Relaxed)
            || t0.elapsed() > Duration::from_secs(5)
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn countdown(tx: &Tx, stop: &AtomicBool, what: &str, secs: u32) -> Result<(), String> {
    for s in (1..=secs).rev() {
        send(tx, WorkerMsg::Status(format!("{what} in {s}…")));
        sleep_checked(1000, stop)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    /// F8: start the macro, or stop it if it's running.
    StartStop,
    /// F9: pause or resume a running macro.
    Pause,
    /// F10: run just the next step (or start the macro one step at a time).
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Step,
    Info,
    Good,
    Warn,
    Bad,
}

/// One line of the run log.
#[derive(Clone, Debug)]
pub struct LogEntry {
    /// Milliseconds since the run started.
    pub ms: u64,
    pub step: Option<u64>,
    pub text: String,
    pub level: LogLevel,
}

/// Why a run of the macro ended, so the editor can highlight the step responsible.
#[derive(Clone, Debug, PartialEq)]
pub struct RunEnd {
    pub message: String,
    /// The step that ended the run (shown in red).
    pub step: Option<u64>,
    /// When it ended because this output of `step` isn't connected to anything.
    pub port: Option<usize>,
    pub kind: EndKind,
    /// A screenshot taken when a step went wrong.
    pub screenshot: Option<PathBuf>,
}

impl RunEnd {
    /// Ended in a way you probably didn't intend: an error, or a path with nothing
    /// connected (rather than an End step, or you pressing Stop).
    pub fn unexpected(&self) -> bool {
        self.kind == EndKind::Error || (self.kind == EndKind::Finished && self.port.is_some())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndKind {
    /// Reached an End step, or a step with nothing connected after it.
    Finished,
    /// You pressed Esc or Stop.
    Stopped,
    /// A step went wrong.
    Error,
}

/// Where the path through the macro last ran out.
#[derive(Clone, Copy, Debug)]
enum EndAt {
    EndStep(u64),
    /// This output of this step isn't connected to anything.
    Dangling(u64, usize),
}

/// The name of the thread that plays macros (the crash handler knows panics there are
/// already reported).
pub const RUN_THREAD: &str = "macroflow-run";

/// Play the macro. With `from`, start at that step instead of Start and run once.
/// `paused` pauses playback while it's set (F9). While `step_mode` is set, it pauses
/// before every step (F10 runs one).
pub fn run_macro(
    doc: Macro,
    tx: Tx,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    step_mode: Arc<AtomicBool>,
    from: Option<u64>,
) {
    // The playback thread gets its own copy of `tx`, so the one here can still report
    // a failure to start it.
    let thread_tx = tx.clone();
    let spawned = thread::Builder::new().name(RUN_THREAD.into()).spawn(move || {
        let tx = thread_tx;
        let t0 = Instant::now();
        let last_step = Arc::new(AtomicU64::new(0));
        let tracked = last_step.clone();
        // Catch crashes inside playback, so they're reported instead of killing MacroFlow.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match Runner::new(&doc, &tx, stop.clone(), paused.clone(), step_mode.clone(), tracked) {
                Ok(mut r) => {
                    let result = r.run(from);
                    r.describe_end(result)
                }
                Err(e) => RunEnd {
                    message: e,
                    step: None,
                    port: None,
                    kind: EndKind::Error,
                    screenshot: None,
                },
            }
        }));
        let mut end = result.unwrap_or_else(|panic| {
            let why = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown error".into());
            let step = match last_step.load(Ordering::Relaxed) {
                0 => None,
                id => Some(id),
            };
            RunEnd {
                message: format!("MacroFlow hit an internal error while running ({why}). Please report this."),
                step,
                port: None,
                kind: EndKind::Error,
                screenshot: None,
            }
        });
        if end.kind == EndKind::Error {
            end.screenshot = failure_screenshot();
        }
        let ms = t0.elapsed().as_millis() as u64;
        send(
            &tx,
            WorkerMsg::Log(LogEntry {
                ms,
                step: end.step,
                text: end.message.clone(),
                level: match end.kind {
                    EndKind::Error => LogLevel::Bad,
                    EndKind::Stopped => LogLevel::Warn,
                    EndKind::Finished if end.unexpected() => LogLevel::Warn,
                    EndKind::Finished => LogLevel::Good,
                },
            }),
        );
        if doc.alert_on_stop && end.unexpected() {
            alert(&end);
        } else if doc.alert_on_finish && end.kind == EndKind::Finished && !end.unexpected() {
            popup("Macro finished", &end.message, "success", 6000);
        }
        send(&tx, WorkerMsg::Active(None));
        send(&tx, WorkerMsg::RunFinished(end));
    });
    if let Err(e) = spawned {
        send(&tx, WorkerMsg::Failed(format!("Couldn't start the macro: {e}")));
    }
}

/// One wheel click (`back` = up / left). `strength` makes the click scroll further.
fn scroll_click(enigo: &mut Enigo, back: bool, horizontal: bool, strength: u32) -> Result<(), String> {
    let axis = if horizontal { Axis::Horizontal } else { Axis::Vertical };
    let n = strength.clamp(1, 20) as i32;
    enigo.scroll(if back { -n } else { n }, axis).map_err(input_err)
}

/// Scroll wherever the mouse is in 3 seconds, to check how far a Scroll step goes.
pub fn test_scroll(tx: Tx, amount: i32, horizontal: bool, strength: u32) {
    thread::spawn(move || {
        for left in (1..=3).rev() {
            send(
                &tx,
                WorkerMsg::Status(format!("Point the mouse at what to scroll – scrolling in {left}…")),
            );
            thread::sleep(Duration::from_secs(1));
        }
        let result = (|| -> Result<(), String> {
            let mut enigo = Enigo::new(&Settings::default()).map_err(input_err)?;
            for i in 0..amount.unsigned_abs() {
                if i > 0 {
                    thread::sleep(Duration::from_millis(35));
                }
                scroll_click(&mut enigo, amount < 0, horizontal, strength)?;
            }
            Ok(())
        })();
        send(
            &tx,
            WorkerMsg::TestResult(match result {
                Ok(()) => format!(
                    "Scrolled {} wheel click{}. Too little? Raise the strength. Too much? Lower the clicks.",
                    amount.unsigned_abs(),
                    if amount.unsigned_abs() == 1 { "" } else { "s" }
                ),
                Err(e) => e,
            }),
        );
    });
}

/// A pop-up that isn't tied to a step (alerts from MacroFlow itself).
fn popup(title: &str, message: &str, style: &'static str, duration_ms: u64) {
    let toast = vision::Toast {
        title: title.into(),
        message: message.into(),
        style,
        position: "top-center",
        duration_ms,
        wait: false,
        sound: true,
    };
    // The pop-up is its own little program, so it's fine to let it run on its own.
    let _ = vision::show_notification(&toast);
}

/// Show a red pop-up saying why the macro stopped.
fn alert(end: &RunEnd) {
    let title = if end.kind == EndKind::Error {
        "Macro stopped – something went wrong"
    } else {
        "Macro stopped unexpectedly"
    };
    let toast = vision::Toast {
        title: title.into(),
        message: end.message.clone(),
        style: "error",
        position: "top-center",
        duration_ms: 9000,
        wait: false,
        sound: true,
    };
    // The pop-up is its own little program, so it's fine to let it run on its own.
    let _ = vision::show_notification(&toast);
}

/// Save a screenshot of the screen the mouse is on, for the run log.
fn failure_screenshot() -> Option<PathBuf> {
    let (x, y) = device().ok().map(|d| d.get_mouse().coords).unwrap_or((0, 0));
    let shot = vision::screenshot_display_at(x, y).ok()?;
    let dir = images_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("failed-{}.png", now_ms()));
    if std::fs::copy(&shot.path, &path).is_err() {
        let _ = std::fs::remove_file(&shot.path);
        return None;
    }
    let _ = std::fs::remove_file(&shot.path);
    Some(path)
}

/// Watch for F8 (start / stop) and F9 (pause / resume) in any app.
pub fn spawn_hotkeys(tx: Tx) {
    let _ = thread::Builder::new().name("macroflow-hotkeys".into()).spawn(move || {
        let Ok(ds) = device() else {
            return;
        };
        let (mut f8, mut f9, mut f10) = (false, false, false);
        loop {
            let keys = ds.get_keys();
            let now8 = keys.contains(&Keycode::F8);
            let now9 = keys.contains(&Keycode::F9);
            let now10 = keys.contains(&Keycode::F10);
            for (now, before, key) in [
                (now8, f8, Hotkey::StartStop),
                (now9, f9, Hotkey::Pause),
                (now10, f10, Hotkey::Step),
            ] {
                if now && !before && tx.send(WorkerMsg::Hotkey(key)).is_err() {
                    return;
                }
            }
            (f8, f9, f10) = (now8, now9, now10);
            thread::sleep(Duration::from_millis(40));
        }
    });
}

struct Runner<'a> {
    doc: &'a Macro,
    tx: &'a Tx,
    stop: Arc<AtomicBool>,
    enigo: Enigo,
    found: Option<(i32, i32)>,
    /// Text from the most recent Read text step (typed by `{text}` in Type text).
    last_text: Option<String>,
    /// Last reading of each Read text step (for "has changed").
    prev_reads: HashMap<u64, String>,
    /// The pop-up currently on screen (a new one replaces it).
    toast: Option<std::process::Child>,
    /// The step running right now (blamed if something goes wrong).
    current: Option<u64>,
    /// Where the path last ran out (blamed when the run ends).
    ended_at: Option<EndAt>,
    /// Where each mouse step last went, so a random spot is never repeated.
    last_points: HashMap<u64, (i32, i32)>,
    /// Variables set by Set variable / Read text steps (also `text`, `run`, `found_x`, `found_y`).
    vars: HashMap<String, String>,
    /// Pause playback while this is set (F9).
    paused: Arc<AtomicBool>,
    /// When the run started (for the log).
    started: Instant,
    /// The step running now, shared with the crash handler.
    last_step: Arc<AtomicU64>,
    /// How much bigger things are on this screen than when the macro was made
    /// (Find image resizes its picture to match).
    scale: f64,
    /// Pause before every step (step-by-step mode).
    step_mode: Arc<AtomicBool>,
    /// Set by the watcher when the mouse is pushed into a screen corner.
    failsafe_hit: Arc<AtomicBool>,
    /// Where the macro itself last put the mouse (so it doesn't trip the failsafe).
    last_auto: Arc<AtomicU64>,
    loops: HashMap<u64, u32>,
    held_keys: Vec<Key>,
    held_buttons: Vec<Button>,
    suppress_esc_until: Arc<AtomicU64>,
    watch_done: Arc<AtomicBool>,
    rng: u64,
}

impl<'a> Runner<'a> {
    fn new(
        doc: &'a Macro,
        tx: &'a Tx,
        stop: Arc<AtomicBool>,
        paused: Arc<AtomicBool>,
        step_mode: Arc<AtomicBool>,
        last_step: Arc<AtomicU64>,
    ) -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default()).map_err(input_err)?;
        let suppress_esc_until = Arc::new(AtomicU64::new(0));
        let watch_done = Arc::new(AtomicBool::new(false));
        let failsafe_hit = Arc::new(AtomicBool::new(false));
        let last_auto = Arc::new(AtomicU64::new(NO_POINT));
        spawn_esc_watch(
            stop.clone(),
            watch_done.clone(),
            suppress_esc_until.clone(),
            doc.failsafe.then(|| (failsafe_hit.clone(), last_auto.clone())),
        );
        Ok(Runner {
            doc,
            tx,
            stop,
            enigo,
            found: None,
            last_text: None,
            prev_reads: HashMap::new(),
            toast: None,
            current: None,
            ended_at: None,
            last_points: HashMap::new(),
            vars: HashMap::new(),
            paused,
            started: Instant::now(),
            last_step,
            scale: 1.0,
            step_mode,
            failsafe_hit,
            last_auto,
            loops: HashMap::new(),
            held_keys: Vec::new(),
            held_buttons: Vec::new(),
            suppress_esc_until,
            watch_done,
            rng: now_ms() | 1,
        })
    }

    /// Wait `ms` (not counting time spent paused). Returns early if the macro is stopped.
    fn sleep(&self, ms: u64) -> Result<(), String> {
        let mut left = Duration::from_millis(ms);
        let mut announced = false;
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(STOPPED.into());
            }
            if self.paused.load(Ordering::Relaxed) {
                if !announced {
                    self.status("❚❚ Paused – press F9 (or Resume) to carry on, Esc or F8 to stop".into());
                    announced = true;
                }
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            if left.is_zero() {
                return Ok(());
            }
            let chunk = left.min(Duration::from_millis(10));
            thread::sleep(chunk);
            left -= chunk;
        }
    }

    fn log(&self, level: LogLevel, text: String) {
        send(
            self.tx,
            WorkerMsg::Log(LogEntry {
                ms: self.started.elapsed().as_millis() as u64,
                step: self.current,
                text,
                level,
            }),
        );
    }

    /// How to turn saved positions into positions on this screen right now.
    fn mapping(&self) -> Result<Option<Mapping>, String> {
        vision::current_mapping(self.doc.reference, self.doc.anchor.as_ref())
    }

    /// Apply the macro's global speed to a delay.
    fn scaled(&self, ms: u64) -> u64 {
        ms * 100 / self.doc.speed_pct.max(1) as u64
    }

    fn random(&mut self, max: u64) -> u64 {
        if max == 0 {
            return 0;
        }
        // xorshift64 – plenty for jittering delays.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng % (max + 1)
    }

    fn status(&self, s: String) {
        send(self.tx, WorkerMsg::Status(s));
    }

    /// Run the macro (or, with `from`, once from that step). Returns how many times it ran.
    fn run(&mut self, from: Option<u64>) -> Result<u32, String> {
        let start = match from {
            Some(f) => {
                self.doc
                    .node(f)
                    .ok_or_else(|| "That step no longer exists.".to_string())?;
                f
            }
            None => self
                .doc
                .start_id()
                .ok_or_else(|| "Add a Start step first.".to_string())?,
        };
        self.current = Some(start);
        if from.is_none() && self.doc.next(start, 0).is_none() {
            return Err("Connect the Start step to your first step (drag from its right-hand dot).".into());
        }
        let wait = self.doc.start_delay_ms;
        if wait > 0 {
            self.status(format!(
                "Starting in {:.1}s – switch to your target app. Press Esc any time to stop.",
                wait as f32 / 1000.0
            ));
            self.sleep(wait)?;
        }
        if from.is_some() {
            // Run from this step; when it's inside a group, carry on after the group
            // (and after any groups around that) like a normal run would.
            let mut first = true;
            let path = self.doc.root.path_to(start).unwrap_or_default();
            let mut exit = self.walk(start, 1, 1, &mut first)?;
            for gid in path.iter().rev() {
                if let Exit::Ended = exit {
                    break;
                }
                self.current = Some(*gid);
                match self.doc.next(*gid, 0) {
                    Some(next) => exit = self.walk(next, 1, 1, &mut first)?,
                    None => {
                        self.ended_at = Some(EndAt::Dangling(*gid, 0));
                        break;
                    }
                }
            }
            return Ok(1);
        }
        let total = self.doc.repeat;
        let mut iteration = 0u32;
        loop {
            iteration += 1;
            self.vars.insert("run".into(), iteration.to_string());
            self.loops.clear();
            let mut first = true;
            self.walk(start, iteration, total, &mut first)?;
            if total != 0 && iteration >= total {
                break;
            }
        }
        Ok(iteration)
    }

    /// A step's name for messages: its title (or group name), plus its note if it has one.
    fn step_label(&self, id: u64) -> String {
        let Some(n) = self.doc.node(id) else {
            return "a step".into();
        };
        let title = n.label();
        if n.note.trim().is_empty() {
            format!("“{title}”")
        } else {
            format!("“{title}” ({})", n.note.trim())
        }
    }

    /// Explain why the run ended, and which step caused it.
    fn describe_end(&self, result: Result<u32, String>) -> RunEnd {
        match result {
            Ok(times) => {
                let ran = if times == 1 {
                    String::new()
                } else {
                    format!(" after {times} runs")
                };
                match self.ended_at {
                    Some(EndAt::EndStep(id)) => RunEnd {
                        message: format!("Finished{ran}: reached the {} step.", self.step_label(id)),
                        step: Some(id),
                        port: None,
                        kind: EndKind::Finished,
                        screenshot: None,
                    },
                    Some(EndAt::Dangling(id, port)) => {
                        let outputs = self.doc.node(id).map(|n| n.kind.outputs()).unwrap_or(&[]);
                        let label = self.step_label(id);
                        let message = if outputs.len() > 1 {
                            let path = outputs.get(port).copied().unwrap_or("next");
                            format!("Stopped{ran} at {label}: it went down its “{path}” path, and nothing is connected there.")
                        } else {
                            format!("Stopped{ran} after {label}: nothing is connected after it.")
                        };
                        RunEnd {
                            message,
                            step: Some(id),
                            port: Some(port),
                            kind: EndKind::Finished,
                            screenshot: None,
                        }
                    }
                    None => RunEnd {
                        message: format!("Finished{ran}."),
                        step: None,
                        port: None,
                        kind: EndKind::Finished,
                        screenshot: None,
                    },
                }
            }
            Err(e) if e == STOPPED => RunEnd {
                message: match (self.failsafe_hit.load(Ordering::Relaxed), self.current) {
                    (true, Some(id)) => format!(
                        "Stopped by the failsafe during {}: the mouse was pushed into a corner of the screen.",
                        self.step_label(id)
                    ),
                    (true, None) => "Stopped by the failsafe: the mouse was pushed into a corner of the screen.".into(),
                    (false, Some(id)) => format!("Stopped by you during {}.", self.step_label(id)),
                    (false, None) => "Stopped by you.".into(),
                },
                step: self.current,
                port: None,
                kind: EndKind::Stopped,
                screenshot: None,
            },
            Err(e) => RunEnd {
                message: match self.current {
                    Some(id) => format!("{} went wrong: {e}", self.step_label(id)),
                    None => e,
                },
                step: self.current,
                port: None,
                kind: EndKind::Error,
                screenshot: None,
            },
        }
    }

    /// Run steps from `start` until the path ends. Group steps run their own
    /// steps (recursively) and then carry on from the group's output.
    fn walk(&mut self, start: u64, iteration: u32, total: u32, first: &mut bool) -> Result<Exit, String> {
        let mut cur = Some(start);
        while let Some(id) = cur {
            if self.stop.load(Ordering::Relaxed) {
                return Err(STOPPED.into());
            }
            let node = self
                .doc
                .node(id)
                .cloned()
                .ok_or_else(|| "A connection points to a step that no longer exists.".to_string())?;
            match &node.kind {
                // Markers inside a group: nothing to do or wait for.
                NodeKind::GroupIn => {
                    cur = self.doc.next(id, 0);
                    if cur.is_none() {
                        self.ended_at = Some(EndAt::Dangling(id, 0));
                    }
                    continue;
                }
                NodeKind::GroupOut => return Ok(Exit::LeftGroup),
                _ => {}
            }
            // Hold here while paused.
            self.sleep(0)?;
            send(self.tx, WorkerMsg::Active(Some(id)));
            self.current = Some(id);
            self.last_step.store(id, Ordering::Relaxed);
            // Breakpoints and step-by-step mode pause *before* the step runs.
            let stepping = self.step_mode.load(Ordering::Relaxed);
            if node.breakpoint || stepping {
                let why = if node.breakpoint { "Breakpoint" } else { "Step by step" };
                self.paused.store(true, Ordering::Relaxed);
                self.log(LogLevel::Warn, format!("⏸ {why} – paused before “{}”", node.label()));
                // Step-by-step pauses before every step, so only breakpoints pop up.
                if node.breakpoint && !stepping && self.doc.alert_on_breakpoint {
                    popup(
                        "Paused at a breakpoint",
                        &format!("Before “{}”. F9 carries on · F10 runs just this step · Esc stops", node.label()),
                        "info",
                        8000,
                    );
                }
                self.status(format!(
                    "⏸ {why} – paused before “{}”.  F10 / Step runs it · F9 / Resume carries on · Esc stops",
                    node.label()
                ));
                while self.paused.load(Ordering::Relaxed) {
                    if self.stop.load(Ordering::Relaxed) {
                        return Err(STOPPED.into());
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
            self.log(LogLevel::Step, node.label());
            let of = if total == 0 {
                String::new()
            } else {
                format!("/{total}")
            };
            let note = if node.note.is_empty() {
                String::new()
            } else {
                format!(" – {}", node.note)
            };
            let title = node.label();
            self.status(format!("▶ Run {iteration}{of}: {title}{note}   (Esc to stop)"));
            let step_gap = if *first { 0 } else { self.doc.step_delay_ms };
            self.sleep(self.scaled(node.delay_ms + step_gap))?;
            *first = false;

            if let NodeKind::Group { graph, .. } = &node.kind {
                let inner_start = graph
                    .nodes
                    .iter()
                    .find(|n| matches!(n.kind, NodeKind::GroupIn))
                    .map(|n| n.id);
                if let Some(s) = inner_start {
                    if let Exit::Ended = self.walk(s, iteration, total, first)? {
                        return Ok(Exit::Ended);
                    }
                }
                self.current = Some(id);
                cur = self.doc.next(id, 0);
                if cur.is_none() {
                    self.ended_at = Some(EndAt::Dangling(id, 0));
                }
                continue;
            }
            if matches!(node.kind, NodeKind::End) {
                self.ended_at = Some(EndAt::EndStep(id));
                return Ok(Exit::Ended);
            }
            // Positions saved for another screen size, or relative to a window, are
            // converted to this screen right now.
            let mut node = node;
            self.scale = 1.0;
            let needs_mapping = uses_screen_positions(&node.kind) || matches!(node.kind, NodeKind::FindImage { .. });
            if (self.doc.anchor.is_some() || self.doc.reference.is_some()) && needs_mapping {
                if let Some(m) = self.mapping()? {
                    map_kind(&mut node.kind, &m);
                    self.scale = m.scale();
                }
            }
            match self.exec(&node)? {
                Some(p) => {
                    let outputs = node.kind.outputs();
                    if outputs.len() > 1 {
                        let level = if p == 0 { LogLevel::Good } else { LogLevel::Warn };
                        self.log(level, format!("→ {}", outputs.get(p).copied().unwrap_or("?")));
                    }
                    cur = self.doc.next(id, p);
                    if cur.is_none() {
                        self.ended_at = Some(EndAt::Dangling(id, p));
                    }
                }
                None => {
                    self.ended_at = Some(EndAt::EndStep(id));
                    cur = None;
                }
            }
        }
        // Ran out of connections: inside a group that means "carry on after the group".
        Ok(Exit::LeftGroup)
    }

    /// Put variables (and what the last Read text step read, as `{text}`) into text.
    fn fill_text(&self, s: &str) -> String {
        fill_vars(s, &self.vars)
    }

    fn set_last_text(&mut self, t: String) {
        self.vars.insert("text".into(), t.clone());
        self.last_text = Some(t);
    }

    /// A random number from 0 to 1.
    fn unit(&mut self) -> f64 {
        self.random(1_000_000) as f64 / 1_000_000.0
    }

    /// Where a mouse step should go, including any randomness.
    /// None means "stay where the mouse already is".
    #[allow(clippy::too_many_arguments)]
    fn resolve(
        &mut self,
        step: u64,
        target: Target,
        x: i32,
        y: i32,
        ox: i32,
        oy: i32,
        area: Option<ScreenRect>,
        jitter: i32,
    ) -> Result<Option<(i32, i32)>, String> {
        let jitter = jitter.clamp(0, 2000);
        // Pick a spot; with randomness, try again if it's the same as last time.
        for attempt in 0..8 {
            let base = match target {
                Target::Fixed => Some((x, y)),
                Target::Current if jitter == 0 => return Ok(None),
                Target::Current => Some(self.enigo.location().map_err(input_err)?),
                Target::FoundImage => Some(
                    self.found
                        .map(|(fx, fy)| (fx + ox, fy + oy))
                        .ok_or_else(|| "It aims at the found image, but no Find image step has found anything yet.".to_string())?,
                ),
                Target::Area => {
                    let r = area.ok_or_else(|| {
                        "It clicks randomly in an area, but no area is selected yet. Use “Select click area”.".to_string()
                    })?;
                    // Stay a couple of pixels inside the edge.
                    let inset = if r.w >= 8 && r.h >= 8 { 2.0 } else { 0.0 };
                    let rx = r.x as f64 + inset + self.unit() * (r.w as f64 - 1.0 - inset * 2.0).max(0.0);
                    let ry = r.y as f64 + inset + self.unit() * (r.h as f64 - 1.0 - inset * 2.0).max(0.0);
                    Some((rx.round() as i32, ry.round() as i32))
                }
            };
            let Some((bx, by)) = base else { return Ok(None) };
            let p = if jitter > 0 {
                // A random point within `jitter` pixels (evenly spread over the circle).
                let angle = self.unit() * std::f64::consts::TAU;
                let dist = self.unit().sqrt() * jitter as f64;
                (bx + (angle.cos() * dist).round() as i32, by + (angle.sin() * dist).round() as i32)
            } else {
                (bx, by)
            };
            let random = jitter > 0 || target == Target::Area;
            if !random || attempt == 7 || self.last_points.get(&step) != Some(&p) {
                if random {
                    self.last_points.insert(step, p);
                }
                return Ok(Some(p));
            }
        }
        unreachable!("the loop always returns")
    }

    /// Move the mouse to (x, y): along a curved, human-like path if that's turned
    /// on, otherwise straight there.
    fn move_to(&mut self, x: i32, y: i32) -> Result<(), String> {
        if self.doc.human_mouse {
            if let Ok(from) = self.enigo.location() {
                let (dx, dy) = ((x - from.0) as f64, (y - from.1) as f64);
                let dist = (dx * dx + dy * dy).sqrt();
                if dist >= 3.0 {
                    // Longer moves take longer, like a real hand, with some variation.
                    let speed = self.doc.mouse_speed.clamp(10, 1000) as f64 / 100.0;
                    let ms = (120.0 + dist * 0.35).min(1200.0) * (0.8 + 0.4 * self.unit()) / speed;
                    self.glide(from, (x, y), ms, true)?;
                }
            }
        }
        self.jump_to(x, y)
    }

    /// Put the mouse straight at (x, y).
    fn jump_to(&mut self, x: i32, y: i32) -> Result<(), String> {
        self.last_auto.store(pack_point(x, y), Ordering::Relaxed);
        self.enigo.move_mouse(x, y, Coordinate::Abs).map_err(input_err)
    }

    /// Move the mouse from `from` to `to` over `ms`, easing in and out. With `curved`,
    /// along a gently bent path (a different bend every time); otherwise in a line.
    fn glide(&mut self, from: (i32, i32), to: (i32, i32), ms: f64, curved: bool) -> Result<(), String> {
        let (sx, sy) = (from.0 as f64, from.1 as f64);
        let (ex, ey) = (to.0 as f64, to.1 as f64);
        let (dx, dy) = (ex - sx, ey - sy);
        let dist = (dx * dx + dy * dy).sqrt().max(1.0);
        let (c1, c2) = if curved {
            // Bend sideways by 5–17% of the distance, to a random side.
            let (nx, ny) = (-dy / dist, dx / dist);
            let side = if self.unit() < 0.5 { -1.0 } else { 1.0 };
            let bend = dist * (0.05 + 0.12 * self.unit()) * side;
            let bend2 = bend * (0.3 + 0.7 * self.unit());
            (
                (sx + dx * 0.3 + nx * bend, sy + dy * 0.3 + ny * bend),
                (sx + dx * 0.7 + nx * bend2, sy + dy * 0.7 + ny * bend2),
            )
        } else {
            ((sx + dx / 3.0, sy + dy / 3.0), (sx + dx * 2.0 / 3.0, sy + dy * 2.0 / 3.0))
        };
        let ms = ms.clamp(0.0, 10_000.0);
        let steps = ((ms / 8.0).round() as usize).clamp(4, 400);
        let pause = Duration::from_micros((ms * 1000.0 / steps as f64) as u64);
        let mut last = from;
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let e = t * t * (3.0 - 2.0 * t); // slow start, fast middle, slow end
            let u = 1.0 - e;
            let px = u * u * u * sx + 3.0 * u * u * e * c1.0 + 3.0 * u * e * e * c2.0 + e * e * e * ex;
            let py = u * u * u * sy + 3.0 * u * u * e * c1.1 + 3.0 * u * e * e * c2.1 + e * e * e * ey;
            let p = (px.round() as i32, py.round() as i32);
            if p != last {
                self.jump_to(p.0, p.1)?;
                last = p;
            }
            // Honour Stop / Pause during the move.
            self.sleep(0)?;
            thread::sleep(pause);
        }
        if last != to {
            self.jump_to(to.0, to.1)?;
        }
        Ok(())
    }

    fn send_key(&mut self, key: Key, dir: Direction) -> Result<(), String> {
        if key == Key::Escape {
            // We're pressing Esc ourselves – don't treat it as "stop".
            self.suppress_esc_until.store(now_ms() + 800, Ordering::Relaxed);
        }
        self.enigo.key(key, dir).map_err(input_err)?;
        match dir {
            Direction::Press => self.held_keys.push(key),
            Direction::Release => self.held_keys.retain(|k| *k != key),
            Direction::Click => {}
        }
        Ok(())
    }

    fn release_all(&mut self) {
        for key in std::mem::take(&mut self.held_keys).into_iter().rev() {
            let _ = self.enigo.key(key, Direction::Release);
        }
        for b in std::mem::take(&mut self.held_buttons) {
            let _ = self.enigo.button(b, Direction::Release);
        }
    }

    /// Run one step. Returns which output to follow (None = stop here).
    fn exec(&mut self, node: &Node) -> Result<Option<usize>, String> {
        match &node.kind {
            NodeKind::Start | NodeKind::GroupIn | NodeKind::Group { .. } => Ok(Some(0)),
            NodeKind::End | NodeKind::GroupOut => Ok(None),

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
                if let Some((px, py)) =
                    self.resolve(node.id, *target, *x, *y, *offset_x, *offset_y, *area, *jitter_px)?
                {
                    self.move_to(px, py)?;
                    self.sleep(25)?;
                }
                let b = to_button(*button);
                for i in 0..(*clicks).max(1) {
                    if i > 0 {
                        self.sleep(60)?;
                    }
                    self.enigo.button(b, Direction::Click).map_err(input_err)?;
                }
                Ok(Some(0))
            }

            NodeKind::MoveMouse {
                target,
                x,
                y,
                offset_x,
                offset_y,
                area,
                jitter_px,
            } => {
                if let Some((px, py)) =
                    self.resolve(node.id, *target, *x, *y, *offset_x, *offset_y, *area, *jitter_px)?
                {
                    self.move_to(px, py)?;
                }
                Ok(Some(0))
            }

            NodeKind::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
                duration_ms,
            } => {
                let b = to_button(*button);
                self.move_to(*from_x, *from_y)?;
                self.sleep(40)?;
                self.enigo.button(b, Direction::Press).map_err(input_err)?;
                self.held_buttons.push(b);
                // A short hold so apps notice the press before the move starts.
                self.sleep(80)?;
                let ms = self.scaled(*duration_ms) as f64;
                let human = self.doc.human_mouse;
                self.glide((*from_x, *from_y), (*to_x, *to_y), ms, human)?;
                self.sleep(80)?;
                self.enigo.button(b, Direction::Release).map_err(input_err)?;
                self.held_buttons.retain(|h| *h != b);
                Ok(Some(0))
            }

            NodeKind::KeyPress { key, mods } => {
                let k = keys::to_enigo(key).ok_or_else(|| format!("This computer can't send the “{key}” key."))?;
                let mod_keys = keys::modifier_keys(mods);
                for m in &mod_keys {
                    self.send_key(*m, Direction::Press)?;
                    self.sleep(8)?;
                }
                self.send_key(k, Direction::Click)?;
                for m in mod_keys.iter().rev() {
                    self.sleep(8)?;
                    self.send_key(*m, Direction::Release)?;
                }
                Ok(Some(0))
            }

            NodeKind::TypeText { text, press_enter } => {
                let text = self.fill_text(text);
                if !text.is_empty() {
                    self.enigo.text(&text).map_err(input_err)?;
                }
                if *press_enter {
                    self.sleep(20)?;
                    self.send_key(Key::Return, Direction::Click)?;
                }
                Ok(Some(0))
            }

            NodeKind::Wait { ms, jitter_ms } => {
                let extra = self.random(*jitter_ms);
                self.sleep(self.scaled(*ms + extra))?;
                Ok(Some(0))
            }

            NodeKind::Scroll { amount, horizontal, strength } => {
                // One wheel click at a time, like a finger on the wheel (a single big
                // scroll counts as just one click in many apps and games).
                let clicks = amount.unsigned_abs();
                for i in 0..clicks {
                    if i > 0 {
                        let gap = if self.doc.human_mouse { 30 + self.random(30) } else { 35 };
                        self.sleep(gap)?;
                    }
                    scroll_click(&mut self.enigo, *amount < 0, *horizontal, *strength)?;
                }
                Ok(Some(0))
            }

            NodeKind::Recording { events, speed } => {
                let sp = if *speed < 0.01 { 1.0 } else { *speed };
                for ev in events {
                    let d = (self.scaled(ev.delay_ms) as f32 / sp).round() as u64;
                    self.sleep(d)?;
                    match &ev.action {
                        RecAction::Move { x, y } => self.jump_to(*x, *y)?,
                        RecAction::Down { button, x, y } => {
                            self.jump_to(*x, *y)?;
                            let b = to_button(*button);
                            self.enigo.button(b, Direction::Press).map_err(input_err)?;
                            self.held_buttons.push(b);
                        }
                        RecAction::Up { button, x, y } => {
                            self.jump_to(*x, *y)?;
                            let b = to_button(*button);
                            self.enigo.button(b, Direction::Release).map_err(input_err)?;
                            self.held_buttons.retain(|h| *h != b);
                        }
                        RecAction::KeyDown { key } => {
                            if let Some(k) = keys::to_enigo(key) {
                                self.send_key(k, Direction::Press)?;
                            }
                        }
                        RecAction::KeyUp { key } => {
                            if let Some(k) = keys::to_enigo(key) {
                                self.send_key(k, Direction::Release)?;
                            }
                        }
                    }
                }
                self.release_all();
                Ok(Some(0))
            }

            NodeKind::FindImage {
                template,
                threshold,
                timeout_ms,
                interval_ms,
                click_when_found,
                search_area,
            } => {
                let area = search_area.map(|r| (r.x, r.y, r.w, r.h));
                let path = template
                    .as_ref()
                    .ok_or_else(|| "A Find image step has no image yet – snip or choose one.".to_string())?;
                let tpl = vision::load_template(path)?;
                // On a bigger or smaller screen, the picture is bigger or smaller too.
                let tpl = if (self.scale - 1.0).abs() > 0.03 {
                    let w = ((tpl.width() as f64 * self.scale).round() as u32).max(4);
                    let h = ((tpl.height() as f64 * self.scale).round() as u32).max(4);
                    image::imageops::resize(&tpl, w, h, image::imageops::FilterType::Triangle)
                } else {
                    tpl
                };
                let t0 = Instant::now();
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return Err(STOPPED.into());
                    }
                    let (hit, view) = vision::locate_in_seen(&tpl, *threshold, area)?;
                    let gave_up = !hit.found && t0.elapsed().as_millis() as u64 >= *timeout_ms;
                    if let (Some(view), true) = (view, hit.found || gave_up) {
                        let caption = if hit.found {
                            format!("Found at ({}, {}), {:.0}% match", hit.x, hit.y, hit.score * 100.0)
                        } else {
                            format!(
                                "Not found: the closest was a {:.0}% match (needs {:.0}%)",
                                hit.score.max(0.0) * 100.0,
                                threshold * 100.0
                            )
                        };
                        send_seen(self.tx, node.id, view, caption, hit.found);
                    }
                    if hit.found {
                        self.found = Some((hit.x, hit.y));
                        self.vars.insert("found_x".into(), hit.x.to_string());
                        self.vars.insert("found_y".into(), hit.y.to_string());
                        self.status(format!(
                            "◎ Found image at ({}, {}) – {:.0}% match",
                            hit.x,
                            hit.y,
                            hit.score * 100.0
                        ));
                        self.log(
                            LogLevel::Info,
                            format!("Found at ({}, {}), {:.0}% match", hit.x, hit.y, hit.score * 100.0),
                        );
                        if *click_when_found {
                            self.move_to(hit.x, hit.y)?;
                            self.sleep(25)?;
                            self.enigo
                                .button(Button::Left, Direction::Click)
                                .map_err(input_err)?;
                        }
                        return Ok(Some(0));
                    }
                    if t0.elapsed().as_millis() as u64 >= *timeout_ms {
                        self.status(format!(
                            "◎ Image not found (best {:.0}% match) – taking the “not found” path",
                            hit.score.max(0.0) * 100.0
                        ));
                        self.log(
                            LogLevel::Info,
                            format!("Not found – best match {:.0}%", hit.score.max(0.0) * 100.0),
                        );
                        return Ok(Some(1));
                    }
                    self.sleep((*interval_ms).max(50))?;
                }
            }

            NodeKind::WindowCheck {
                scope,
                field,
                mode,
                text,
                case_sensitive,
                timeout_ms,
                interval_ms,
                bring_to_front,
            } => {
                if text.trim().is_empty() {
                    return Err("A Window check step has no text to look for.".into());
                }
                let t0 = Instant::now();
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return Err(STOPPED.into());
                    }
                    let list = vision::list_windows()?;
                    let candidates: Vec<vision::WinInfo> = match scope {
                        WinScope::Frontmost => vision::frontmost_window(&list).into_iter().collect(),
                        WinScope::AnyOpen => list,
                    };
                    let hit = candidates.iter().find(|w| {
                        let t = win_matches(&w.title, text, *mode, *case_sensitive);
                        let a = win_matches(&w.app, text, *mode, *case_sensitive);
                        match field {
                            WinField::Title => t,
                            WinField::App => a,
                            WinField::Either => t || a,
                        }
                    });
                    if let Some(w) = hit {
                        self.status(format!("▣ Window found: {} – {}", w.app, w.title));
                        if *bring_to_front && !w.focused {
                            vision::bring_to_front(w);
                            self.sleep(300)?;
                        }
                        return Ok(Some(0));
                    }
                    if t0.elapsed().as_millis() as u64 >= *timeout_ms {
                        let now = candidates
                            .first()
                            .map(|w| format!(" (current: {} – {})", w.app, w.title))
                            .unwrap_or_default();
                        self.status(format!("▣ No matching window{now} – taking the “no” path"));
                        return Ok(Some(1));
                    }
                    self.sleep((*interval_ms).max(50))?;
                }
            }

            NodeKind::ReadText {
                x,
                y,
                w,
                h,
                compare,
                value,
                case_sensitive,
                timeout_ms,
                interval_ms,
                numbers_only,
                save_to,
                ..
            } => {
                let save = save_to.trim();
                if !save.is_empty() && !is_ident(save) {
                    return Err(format!("“{save}” can't be a variable name. Use letters, numbers and _ (e.g. gold_now)."));
                }
                let mode = if compare.is_number() || *numbers_only {
                    vision::OcrMode::Number
                } else {
                    vision::OcrMode::Text
                };
                let t0 = Instant::now();
                let mut baseline = self.prev_reads.get(&node.id).cloned();
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return Err(STOPPED.into());
                    }
                    let stop = self.stop.clone();
                    let (result, shot) = vision::read_text_seen(*x, *y, *w, *h, mode, &move || {
                        stop.load(Ordering::Relaxed)
                    })?;
                    let read = result.text.clone();
                    // "Compare with" can use variables too, e.g. {gold_before}.
                    let value = self.fill_text(value);
                    let ok = text_matches(&read, *compare, &value, *case_sensitive, baseline.as_deref())?;
                    let verdict = if ok { "match" } else { "no match" };
                    send_seen(self.tx, node.id, shot, format!("{} – {verdict}", read_caption(&result)), ok);
                    if baseline.is_none() {
                        baseline = Some(read.clone());
                    }
                    self.prev_reads.insert(node.id, read.clone());
                    self.set_last_text(read.clone());
                    let save = save_to.trim();
                    if !save.is_empty() {
                        self.vars.insert(save.to_string(), read.clone());
                    }
                    let shown: String = read.replace('\n', " ⏎ ").chars().take(60).collect();
                    if ok {
                        self.status(format!("Aa Read “{shown}” – match"));
                        self.log(LogLevel::Info, format!("Read “{shown}”"));
                        return Ok(Some(0));
                    }
                    if t0.elapsed().as_millis() as u64 >= *timeout_ms {
                        self.status(format!("Aa Read “{shown}” – no match"));
                        self.log(LogLevel::Info, format!("Read “{shown}”"));
                        return Ok(Some(1));
                    }
                    self.sleep((*interval_ms).max(100))?;
                }
            }

            NodeKind::Notify {
                title,
                message,
                style,
                position,
                duration_ms,
                wait_for_click,
                sound,
            } => {
                let toast = vision::Toast {
                    title: self.fill_text(title),
                    message: self.fill_text(message),
                    style: style.key(),
                    position: position.key(),
                    duration_ms: *duration_ms,
                    wait: *wait_for_click,
                    sound: *sound,
                };
                // A new pop-up replaces the one already showing.
                if let Some(mut old) = self.toast.take() {
                    let _ = old.kill();
                    let _ = old.wait();
                }
                let mut child = vision::show_notification(&toast)?;
                if *wait_for_click {
                    self.status("✦ Paused – click the pop-up to carry on (Esc stops)".into());
                    loop {
                        if self.stop.load(Ordering::Relaxed) {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(STOPPED.into());
                        }
                        match child.try_wait() {
                            Ok(Some(_)) | Err(_) => break,
                            Ok(None) => thread::sleep(Duration::from_millis(30)),
                        }
                    }
                } else {
                    self.toast = Some(child);
                }
                Ok(Some(0))
            }

            NodeKind::SetVar { name, expr } => {
                let name = name.trim();
                if !is_ident(name) {
                    return Err(format!(
                        "“{name}” can't be a variable name. Use letters, numbers and _ (e.g. gold_now)."
                    ));
                }
                let value = eval_expr(expr, &self.vars)?.as_text();
                self.log(LogLevel::Info, format!("{name} = {value}"));
                self.status(format!("x= {name} = {value}"));
                self.vars.insert(name.to_string(), value);
                Ok(Some(0))
            }

            NodeKind::IfCompare { left, op, right } => {
                let a = eval_expr(left, &self.vars)?;
                let b = eval_expr(right, &self.vars)?;
                let yes = compare_values(&a, *op, &b);
                self.log(
                    LogLevel::Info,
                    format!("“{}” {} “{}” → {}", a.as_text(), op.label(), b.as_text(), if yes { "yes" } else { "no" }),
                );
                Ok(Some(if yes { 0 } else { 1 }))
            }

            NodeKind::PixelColor {
                x,
                y,
                color,
                tolerance,
                timeout_ms,
                interval_ms,
            } => {
                let t0 = Instant::now();
                loop {
                    let now = vision::pixel_at(*x, *y)?;
                    let diff = color_distance(now, *color);
                    if diff <= *tolerance {
                        self.log(LogLevel::Info, format!("#{now:06X} at ({x}, {y}) – matches"));
                        return Ok(Some(0));
                    }
                    if t0.elapsed().as_millis() as u64 >= *timeout_ms {
                        self.log(
                            LogLevel::Info,
                            format!("#{now:06X} at ({x}, {y}) – off by {diff} (allowed {tolerance})"),
                        );
                        return Ok(Some(1));
                    }
                    self.sleep((*interval_ms).max(20))?;
                }
            }

            NodeKind::Loop { count } => {
                if *count == 0 {
                    return Ok(Some(0));
                }
                let left = self.loops.entry(node.id).or_insert(*count);
                if *left > 0 {
                    *left -= 1;
                    Ok(Some(0))
                } else {
                    self.loops.remove(&node.id);
                    Ok(Some(1))
                }
            }
        }
    }
}

impl Drop for Runner<'_> {
    fn drop(&mut self) {
        self.release_all();
        self.watch_done.store(true, Ordering::Relaxed);
    }
}

/// "No position yet" for `last_auto`.
const NO_POINT: u64 = u64::MAX;

fn pack_point(x: i32, y: i32) -> u64 {
    ((x as u32 as u64) << 32) | (y as u32 as u64)
}

fn unpack_point(p: u64) -> Option<(i32, i32)> {
    (p != NO_POINT).then(|| ((p >> 32) as u32 as i32, p as u32 as i32))
}

/// Watches for the Esc key while a macro plays, and sets the stop flag. With
/// `failsafe`, it also stops the macro when the mouse is pushed into an outer corner
/// of the screen (unless the macro itself put it there).
fn spawn_esc_watch(
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    suppress_until: Arc<AtomicU64>,
    failsafe: Option<(Arc<AtomicBool>, Arc<AtomicU64>)>,
) {
    thread::spawn(move || {
        let Ok(ds) = device() else {
            return;
        };
        let corners = if failsafe.is_some() { vision::screen_corners() } else { Vec::new() };
        let in_corner = |(x, y): (i32, i32)| corners.iter().any(|(cx, cy)| (x - cx).abs() <= 2 && (y - cy).abs() <= 2);
        // Only moving *into* a corner counts, so a mouse already resting in one is fine.
        let mut was_in_corner = in_corner(ds.get_mouse().coords);
        loop {
            if done.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                return;
            }
            if now_ms() >= suppress_until.load(Ordering::Relaxed)
                && ds.get_keys().contains(&Keycode::Escape)
            {
                stop.store(true, Ordering::Relaxed);
                return;
            }
            if let Some((hit, last_auto)) = &failsafe {
                let pos = ds.get_mouse().coords;
                let now_in = in_corner(pos);
                let put_there_by_macro = unpack_point(last_auto.load(Ordering::Relaxed))
                    .map_or(false, |(ax, ay)| (ax - pos.0).abs() <= 2 && (ay - pos.1).abs() <= 2);
                if now_in && !was_in_corner && !put_there_by_macro {
                    hit.store(true, Ordering::Relaxed);
                    stop.store(true, Ordering::Relaxed);
                    return;
                }
                was_in_corner = now_in;
            }
            thread::sleep(Duration::from_millis(25));
        }
    });
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

fn push(events: &mut Vec<RecEvent>, last: &mut Instant, now: Instant, action: RecAction) {
    let delay_ms = now.duration_since(*last).as_millis() as u64;
    *last = now;
    events.push(RecEvent { delay_ms, action });
}

const BUTTONS: [MouseBtn; 3] = [MouseBtn::Left, MouseBtn::Right, MouseBtn::Middle];

/// Record everything until Esc is pressed (or the stop flag is set).
pub fn record_macro(tx: Tx, stop: Arc<AtomicBool>, record_moves: bool) {
    thread::spawn(move || {
        let ds = match device() {
            Ok(d) => d,
            Err(e) => return send(&tx, WorkerMsg::Failed(e)),
        };
        wait_all_released(&ds, &stop);
        send(
            &tx,
            WorkerMsg::Status("● Recording – do your actions, then press Esc to finish.".into()),
        );

        let mut events: Vec<RecEvent> = Vec::new();
        let mut prev_keys: Vec<Keycode> = Vec::new();
        let mut prev_btn = [false; 3];
        let mut last = Instant::now();
        let mut last_move = Instant::now();
        let mut last_pos = ds.get_mouse().coords;
        let mut stopped_by_button = false;

        loop {
            if stop.load(Ordering::Relaxed) {
                stopped_by_button = true;
                break;
            }
            thread::sleep(Duration::from_millis(4));
            let keys_now = ds.get_keys();
            if keys_now.contains(&Keycode::Escape) {
                break;
            }
            let mouse = ds.get_mouse();
            let (mx, my) = mouse.coords;
            let now = Instant::now();

            if record_moves
                && (mx, my) != last_pos
                && now.duration_since(last_move) >= Duration::from_millis(15)
            {
                push(&mut events, &mut last, now, RecAction::Move { x: mx, y: my });
                last_move = now;
                last_pos = (mx, my);
            }
            for (i, b) in BUTTONS.iter().enumerate() {
                let is = mouse.button_pressed.get(i + 1).copied().unwrap_or(false);
                if is != prev_btn[i] {
                    let action = if is {
                        RecAction::Down { button: *b, x: mx, y: my }
                    } else {
                        RecAction::Up { button: *b, x: mx, y: my }
                    };
                    push(&mut events, &mut last, now, action);
                    prev_btn[i] = is;
                    last_pos = (mx, my);
                }
            }
            for k in &keys_now {
                if !prev_keys.contains(k) {
                    push(&mut events, &mut last, now, RecAction::KeyDown { key: keys::from_device(*k) });
                }
            }
            for k in &prev_keys {
                if !keys_now.contains(k) {
                    push(&mut events, &mut last, now, RecAction::KeyUp { key: keys::from_device(*k) });
                }
            }
            prev_keys = keys_now;
        }

        if stopped_by_button {
            // Drop the click on MacroFlow's own Stop button.
            if let Some(i) = events
                .iter()
                .rposition(|e| matches!(e.action, RecAction::Down { .. }))
            {
                events.truncate(i);
            }
        }
        // Never leave keys or buttons stuck down at the end of a recording.
        let mut down_keys: Vec<String> = Vec::new();
        let mut down_btns: Vec<(MouseBtn, i32, i32)> = Vec::new();
        for e in &events {
            match &e.action {
                RecAction::KeyDown { key } => down_keys.push(key.clone()),
                RecAction::KeyUp { key } => down_keys.retain(|k| k != key),
                RecAction::Down { button, x, y } => down_btns.push((*button, *x, *y)),
                RecAction::Up { button, .. } => down_btns.retain(|(b, _, _)| b != button),
                RecAction::Move { .. } => {}
            }
        }
        for key in down_keys.into_iter().rev() {
            events.push(RecEvent { delay_ms: 0, action: RecAction::KeyUp { key } });
        }
        for (button, x, y) in down_btns {
            events.push(RecEvent { delay_ms: 0, action: RecAction::Up { button, x, y } });
        }
        send(&tx, WorkerMsg::Recorded(events));
    });
}

/// Record a single click or key press and turn it into a step.
pub fn record_step(tx: Tx, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let ds = match device() {
            Ok(d) => d,
            Err(e) => return send(&tx, WorkerMsg::Failed(e)),
        };
        wait_all_released(&ds, &stop);
        send(
            &tx,
            WorkerMsg::Status("◎ Waiting for one click or key press… (Esc cancels)".into()),
        );
        let pressed = |i: usize| ds.get_mouse().button_pressed.get(i + 1).copied().unwrap_or(false);
        let wait_release = |i: usize| {
            let t0 = Instant::now();
            while pressed(i) && t0.elapsed() < Duration::from_secs(10) {
                thread::sleep(Duration::from_millis(5));
            }
        };
        loop {
            if stop.load(Ordering::Relaxed) {
                return send(&tx, WorkerMsg::Failed("Step recording cancelled.".into()));
            }
            thread::sleep(Duration::from_millis(5));
            let keys_now = ds.get_keys();
            if keys_now.contains(&Keycode::Escape) {
                return send(&tx, WorkerMsg::Failed("Step recording cancelled.".into()));
            }
            let mouse = ds.get_mouse();
            for (i, b) in BUTTONS.iter().enumerate() {
                if mouse.button_pressed.get(i + 1).copied().unwrap_or(false) {
                    let (x, y) = mouse.coords;
                    wait_release(i);
                    let mut clicks = 1;
                    let t = Instant::now();
                    while t.elapsed() < Duration::from_millis(350) {
                        if pressed(i) {
                            clicks = 2;
                            wait_release(i);
                            break;
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    return send(
                        &tx,
                        WorkerMsg::StepRecorded(NodeKind::Click {
                            target: Target::Fixed,
                            x,
                            y,
                            offset_x: 0,
                            offset_y: 0,
                            button: *b,
                            clicks,
                            area: None,
                            jitter_px: 0,
                        }),
                    );
                }
            }
            let names: Vec<String> = keys_now.iter().map(|k| keys::from_device(*k)).collect();
            if let Some(key) = names.iter().find(|n| !keys::is_modifier(n)) {
                let has = |a: &str, b: &str| names.iter().any(|n| n == a || n == b);
                let mods = Mods {
                    cmd: has("cmd", "rcmd"),
                    shift: has("shift", "rshift"),
                    alt: has("alt", "ralt"),
                    ctrl: has("ctrl", "rctrl"),
                };
                let step = NodeKind::KeyPress { key: key.clone(), mods };
                wait_all_released(&ds, &stop);
                return send(&tx, WorkerMsg::StepRecorded(step));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Helpers for the editor
// ---------------------------------------------------------------------------

/// The biggest difference between two colours' red, green or blue (0 = the same).
pub fn color_distance(a: u32, b: u32) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 0xff) as i32;
    [16, 8, 0]
        .iter()
        .map(|s| (ch(a, *s) - ch(b, *s)).unsigned_abs())
        .max()
        .unwrap_or(0)
}

/// After a countdown, report where the mouse is and the colour under it.
pub fn pick_pixel(tx: Tx, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let result = (|| -> Result<(i32, i32, u32), String> {
            let ds = device()?;
            countdown(&tx, &stop, "Hover over the spot – reading its colour", 3)?;
            let (x, y) = ds.get_mouse().coords;
            Ok((x, y, vision::pixel_at(x, y)?))
        })();
        match result {
            Ok((x, y, color)) => send(&tx, WorkerMsg::Pixel { x, y, color }),
            Err(e) => send(&tx, WorkerMsg::Failed(e)),
        }
    });
}

/// Check a Pixel colour step's spot once.
pub fn test_pixel(tx: Tx, x: i32, y: i32, color: u32, tolerance: u32, wait_ms: u64) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(wait_ms));
        let msg = match vision::pixel_at(x, y) {
            Err(e) => e,
            Ok(now) => {
                let diff = color_distance(now, color);
                let verdict = if diff <= tolerance { "MATCH" } else { "no match" };
                format!("The colour at ({x}, {y}) is #{now:06X}, off by {diff} from #{color:06X} (allowed {tolerance}) → {verdict}")
            }
        };
        send(&tx, WorkerMsg::TestResult(msg));
    });
}

/// After a countdown, report where the mouse is.
pub fn pick_position(tx: Tx, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let ds = match device() {
            Ok(d) => d,
            Err(e) => return send(&tx, WorkerMsg::Failed(e)),
        };
        if let Err(e) = countdown(&tx, &stop, "Hover over the target – capturing position", 3) {
            return send(&tx, WorkerMsg::Failed(e));
        }
        let (x, y) = ds.get_mouse().coords;
        send(&tx, WorkerMsg::Position { x, y });
    });
}

/// After a countdown, save a square image around the mouse for a Find image step.
pub fn snip(tx: Tx, stop: Arc<AtomicBool>, size: u32) {
    thread::spawn(move || {
        let result = (|| -> Result<PathBuf, String> {
            let ds = device()?;
            countdown(&tx, &stop, "Hover over the thing to find – snipping", 3)?;
            let (x, y) = ds.get_mouse().coords;
            let img = vision::snip_around(x, y, size)?;
            let dir = images_dir();
            std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
            let path = dir.join(format!("snip-{}.png", now_ms()));
            img.save(&path).map_err(|e| format!("Couldn't save the snip: {e}"))?;
            Ok(path)
        })();
        match result {
            Ok(p) => send(&tx, WorkerMsg::Snipped(p)),
            Err(e) => send(&tx, WorkerMsg::Failed(e)),
        }
    });
}

/// Search the screen once and report the result (and point the mouse at it).
pub fn test_find(
    tx: Tx,
    step: u64,
    path: PathBuf,
    threshold: f32,
    area: Option<ScreenRect>,
    scale: f64,
    wait_ms: u64,
) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(wait_ms));
        let result = (|| -> Result<String, String> {
            let tpl = vision::load_template(&path)?;
            let tpl = if (scale - 1.0).abs() > 0.03 {
                let w = ((tpl.width() as f64 * scale).round() as u32).max(4);
                let h = ((tpl.height() as f64 * scale).round() as u32).max(4);
                image::imageops::resize(&tpl, w, h, image::imageops::FilterType::Triangle)
            } else {
                tpl
            };
            let t0 = Instant::now();
            let (hit, view) = vision::locate_in_seen(&tpl, threshold, area.map(|r| (r.x, r.y, r.w, r.h)))?;
            let ms = t0.elapsed().as_millis();
            if let Some(view) = view {
                let caption = if hit.found {
                    format!("Test: found at ({}, {}), {:.0}% match", hit.x, hit.y, hit.score * 100.0)
                } else {
                    format!(
                        "Test: not found, the closest was a {:.0}% match (needs {:.0}%)",
                        hit.score.max(0.0) * 100.0,
                        threshold * 100.0
                    )
                };
                send_seen(&tx, step, view, caption, hit.found);
            }
            if hit.found {
                if let Ok(mut e) = Enigo::new(&Settings::default()) {
                    let _ = e.move_mouse(hit.x, hit.y, Coordinate::Abs);
                }
                Ok(format!(
                    "✓ Found at ({}, {}) – {:.0}% match in {ms} ms. The mouse is pointing at it.",
                    hit.x,
                    hit.y,
                    hit.score * 100.0
                ))
            } else {
                Ok(format!(
                    "✗ Not found. Best match was {:.0}% (needs {:.0}%) at ({}, {}). Try lowering the match threshold.",
                    hit.score.max(0.0) * 100.0,
                    threshold * 100.0,
                    hit.x,
                    hit.y
                ))
            }
        })();
        send(
            &tx,
            WorkerMsg::TestResult(match result {
                Ok(s) => s,
                Err(e) => e,
            }),
        );
    });
}

// ---------------------------------------------------------------------------
// Drag-a-box selection, reading text and listing windows
// ---------------------------------------------------------------------------

/// Once MacroFlow has minimised: freeze the display under the mouse and let the user
/// drag a box on it, Snipping Tool style. With `countdown_first`, wait 3 seconds first
/// so they can switch to another window.
pub fn capture_for_area(tx: Tx, stop: Arc<AtomicBool>, countdown_first: bool, purpose: AreaPurpose) {
    thread::spawn(move || {
        let result = (|| -> Result<(vision::Shot, Option<(f32, f32, f32, f32)>), String> {
            let ds = device()?;
            // Let the minimise animation finish.
            sleep_checked(700, &stop)?;
            if countdown_first {
                countdown(&tx, &stop, "Switch to the window you want – capturing the screen", 3)?;
            }
            let (x, y) = ds.get_mouse().coords;
            let shot = vision::screenshot_display_at(x, y)?;
            send(
                &tx,
                WorkerMsg::Status(
                    "Drag a box on the screen. Esc, Stop, or clicking MacroFlow cancels.".into(),
                ),
            );
            let hint = match purpose {
                AreaPurpose::ReadText(_) => "Drag a box around the text or number to read",
                AreaPurpose::Template(_) => "Drag a box around what to look for",
                AreaPurpose::SearchArea(_) => "Drag a box around where the image can appear",
                AreaPurpose::ClickArea(_) => "Drag a box around where to click",
            };
            // Esc cancels even if the selector never got keyboard focus.
            let esc_was_down = ds.get_keys().contains(&Keycode::Escape);
            let cancel = || {
                stop.load(Ordering::Relaxed)
                    || (!esc_was_down && ds.get_keys().contains(&Keycode::Escape))
            };
            let picked = match vision::select_area(&shot, hint, &cancel) {
                Ok(p) => p,
                Err(e) => {
                    let _ = std::fs::remove_file(&shot.path);
                    return Err(e);
                }
            };
            // Fractions of the display -> (x0, y0, x1, y1) on the screenshot.
            let (sw, sh) = (shot.w as f32, shot.h as f32);
            let sel = picked.map(|(x0, y0, x1, y1)| (x0 * sw, y0 * sh, x1 * sw, y1 * sh));
            Ok((shot, sel))
        })();
        match result {
            Ok((shot, sel)) => send(&tx, WorkerMsg::AreaPicked { purpose, shot, sel }),
            Err(e) => send(&tx, WorkerMsg::Failed(e)),
        }
    });
}

/// Cut the selected box out of the screenshot and save it.
pub fn crop_area(tx: Tx, shot: vision::Shot, sel: (u32, u32, u32, u32), purpose: AreaPurpose) {
    thread::spawn(move || {
        let result = (|| -> Result<PathBuf, String> {
            let to_points = matches!(purpose, AreaPurpose::Template(_));
            let img = vision::crop_shot(&shot, sel.0, sel.1, sel.2, sel.3, to_points)?;
            let dir = images_dir();
            std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
            let prefix = if to_points { "snip" } else { "area" };
            let path = dir.join(format!("{prefix}-{}.png", now_ms()));
            img.save(&path).map_err(|e| format!("Couldn't save the image: {e}"))?;
            Ok(path)
        })();
        let _ = std::fs::remove_file(&shot.path);
        match result {
            Ok(path) => send(&tx, WorkerMsg::Cropped { purpose, path }),
            Err(e) => send(&tx, WorkerMsg::Failed(e)),
        }
    });
}

/// Read a Read text step's area and say whether it matches.
/// `wait_ms` gives MacroFlow time to minimise first, so it reads what's really there.
#[allow(clippy::too_many_arguments)]
pub fn test_read(
    tx: Tx,
    step: u64,
    rect: (i32, i32, i32, i32),
    compare: TextCompare,
    value: String,
    case_sensitive: bool,
    numbers_only: bool,
    wait_ms: u64,
) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(wait_ms));
        let mode = if compare.is_number() || numbers_only {
            vision::OcrMode::Number
        } else {
            vision::OcrMode::Text
        };
        let t0 = Instant::now();
        let msg = match vision::read_text_seen(rect.0, rect.1, rect.2, rect.3, mode, &|| false) {
            Err(e) => e,
            Ok((r, shot)) => {
                let ok = compare != TextCompare::Changed
                    && text_matches(&r.text, compare, &value, case_sensitive, None).unwrap_or(false);
                send_seen(&tx, step, shot, format!("Read now: {}", read_caption(&r)), ok);
                let ms = t0.elapsed().as_millis();
                let read = r.text.clone();
                let shown = if read.is_empty() {
                    "nothing".to_string()
                } else {
                    format!("“{}”", read.replace('\n', " ⏎ "))
                };
                let confidence = if r.tries > 0 {
                    format!(" ({}/{} reads agreed)", r.agreed, r.tries)
                } else {
                    String::new()
                };
                let verdict = if compare == TextCompare::Changed {
                    " · “has changed” compares with the previous reading during a run".to_string()
                } else {
                    match text_matches(&read, compare, &value, case_sensitive, None) {
                        Ok(true) => " → MATCH".into(),
                        Ok(false) => " → no match".into(),
                        Err(e) => format!(" → {e}"),
                    }
                };
                // If the reads disagreed, show what they saw, to help pick a better box.
                let detail = if r.agreed < r.tries {
                    let seen: Vec<String> = r
                        .raw
                        .iter()
                        .map(|s| if s.is_empty() { "–".to_string() } else { format!("“{s}”") })
                        .collect();
                    format!(" · saw {}", seen.join(", "))
                } else {
                    String::new()
                };
                format!("Read {shown}{confidence} in {ms} ms{verdict}{detail}")
            }
        };
        send(&tx, WorkerMsg::TestResult(msg));
    });
}

pub fn list_windows(tx: Tx) {
    thread::spawn(move || match vision::list_windows() {
        Ok(list) => send(&tx, WorkerMsg::Windows(list)),
        Err(e) => send(&tx, WorkerMsg::Failed(e)),
    });
}
