//! Small pieces that need the operating system directly: hearing the scroll wheel
//! while recording, moving the mouse on any monitor (Windows), and the local time.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Wheel clicks heard since the last `take` (positive = down / right).
static WHEEL_V: AtomicI32 = AtomicI32::new(0);
static WHEEL_H: AtomicI32 = AtomicI32::new(0);

/// Listens to the scroll wheel while it's alive (one recording).
pub struct WheelWatch {
    stop: Arc<AtomicBool>,
}

impl WheelWatch {
    /// Start listening. None if this system can't (the recording still works, just
    /// without scrolls).
    pub fn start() -> Option<WheelWatch> {
        WHEEL_V.store(0, Ordering::Relaxed);
        WHEEL_H.store(0, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let s = stop.clone();
        thread::Builder::new()
            .name("macroflow-wheel".into())
            .spawn(move || listen(s, ready_tx))
            .ok()?;
        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(true) => Some(WheelWatch { stop }),
            _ => {
                stop.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    /// Wheel clicks since the last call: (vertical, horizontal), positive = down / right.
    pub fn take(&self) -> (i32, i32) {
        (WHEEL_V.swap(0, Ordering::Relaxed), WHEEL_H.swap(0, Ordering::Relaxed))
    }
}

impl Drop for WheelWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicPtr, Ordering};

    pub type Ref = *mut c_void;
    pub type TapCallback = extern "C" fn(Ref, u32, Ref, *mut c_void) -> Ref;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, callback: TapCallback, info: *mut c_void) -> Ref;
        pub fn CGEventTapEnable(tap: Ref, enable: bool);
        pub fn CGEventGetIntegerValueField(event: Ref, field: u32) -> i64;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFMachPortCreateRunLoopSource(alloc: *const c_void, port: Ref, order: isize) -> Ref;
        pub fn CFMachPortInvalidate(port: Ref);
        pub fn CFRunLoopGetCurrent() -> Ref;
        pub fn CFRunLoopAddSource(rl: Ref, source: Ref, mode: Ref);
        pub fn CFRunLoopRemoveSource(rl: Ref, source: Ref, mode: Ref);
        pub fn CFRunLoopRunInMode(mode: Ref, seconds: f64, return_after_source_handled: u8) -> i32;
        pub fn CFRelease(cf: *const c_void);
        pub static kCFRunLoopDefaultMode: Ref;
    }

    pub const SESSION_TAP: u32 = 1;
    pub const HEAD_INSERT: u32 = 0;
    pub const LISTEN_ONLY: u32 = 1;
    pub const SCROLL_WHEEL: u32 = 22;
    pub const DELTA_AXIS_1: u32 = 11;
    pub const DELTA_AXIS_2: u32 = 12;
    pub const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    pub const TAP_DISABLED_BY_USER: u32 = 0xFFFF_FFFF;

    pub static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

    pub extern "C" fn on_event(_proxy: Ref, kind: u32, event: Ref, _info: *mut c_void) -> Ref {
        if kind == TAP_DISABLED_BY_TIMEOUT || kind == TAP_DISABLED_BY_USER {
            let tap = TAP.load(Ordering::Relaxed);
            if !tap.is_null() {
                // SAFETY: `tap` is the live event tap created by `listen`.
                unsafe { CGEventTapEnable(tap, true) };
            }
            return event;
        }
        if kind == SCROLL_WHEEL && !event.is_null() {
            // SAFETY: `event` is the scroll event the system handed to this callback.
            let (v, h) = unsafe {
                (CGEventGetIntegerValueField(event, DELTA_AXIS_1), CGEventGetIntegerValueField(event, DELTA_AXIS_2))
            };
            // The system reports up / left as positive; MacroFlow's scrolls are the other way round.
            super::WHEEL_V.fetch_sub(v as i32, Ordering::Relaxed);
            super::WHEEL_H.fetch_sub(h as i32, Ordering::Relaxed);
        }
        event
    }
}

#[cfg(target_os = "macos")]
fn listen(stop: Arc<AtomicBool>, ready: std::sync::mpsc::Sender<bool>) {
    use mac::*;
    // SAFETY: plain CoreGraphics / CoreFoundation calls; everything created here is
    // released before the thread ends.
    unsafe {
        let tap = CGEventTapCreate(SESSION_TAP, HEAD_INSERT, LISTEN_ONLY, 1u64 << SCROLL_WHEEL, on_event, std::ptr::null_mut());
        if tap.is_null() {
            let _ = ready.send(false);
            return;
        }
        TAP.store(tap, Ordering::Relaxed);
        let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
        if source.is_null() {
            TAP.store(std::ptr::null_mut(), Ordering::Relaxed);
            CFMachPortInvalidate(tap);
            CFRelease(tap);
            let _ = ready.send(false);
            return;
        }
        let rl = CFRunLoopGetCurrent();
        CFRunLoopAddSource(rl, source, kCFRunLoopDefaultMode);
        CGEventTapEnable(tap, true);
        let _ = ready.send(true);
        while !stop.load(Ordering::Relaxed) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 0);
        }
        TAP.store(std::ptr::null_mut(), Ordering::Relaxed);
        CGEventTapEnable(tap, false);
        CFRunLoopRemoveSource(rl, source, kCFRunLoopDefaultMode);
        CFMachPortInvalidate(tap);
        CFRelease(source);
        CFRelease(tap);
    }
}

#[cfg(windows)]
fn listen(stop: Arc<AtomicBool>, ready: std::sync::mpsc::Sender<bool>) {
    use std::sync::atomic::AtomicI32;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, PeekMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
        MSG, MSLLHOOKSTRUCT, PM_REMOVE, WH_MOUSE_LL, WM_MOUSEHWHEEL, WM_MOUSEWHEEL,
    };

    /// Leftover parts of a wheel click (high-resolution wheels send less than 120).
    static REST_V: AtomicI32 = AtomicI32::new(0);
    static REST_H: AtomicI32 = AtomicI32::new(0);

    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && (wparam as u32 == WM_MOUSEWHEEL || wparam as u32 == WM_MOUSEHWHEEL) {
            // SAFETY: for WH_MOUSE_LL, lparam points at an MSLLHOOKSTRUCT.
            let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
            let delta = (info.mouseData >> 16) as u16 as i16 as i32;
            let (rest, total, sign) = if wparam as u32 == WM_MOUSEWHEEL {
                // Forward (away from you) is positive, which scrolls up.
                (&REST_V, &WHEEL_V, -1)
            } else {
                (&REST_H, &WHEEL_H, 1)
            };
            let sum = rest.load(Ordering::Relaxed) + delta;
            let clicks = sum / 120;
            rest.store(sum - clicks * 120, Ordering::Relaxed);
            if clicks != 0 {
                total.fetch_add(sign * clicks, Ordering::Relaxed);
            }
        }
        // SAFETY: passes the event on unchanged, as every hook must.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    REST_V.store(0, Ordering::Relaxed);
    REST_H.store(0, Ordering::Relaxed);
    // SAFETY: a low-level mouse hook on this thread, removed before the thread ends.
    // The thread pumps messages, which is what keeps the hook running.
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let h = SetWindowsHookExW(WH_MOUSE_LL, Some(hook), module, 0);
        if h.is_null() {
            let _ = ready.send(false);
            return;
        }
        let _ = ready.send(true);
        let mut msg: MSG = std::mem::zeroed();
        while !stop.load(Ordering::Relaxed) {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            thread::sleep(Duration::from_millis(5));
        }
        UnhookWindowsHookEx(h);
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn listen(_stop: Arc<AtomicBool>, ready: std::sync::mpsc::Sender<bool>) {
    let _ = ready.send(false);
}

/// Put the mouse at (x, y) in desktop coordinates, on whichever monitor that is.
/// enigo maps absolute moves to the main display only, so on Windows MacroFlow
/// moves the mouse itself, over the whole virtual desktop.
#[cfg(windows)]
pub fn move_mouse_abs(x: i32, y: i32) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
        MOUSEINPUT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };
    // SAFETY: plain Win32 calls with a fully initialised INPUT.
    unsafe {
        let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        let (dx, dy) = virtual_desk_coords(x, y, vx, vy, vw, vh);
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        if SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) == 1 {
            Ok(())
        } else {
            Err("Windows didn't accept the mouse move.".into())
        }
    }
}

/// (x, y) on a virtual desktop starting at (vx, vy), vw × vh pixels, as the 0…65535
/// range SendInput wants.
#[cfg_attr(not(windows), allow(dead_code))]
fn virtual_desk_coords(x: i32, y: i32, vx: i32, vy: i32, vw: i32, vh: i32) -> (i32, i32) {
    let norm = |p: i32, origin: i32, size: i32| -> i32 {
        let span = (size - 1).max(1) as i64;
        let rel = (p - origin).clamp(0, size.max(1) - 1) as i64;
        ((rel * 65535 + span / 2) / span) as i32
    };
    (norm(x, vx, vw), norm(y, vy, vh))
}

/// The local time of day: (hour, minute), and a number for the day that changes at
/// local midnight (so a daily time runs once a day).
pub fn local_time() -> (u32, u32, i64) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        // SAFETY: GetLocalTime fills the SYSTEMTIME it's given.
        let t = unsafe {
            let mut t = std::mem::zeroed();
            GetLocalTime(&mut t);
            t
        };
        let day = t.wYear as i64 * 400 + t.wMonth as i64 * 32 + t.wDay as i64;
        (t.wHour as u32, t.wMinute as u32, day)
    }
    #[cfg(unix)]
    {
        #[repr(C)]
        struct Tm {
            sec: i32,
            min: i32,
            hour: i32,
            mday: i32,
            mon: i32,
            year: i32,
            wday: i32,
            yday: i32,
            isdst: i32,
            gmtoff: std::ffi::c_long,
            zone: *const std::ffi::c_char,
        }
        extern "C" {
            fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        // SAFETY: localtime_r only writes into `tm`.
        let mut tm: Tm = unsafe { std::mem::zeroed() };
        if unsafe { localtime_r(&now, &mut tm) }.is_null() {
            let secs = now.rem_euclid(86_400);
            return ((secs / 3600) as u32, ((secs / 60) % 60) as u32, now.div_euclid(86_400));
        }
        let day = tm.year as i64 * 400 + tm.yday as i64;
        (tm.hour as u32, tm.min as u32, day)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_desktop_covers_every_monitor() {
        // Two 1920-wide monitors, the second one to the left of the main one.
        assert_eq!(virtual_desk_coords(-1920, 0, -1920, 0, 3840, 1080), (0, 0));
        assert_eq!(virtual_desk_coords(1919, 1079, -1920, 0, 3840, 1080), (65535, 65535));
        let (mid, _) = virtual_desk_coords(0, 0, -1920, 0, 3840, 1080);
        assert!((32760..=32780).contains(&mid), "{mid}");
    }

    #[test]
    fn local_time_is_a_time_of_day() {
        let (h, m, _) = local_time();
        assert!(h < 24 && m < 60);
    }
}
