//! The green dot that shows where a running macro is about to click.
//!
//! It's a tiny window that clicks pass straight through, drawn by a helper program
//! that stays running (helpers/marker.swift on macOS, helpers/marker.ps1 on Windows)
//! and takes one command per line:
//!
//!   show <x> <y> click|move   put the dot there (global mouse coordinates)
//!   pulse                     a quick ripple (the click itself), then it goes away
//!   hide                      remove it now
//!
//! It never ends up in what MacroFlow sees on screen, in two ways:
//! 1. The window is marked "exclude from screen capture" (macOS sharingType none,
//!    Windows WDA_EXCLUDEFROMCAPTURE), so screenshots skip it.
//! 2. Belt and braces: every screen capture in vision.rs calls `before_capture` first,
//!    which takes the dot down if it might be showing. So Find image, Read text,
//!    Pixel colour and failure screenshots can't see it even if (1) isn't supported.

use std::io::Write;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct Helper {
    child: Child,
    stdin: ChildStdin,
    generation: u64,
}

struct State {
    helper: Option<Helper>,
    /// The helper couldn't be started: don't keep trying (macros still run fine).
    failed: bool,
    /// The dot may be on screen until then.
    visible_until: Option<Instant>,
}

static STATE: Mutex<State> = Mutex::new(State {
    helper: None,
    failed: false,
    visible_until: None,
});
/// Which helper said "ready" (0 = none yet). Commands are only sent once it has,
/// so nothing queues up and shows late.
static READY: AtomicU64 = AtomicU64::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// How long the dot may stay up with no new command. The helper hides it sooner
/// (after 3.5 s) by itself, so this is always on the safe side.
const SHOW_FOR: Duration = Duration::from_millis(4000);
/// The click ripple: 300 ms in the helper, with some margin.
const PULSE_FOR: Duration = Duration::from_millis(450);

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Start the helper now (when a run starts), so it's ready by the first click.
pub fn prepare() {
    let mut st = lock();
    ensure_started(&mut st);
}

/// Show the dot at (x, y): filled for a click, a ring for a plain mouse move.
pub fn show(x: i32, y: i32, click: bool) {
    send(&format!("show {x} {y} {}", if click { "click" } else { "move" }), Some(SHOW_FOR));
}

/// Show the dot for a moment, then ripple (for going back over the run log).
/// Starts the helper if needed and waits for it, without holding anything up.
pub fn flash(x: i32, y: i32, click: bool) {
    std::thread::spawn(move || {
        prepare();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(2500) {
            let ready = {
                let st = lock();
                st.helper.as_ref().map_or(false, |h| READY.load(Ordering::SeqCst) == h.generation)
            };
            if ready {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        show(x, y, click);
        std::thread::sleep(Duration::from_millis(700));
        pulse();
    });
}

/// The click itself: a short ripple, then the dot goes away.
pub fn pulse() {
    if is_visible() {
        send("pulse", Some(PULSE_FOR));
    }
}

/// Take the dot down (e.g. when the run ends).
pub fn hide() {
    if is_visible() {
        send("hide", None);
    }
}

/// Called before every screen capture: make sure the dot isn't on screen.
pub fn before_capture() {
    if is_visible() {
        send("hide", None);
        // Give the window server a moment to take it off the screen.
        std::thread::sleep(Duration::from_millis(60));
    }
}

fn is_visible() -> bool {
    lock().visible_until.map_or(false, |t| Instant::now() < t)
}

fn send(cmd: &str, visible_for: Option<Duration>) {
    let mut st = lock();
    if !ensure_started(&mut st) {
        return;
    }
    let Some(h) = st.helper.as_mut() else { return };
    if READY.load(Ordering::SeqCst) != h.generation {
        // Still starting up: skip this one rather than show it late.
        return;
    }
    let ok = writeln!(h.stdin, "{cmd}").and_then(|_| h.stdin.flush()).is_ok();
    if ok {
        st.visible_until = visible_for.map(|d| Instant::now() + d);
    } else {
        // It quit: start a fresh one next time. (If it's gone, so is its dot.)
        if let Some(mut h) = st.helper.take() {
            let _ = h.child.kill();
            let _ = h.child.wait();
        }
        st.visible_until = None;
    }
}

/// Make sure a helper is running. False if there isn't one (and can't be).
fn ensure_started(st: &mut State) -> bool {
    if let Some(h) = st.helper.as_mut() {
        match h.child.try_wait() {
            Ok(None) => return true,
            _ => {
                st.helper = None;
                st.visible_until = None;
            }
        }
    }
    if st.failed {
        return false;
    }
    match start() {
        Ok(h) => {
            st.helper = Some(h);
            true
        }
        Err(_) => {
            st.failed = true;
            false
        }
    }
}

fn start() -> Result<Helper, String> {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let mut child = command()?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't start the click marker: {e}"))?;
    let stdin = child.stdin.take().ok_or("no input pipe")?;
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(out).lines() {
                match line {
                    Ok(l) if l.trim() == "ready" => READY.store(generation, Ordering::SeqCst),
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            let _ = READY.compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst);
        });
    }
    Ok(Helper { child, stdin, generation })
}

#[cfg(target_os = "macos")]
fn command() -> Result<std::process::Command, String> {
    let helper = crate::vision::mac_helper(option_env!("MACROFLOW_MARKER_HELPER"), "macroflow-marker", "click marker")?;
    Ok(std::process::Command::new(helper))
}

#[cfg(target_os = "windows")]
const WINDOWS_MARKER_SCRIPT: &str = include_str!("../helpers/marker.ps1");

#[cfg(target_os = "windows")]
fn write_script() -> Result<std::path::PathBuf, String> {
    let script = std::env::temp_dir().join("macroflow-marker.ps1");
    if std::fs::read_to_string(&script).ok().as_deref() != Some(WINDOWS_MARKER_SCRIPT) {
        std::fs::write(&script, WINDOWS_MARKER_SCRIPT).map_err(|e| format!("Couldn't write the click marker script: {e}"))?;
    }
    Ok(script)
}

#[cfg(target_os = "windows")]
fn command() -> Result<std::process::Command, String> {
    let script = write_script()?;
    let mut cmd = crate::vision::hidden_powershell();
    cmd.arg("-File").arg(script);
    Ok(cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn command() -> Result<std::process::Command, String> {
    Err("The click marker is only supported on macOS and Windows.".into())
}

/// Windows: compile the marker's window code once when MacroFlow starts, so the
/// first run doesn't wait for it.
pub fn warm_up() {
    #[cfg(target_os = "windows")]
    if let Ok(script) = write_script() {
        let _ = crate::vision::hidden_powershell()
            .arg("-File")
            .arg(&script)
            .arg("-Warmup")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
