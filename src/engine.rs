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
    RunFinished(String),
    Recorded(Vec<RecEvent>),
    StepRecorded(NodeKind),
    Position { x: i32, y: i32 },
    Snipped(PathBuf),
    TestResult(String),
    /// A box cut out of a screenshot was saved.
    Cropped { purpose: AreaPurpose, path: PathBuf },
    Windows(Vec<vision::WinInfo>),
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
}

pub type Tx = Sender<WorkerMsg>;

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

pub fn run_macro(doc: Macro, tx: Tx, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let result = Runner::new(&doc, &tx, stop.clone()).and_then(|mut r| r.run());
        send(&tx, WorkerMsg::Active(None));
        send(
            &tx,
            WorkerMsg::RunFinished(match result {
                Ok(s) => s,
                Err(e) => e,
            }),
        );
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
    loops: HashMap<u64, u32>,
    held_keys: Vec<Key>,
    held_buttons: Vec<Button>,
    suppress_esc_until: Arc<AtomicU64>,
    watch_done: Arc<AtomicBool>,
    rng: u64,
}

impl<'a> Runner<'a> {
    fn new(doc: &'a Macro, tx: &'a Tx, stop: Arc<AtomicBool>) -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default()).map_err(input_err)?;
        let suppress_esc_until = Arc::new(AtomicU64::new(0));
        let watch_done = Arc::new(AtomicBool::new(false));
        spawn_esc_watch(stop.clone(), watch_done.clone(), suppress_esc_until.clone());
        Ok(Runner {
            doc,
            tx,
            stop,
            enigo,
            found: None,
            last_text: None,
            prev_reads: HashMap::new(),
            loops: HashMap::new(),
            held_keys: Vec::new(),
            held_buttons: Vec::new(),
            suppress_esc_until,
            watch_done,
            rng: now_ms() | 1,
        })
    }

    fn sleep(&self, ms: u64) -> Result<(), String> {
        sleep_checked(ms, &self.stop)
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

    fn run(&mut self) -> Result<String, String> {
        let start = self
            .doc
            .start_id()
            .ok_or_else(|| "Add a Start step first.".to_string())?;
        if self.doc.next(start, 0).is_none() {
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
        let total = self.doc.repeat;
        let mut iteration = 0u32;
        loop {
            iteration += 1;
            self.loops.clear();
            let mut first = true;
            self.walk(start, iteration, total, &mut first)?;
            if total != 0 && iteration >= total {
                break;
            }
        }
        Ok(format!(
            "Finished – ran {iteration} time{}.",
            if iteration == 1 { "" } else { "s" }
        ))
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
                    continue;
                }
                NodeKind::GroupOut => return Ok(Exit::LeftGroup),
                _ => {}
            }
            send(self.tx, WorkerMsg::Active(Some(id)));
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
            let title = match &node.kind {
                NodeKind::Group { name, .. } if !name.trim().is_empty() => name.trim().to_string(),
                k => k.title().to_string(),
            };
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
                cur = self.doc.next(id, 0);
                continue;
            }
            if matches!(node.kind, NodeKind::End) {
                return Ok(Exit::Ended);
            }
            let port = self.exec(&node)?;
            cur = port.and_then(|p| self.doc.next(id, p));
        }
        // Ran out of connections: inside a group that means "carry on after the group".
        Ok(Exit::LeftGroup)
    }

    fn resolve(&self, target: Target, x: i32, y: i32, ox: i32, oy: i32) -> Result<Option<(i32, i32)>, String> {
        match target {
            Target::Fixed => Ok(Some((x, y))),
            Target::Current => Ok(None),
            Target::FoundImage => self
                .found
                .map(|(fx, fy)| Some((fx + ox, fy + oy)))
                .ok_or_else(|| "A step targets the found image, but no Find image step has found anything yet.".to_string()),
        }
    }

    fn move_to(&mut self, x: i32, y: i32) -> Result<(), String> {
        self.enigo.move_mouse(x, y, Coordinate::Abs).map_err(input_err)
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
            } => {
                if let Some((px, py)) = self.resolve(*target, *x, *y, *offset_x, *offset_y)? {
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
            } => {
                if let Some((px, py)) = self.resolve(*target, *x, *y, *offset_x, *offset_y)? {
                    self.move_to(px, py)?;
                }
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
                let text = match &self.last_text {
                    Some(read) => text.replace("{text}", read),
                    None => text.replace("{text}", ""),
                };
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

            NodeKind::Scroll { amount, horizontal } => {
                let axis = if *horizontal { Axis::Horizontal } else { Axis::Vertical };
                self.enigo.scroll(*amount, axis).map_err(input_err)?;
                Ok(Some(0))
            }

            NodeKind::Recording { events, speed } => {
                let sp = if *speed < 0.01 { 1.0 } else { *speed };
                for ev in events {
                    let d = (self.scaled(ev.delay_ms) as f32 / sp).round() as u64;
                    self.sleep(d)?;
                    match &ev.action {
                        RecAction::Move { x, y } => self.move_to(*x, *y)?,
                        RecAction::Down { button, x, y } => {
                            self.move_to(*x, *y)?;
                            let b = to_button(*button);
                            self.enigo.button(b, Direction::Press).map_err(input_err)?;
                            self.held_buttons.push(b);
                        }
                        RecAction::Up { button, x, y } => {
                            self.move_to(*x, *y)?;
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
                let t0 = Instant::now();
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return Err(STOPPED.into());
                    }
                    let hit = vision::locate_in(&tpl, *threshold, area)?;
                    if hit.found {
                        self.found = Some((hit.x, hit.y));
                        self.status(format!(
                            "◎ Found image at ({}, {}) – {:.0}% match",
                            hit.x,
                            hit.y,
                            hit.score * 100.0
                        ));
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
                ..
            } => {
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
                    let read = vision::read_text_sure(*x, *y, *w, *h, mode, &move || {
                        stop.load(Ordering::Relaxed)
                    })?
                    .text;
                    let ok = text_matches(&read, *compare, value, *case_sensitive, baseline.as_deref())?;
                    if baseline.is_none() {
                        baseline = Some(read.clone());
                    }
                    self.prev_reads.insert(node.id, read.clone());
                    self.last_text = Some(read.clone());
                    let shown: String = read.replace('\n', " ⏎ ").chars().take(60).collect();
                    if ok {
                        self.status(format!("Aa Read “{shown}” – match"));
                        return Ok(Some(0));
                    }
                    if t0.elapsed().as_millis() as u64 >= *timeout_ms {
                        self.status(format!("Aa Read “{shown}” – no match"));
                        return Ok(Some(1));
                    }
                    self.sleep((*interval_ms).max(100))?;
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

/// Watches for the Esc key while a macro plays, and sets the stop flag.
fn spawn_esc_watch(stop: Arc<AtomicBool>, done: Arc<AtomicBool>, suppress_until: Arc<AtomicU64>) {
    thread::spawn(move || {
        let Ok(ds) = device() else {
            return;
        };
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
pub fn test_find(tx: Tx, path: PathBuf, threshold: f32, area: Option<ScreenRect>, wait_ms: u64) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(wait_ms));
        let result = (|| -> Result<String, String> {
            let tpl = vision::load_template(&path)?;
            let t0 = Instant::now();
            let hit = vision::locate_in(&tpl, threshold, area.map(|r| (r.x, r.y, r.w, r.h)))?;
            let ms = t0.elapsed().as_millis();
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
        let msg = match vision::read_text_sure(rect.0, rect.1, rect.2, rect.3, mode, &|| false) {
            Err(e) => e,
            Ok(r) => {
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
