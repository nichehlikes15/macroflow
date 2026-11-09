//! Screen capture and image search (normalised cross-correlation template matching).
//!
//! Everything here works in the same coordinates the mouse uses: "points" on macOS
//! (Retina screenshots are scaled down to match) and physical pixels on Windows
//! (MacroFlow is DPI-aware there, so the scale is 1).

use image::{imageops, imageops::FilterType, GrayImage, RgbaImage};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use xcap::Monitor;

#[cfg(target_os = "macos")]
pub const SCREEN_PERMISSION: &str = "Couldn't capture the screen. Turn on Screen Recording for the app you launched MacroFlow from (System Settings → Privacy & Security → Screen & System Audio Recording), then restart it.";
#[cfg(not(target_os = "macos"))]
pub const SCREEN_PERMISSION: &str = "Couldn't capture the screen.";

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub found: bool,
    /// Centre of the best match, in global screen points.
    pub x: i32,
    pub y: i32,
    /// Match quality from -1 to 1 (1 = identical).
    pub score: f32,
    /// Size of the best match, in points.
    pub w: u32,
    pub h: u32,
}

struct Screen {
    ox: i32,
    oy: i32,
    /// In the coordinates the mouse uses.
    gray: GrayImage,
    /// The capture itself (on a Retina display, twice the size of `gray`).
    color: RgbaImage,
}

pub fn load_template(path: &Path) -> Result<GrayImage, String> {
    let img = image::open(path)
        .map_err(|e| format!("Couldn't open the image {}: {e}", path.display()))?;
    let gray = img.to_luma8();
    if gray.width() < 4 || gray.height() < 4 {
        return Err("That image is too small to search for.".into());
    }
    Ok(gray)
}

fn capture_all() -> Result<Vec<Screen>, String> {
    let monitors = Monitor::all().map_err(|e| format!("{SCREEN_PERMISSION} ({e})"))?;
    let mut out = Vec::new();
    for m in monitors {
        let (Ok(ox), Ok(oy), Ok(w), Ok(h)) = (m.x(), m.y(), m.width(), m.height()) else {
            continue;
        };
        let Ok(shot) = m.capture_image() else {
            continue;
        };
        if shot.width() == 0 || shot.height() == 0 {
            continue;
        }
        let mut gray = imageops::grayscale(&shot);
        if w > 0 && h > 0 && (gray.width() != w || gray.height() != h) {
            gray = imageops::resize(&gray, w, h, FilterType::Triangle);
        }
        out.push(Screen { ox, oy, gray, color: shot });
    }
    if out.is_empty() {
        return Err(SCREEN_PERMISSION.into());
    }
    Ok(out)
}

/// Search every display for the template. Also tries the template at half size,
/// so screenshots taken on a Retina display (2×) still match.
/// Capture just one area of the screen, scaled to the coordinates the mouse uses.
fn capture_search_area(x: i32, y: i32, w: i32, h: i32) -> Result<Vec<Screen>, String> {
    let img = capture_area(x, y, w, h)?;
    let mut gray = imageops::grayscale(&img);
    // macOS returns Retina pixels; bring them back to points.
    if w > 0 && h > 0 && (gray.width() != w as u32 || gray.height() != h as u32) {
        gray = imageops::resize(&gray, w as u32, h as u32, FilterType::Triangle);
    }
    Ok(vec![Screen { ox: x, oy: y, gray, color: img }])
}

/// Search for the template, either on every display or only inside `area`
/// (x, y, w, h). Searching a small area is much faster and avoids false matches elsewhere.
pub fn locate_in(template: &GrayImage, threshold: f32, area: Option<(i32, i32, i32, i32)>) -> Result<Hit, String> {
    match area {
        Some((x, y, w, h)) => {
            if (w as u32) < template.width() / 2 || (h as u32) < template.height() / 2 {
                return Err("The search area is smaller than the image to find. Select a bigger search area.".into());
            }
            locate_on(&capture_search_area(x, y, w, h)?, template, threshold)
        }
        None => locate(template, threshold),
    }
}

/// Like `locate_in`, and also returns what the search saw: the search area (or, without
/// one, the part of the screen round the best match) with a box round the best match,
/// green if it was good enough and amber if not.
pub fn locate_in_seen(
    template: &GrayImage,
    threshold: f32,
    area: Option<(i32, i32, i32, i32)>,
) -> Result<(Hit, Option<RgbaImage>), String> {
    let screens = match area {
        Some((x, y, w, h)) => {
            if (w as u32) < template.width() / 2 || (h as u32) < template.height() / 2 {
                return Err("The search area is smaller than the image to find. Select a bigger search area.".into());
            }
            capture_search_area(x, y, w, h)?
        }
        None => capture_all()?,
    };
    let hit = locate_on(&screens, template, threshold)?;
    let view = seen_view(&screens, &hit, area.is_some());
    Ok((hit, view))
}

fn seen_view(screens: &[Screen], hit: &Hit, whole: bool) -> Option<RgbaImage> {
    let s = screens.iter().find(|s| {
        hit.x >= s.ox && hit.y >= s.oy && hit.x < s.ox + s.gray.width() as i32 && hit.y < s.oy + s.gray.height() as i32
    })?;
    let (gw, gh) = (s.gray.width() as i32, s.gray.height() as i32);
    let scale = s.color.width() as f32 / gw.max(1) as f32;
    let (hx, hy) = (hit.x - s.ox, hit.y - s.oy);
    let (tw, th) = (hit.w.max(4) as i32, hit.h.max(4) as i32);
    // The part to show, in points relative to this screen.
    let (x0, y0, x1, y1) = if whole {
        (0, 0, gw, gh)
    } else {
        let (pw, ph) = ((tw * 3 / 2).max(80), (th * 3 / 2).max(50));
        ((hx - pw).max(0), (hy - ph).max(0), (hx + pw).min(gw), (hy + ph).min(gh))
    };
    if x1 - x0 < 2 || y1 - y0 < 2 {
        return None;
    }
    let px = |v: i32| (v as f32 * scale).round().max(0.0) as u32;
    let mut out = imageops::crop_imm(&s.color, px(x0), px(y0), px(x1 - x0), px(y1 - y0)).to_image();
    if hit.score > -1.0 {
        let color = if hit.found { [52, 199, 123, 255] } else { [240, 180, 41, 255] };
        let (mx, my) = (hx - tw / 2 - x0, hy - th / 2 - y0);
        draw_box(&mut out, px(mx.max(0)), px(my.max(0)), px(tw), px(th), (2.0 * scale).round().max(2.0) as u32, color);
    }
    Some(shrink_for_display(out))
}

/// Draw a hollow rectangle (clipped to the image).
fn draw_box(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, t: u32, color: [u8; 4]) {
    let (iw, ih) = img.dimensions();
    let mut put = |px: u32, py: u32| {
        if px < iw && py < ih {
            img.put_pixel(px, py, image::Rgba(color));
        }
    };
    for k in 0..t {
        for xx in x..x.saturating_add(w) {
            put(xx, y.saturating_add(k));
            put(xx, (y + h).saturating_sub(1 + k));
        }
        for yy in y..y.saturating_add(h) {
            put(x.saturating_add(k), yy);
            put((x + w).saturating_sub(1 + k), yy);
        }
    }
}

/// Keep pictures shown in the editor a sensible size (a big search area on a 5K
/// screen would be slow to save every time).
pub fn shrink_for_display(img: RgbaImage) -> RgbaImage {
    let max = 1400;
    let (w, h) = img.dimensions();
    if w <= max && h <= max {
        return img;
    }
    let f = max as f32 / w.max(h) as f32;
    imageops::resize(
        &img,
        ((w as f32 * f).round() as u32).max(1),
        ((h as f32 * f).round() as u32).max(1),
        FilterType::Triangle,
    )
}

pub fn locate(template: &GrayImage, threshold: f32) -> Result<Hit, String> {
    locate_on(&capture_all()?, template, threshold)
}

fn locate_on(screens: &[Screen], template: &GrayImage, threshold: f32) -> Result<Hit, String> {
    let mut best = Hit {
        found: false,
        x: 0,
        y: 0,
        score: -1.0,
        w: template.width(),
        h: template.height(),
    };
    let half = {
        let w = template.width() / 2;
        let h = template.height() / 2;
        if w >= 8 && h >= 8 {
            Some(imageops::resize(template, w, h, FilterType::Triangle))
        } else {
            None
        }
    };
    for s in screens {
        let mut variants: Vec<&GrayImage> = vec![template];
        if let Some(h) = &half {
            variants.push(h);
        }
        for t in variants {
            if let Some((x, y, score)) = match_template(&s.gray, t) {
                if score > best.score {
                    best = Hit {
                        found: false,
                        x: s.ox + x as i32 + (t.width() / 2) as i32,
                        y: s.oy + y as i32 + (t.height() / 2) as i32,
                        score,
                        w: t.width(),
                        h: t.height(),
                    };
                }
            }
            if best.score >= 0.98 {
                break;
            }
        }
    }
    best.found = best.score >= threshold;
    Ok(best)
}

/// Grab a square of `size` points centred on (x, y) in global screen points.
pub fn snip_around(x: i32, y: i32, size: u32) -> Result<RgbaImage, String> {
    let m = Monitor::from_point(x, y).map_err(|e| format!("{SCREEN_PERMISSION} ({e})"))?;
    let ox = m.x().map_err(|e| e.to_string())?;
    let oy = m.y().map_err(|e| e.to_string())?;
    let w = m.width().map_err(|e| e.to_string())?;
    let h = m.height().map_err(|e| e.to_string())?;
    let shot = m.capture_image().map_err(|e| format!("{SCREEN_PERMISSION} ({e})"))?;
    if shot.width() == 0 || w == 0 {
        return Err(SCREEN_PERMISSION.into());
    }
    let scale = shot.width() as f32 / w as f32;
    let size = size.min(w).min(h).max(8);
    let half = size as i32 / 2;
    let lx = (x - ox - half).clamp(0, (w - size) as i32) as u32;
    let ly = (y - oy - half).clamp(0, (h - size) as i32) as u32;
    let px = |v: u32| (v as f32 * scale).round() as u32;
    let (cx, cy) = (px(lx), px(ly));
    let cw = px(size).min(shot.width().saturating_sub(cx));
    let ch = px(size).min(shot.height().saturating_sub(cy));
    let crop = imageops::crop_imm(&shot, cx, cy, cw, ch).to_image();
    if scale > 1.01 {
        Ok(imageops::resize(&crop, size, size, FilterType::Triangle))
    } else {
        Ok(crop)
    }
}

fn unique_temp(prefix: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("macroflow-{prefix}-{n}.png"))
}

// ---------------------------------------------------------------------------
// Area selection (the "drag a box" screenshot)
// ---------------------------------------------------------------------------

/// A frozen screenshot of one display, used to drag a box on.
#[derive(Clone, Debug)]
pub struct Shot {
    pub path: PathBuf,
    /// The display's system name (e.g. \\.\DISPLAY1 on Windows), so the selector
    /// covers the same display the screenshot came from.
    pub device: String,
    /// Display origin and size, in global points.
    pub ox: i32,
    pub oy: i32,
    pub w: u32,
    pub h: u32,
}

/// Screenshot the display under (x, y) and save it to a temporary file.
pub fn screenshot_display_at(x: i32, y: i32) -> Result<Shot, String> {
    let m = Monitor::from_point(x, y)
        .or_else(|_| {
            Monitor::all()
                .map_err(|e| e.to_string())?
                .into_iter()
                .next()
                .ok_or_else(|| "No display found.".to_string())
        })
        .map_err(|e| format!("{SCREEN_PERMISSION} ({e})"))?;
    let ox = m.x().map_err(|e| e.to_string())?;
    let oy = m.y().map_err(|e| e.to_string())?;
    let w = m.width().map_err(|e| e.to_string())?;
    let h = m.height().map_err(|e| e.to_string())?;
    let img = m.capture_image().map_err(|e| format!("{SCREEN_PERMISSION} ({e})"))?;
    if img.width() == 0 || img.height() == 0 {
        return Err(SCREEN_PERMISSION.into());
    }
    let path = unique_temp("shot");
    img.save(&path).map_err(|e| format!("Couldn't save the screenshot: {e}"))?;
    Ok(Shot {
        path,
        device: m.name().unwrap_or_default(),
        ox,
        oy,
        w,
        h,
    })
}

/// Cut a box (in the display's local points) out of a saved screenshot.
/// With `to_points`, the result is scaled down to point size (what Find image searches in);
/// otherwise it's kept at full Retina resolution.
pub fn crop_shot(shot: &Shot, x: u32, y: u32, w: u32, h: u32, to_points: bool) -> Result<RgbaImage, String> {
    let img = image::open(&shot.path)
        .map_err(|e| format!("Couldn't open the screenshot: {e}"))?
        .to_rgba8();
    let scale = img.width() as f32 / shot.w.max(1) as f32;
    let px = |v: u32| (v as f32 * scale).round() as u32;
    let cx = px(x).min(img.width().saturating_sub(1));
    let cy = px(y).min(img.height().saturating_sub(1));
    let cw = px(w).max(1).min(img.width() - cx);
    let ch = px(h).max(1).min(img.height() - cy);
    let crop = imageops::crop_imm(&img, cx, cy, cw, ch).to_image();
    if to_points && scale > 1.01 {
        Ok(imageops::resize(&crop, w.max(1), h.max(1), FilterType::Triangle))
    } else {
        Ok(crop)
    }
}

/// Exit codes shared by the selector helpers.
const SELECT_CANCELLED: i32 = 3;

/// Let the user drag a box on a frozen copy of the display in `shot`, Snipping Tool style.
/// Returns the box as fractions of the display (x0, y0, x1, y1, each 0–1), or None if
/// they cancelled. `cancel` is polled while the selector is open; when it returns true
/// the selector is closed (Stop button, Esc, or clicking back on MacroFlow).
pub fn select_area(
    shot: &Shot,
    hint: &str,
    cancel: &dyn Fn() -> bool,
) -> Result<Option<(f32, f32, f32, f32)>, String> {
    let mut child = selector_command(shot, hint)?
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't open the area selector: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    let status = loop {
        if cancel() || std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(30)),
            Err(e) => {
                let _ = child.kill();
                return Err(format!("The area selector stopped responding: {e}"));
            }
        }
    };
    let out = child
        .wait_with_output()
        .map_err(|e| format!("Couldn't read the area selector's answer: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if status.code() == Some(SELECT_CANCELLED) {
        return Ok(None);
    }
    if !status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let detail = err
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .last()
            .unwrap_or("unknown error")
            .to_string();
        return Err(format!("The area selector failed: {detail}"));
    }
    // Expected: "OK x0 y0 x1 y1" with fractions of the display.
    let nums: Vec<f32> = stdout
        .split_whitespace()
        .filter_map(|t| t.parse::<f32>().ok())
        .collect();
    match nums.as_slice() {
        [x0, y0, x1, y1, ..] if x1 > x0 && y1 > y0 => Ok(Some((
            x0.clamp(0.0, 1.0),
            y0.clamp(0.0, 1.0),
            x1.clamp(0.0, 1.0),
            y1.clamp(0.0, 1.0),
        ))),
        _ => Ok(None),
    }
}

#[cfg(target_os = "macos")]
fn selector_command(shot: &Shot, hint: &str) -> Result<Command, String> {
    let helper = mac_helper(
        option_env!("MACROFLOW_SELECT_HELPER"),
        "macroflow-select",
        "area selector",
    )?;
    let mut cmd = Command::new(helper);
    cmd.arg(&shot.path).args([
        shot.ox.to_string(),
        shot.oy.to_string(),
        shot.w.to_string(),
        shot.h.to_string(),
        hint.to_string(),
    ]);
    Ok(cmd)
}

#[cfg(target_os = "windows")]
const WINDOWS_SELECT_SCRIPT: &str = include_str!("../helpers/select.ps1");

#[cfg(target_os = "windows")]
fn write_select_script() -> Result<PathBuf, String> {
    let script = std::env::temp_dir().join("macroflow-select.ps1");
    std::fs::write(&script, WINDOWS_SELECT_SCRIPT)
        .map_err(|e| format!("Couldn't write the area selector script: {e}"))?;
    Ok(script)
}

#[cfg(target_os = "windows")]
fn selector_command(shot: &Shot, hint: &str) -> Result<Command, String> {
    let script = write_select_script()?;
    let mut cmd = hidden_powershell();
    cmd.arg("-File")
        .arg(&script)
        .arg("-ImagePath")
        .arg(&shot.path)
        .arg("-Device")
        .arg(if shot.device.is_empty() { "-" } else { shot.device.as_str() })
        .arg("-X")
        .arg(shot.ox.to_string())
        .arg("-Y")
        .arg(shot.oy.to_string())
        .arg("-W")
        .arg(shot.w.to_string())
        .arg("-H")
        .arg(shot.h.to_string())
        .arg("-Hint")
        .arg(hint);
    Ok(cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn selector_command(_shot: &Shot, _hint: &str) -> Result<Command, String> {
    Err("Selecting an area is only supported on macOS and Windows.".into())
}

/// Windows: compile the area selector and the pop-up once in the background when
/// MacroFlow starts, so the first "Select area" or Pop-up appears straight away
/// instead of after a few seconds.
pub fn warm_up_selector() {
    #[cfg(target_os = "windows")]
    std::thread::spawn(|| {
        let scripts = [write_select_script(), write_notify_script()];
        for script in scripts.into_iter().flatten() {
            let _ = hidden_powershell()
                .arg("-File")
                .arg(&script)
                .arg("-Warmup")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    });
}

// ---------------------------------------------------------------------------
// Pop-up notifications
// ---------------------------------------------------------------------------

/// A pop-up card to show at the top of the screen.
#[derive(Clone, Debug)]
pub struct Toast {
    pub title: String,
    pub message: String,
    /// "info" | "success" | "warning" | "error"
    pub style: &'static str,
    /// "top-center" | "top-right"
    pub position: &'static str,
    pub duration_ms: u64,
    /// Stay until clicked.
    pub wait: bool,
    pub sound: bool,
}

/// Show a pop-up without taking focus from the app you're using. Returns the helper
/// process: it exits when the pop-up closes (timed out or clicked), and killing it
/// removes the pop-up straight away.
pub fn show_notification(t: &Toast) -> Result<std::process::Child, String> {
    notify_command(t)?
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't show the pop-up: {e}"))
}

#[cfg(target_os = "macos")]
fn notify_command(t: &Toast) -> Result<Command, String> {
    let helper = mac_helper(
        option_env!("MACROFLOW_NOTIFY_HELPER"),
        "macroflow-notify",
        "pop-up helper",
    )?;
    let mut cmd = Command::new(helper);
    cmd.arg(&t.title)
        .arg(&t.message)
        .arg(t.style)
        .arg(t.position)
        .arg(t.duration_ms.to_string())
        .arg(if t.wait { "1" } else { "0" })
        .arg(if t.sound { "1" } else { "0" });
    Ok(cmd)
}

#[cfg(target_os = "windows")]
const WINDOWS_NOTIFY_SCRIPT: &str = include_str!("../helpers/notify.ps1");

#[cfg(target_os = "windows")]
fn write_notify_script() -> Result<PathBuf, String> {
    let script = std::env::temp_dir().join("macroflow-notify.ps1");
    // Only rewrite it when it changed: pop-ups can start close together.
    if std::fs::read_to_string(&script).ok().as_deref() != Some(WINDOWS_NOTIFY_SCRIPT) {
        std::fs::write(&script, WINDOWS_NOTIFY_SCRIPT)
            .map_err(|e| format!("Couldn't write the pop-up script: {e}"))?;
    }
    Ok(script)
}

/// "b64:" + base64 of the UTF-8 text, so any characters (and empty text) survive
/// being passed to PowerShell on the command line.
#[cfg(target_os = "windows")]
fn b64_arg(text: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut out = String::from("b64:");
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(target_os = "windows")]
fn notify_command(t: &Toast) -> Result<Command, String> {
    let script = write_notify_script()?;
    let mut cmd = hidden_powershell();
    cmd.arg("-File")
        .arg(&script)
        .arg("-Title")
        .arg(b64_arg(&t.title))
        .arg("-Message")
        .arg(b64_arg(&t.message))
        .arg("-Style")
        .arg(t.style)
        .arg("-Position")
        .arg(t.position)
        .arg("-DurationMs")
        .arg(t.duration_ms.to_string());
    if t.wait {
        cmd.arg("-Wait");
    }
    if t.sound {
        cmd.arg("-Sound");
    }
    Ok(cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn notify_command(_t: &Toast) -> Result<Command, String> {
    Err("Pop-ups are only supported on macOS and Windows.".into())
}

/// The outer corners of the whole desktop (corners where one display meets another
/// are left out, since the mouse can pass straight through those).
pub fn screen_corners() -> Vec<(i32, i32)> {
    let rects: Vec<(i32, i32, i32, i32)> = Monitor::all()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            Some((
                m.x().ok()?,
                m.y().ok()?,
                m.width().ok()? as i32,
                m.height().ok()? as i32,
            ))
        })
        .filter(|r| r.2 > 0 && r.3 > 0)
        .collect();
    let on_screen = |x: i32, y: i32| rects.iter().any(|(rx, ry, rw, rh)| x >= *rx && x < rx + rw && y >= *ry && y < ry + rh);
    let mut out = Vec::new();
    for (x, y, w, h) in &rects {
        for (cx, cy, ox, oy) in [
            (*x, *y, -1, -1),
            (x + w - 1, *y, 1, -1),
            (*x, y + h - 1, -1, 1),
            (x + w - 1, y + h - 1, 1, 1),
        ] {
            // Outer only if there's no display beyond it in either direction.
            let beyond = on_screen(cx + ox * 3, cy) || on_screen(cx, cy + oy * 3) || on_screen(cx + ox * 3, cy + oy * 3);
            if !beyond {
                out.push((cx, cy));
            }
        }
    }
    out
}

/// The colour (0xRRGGBB) of one spot on the screen.
pub fn pixel_at(x: i32, y: i32) -> Result<u32, String> {
    let m = Monitor::from_point(x, y).map_err(|e| format!("({x}, {y}) isn't on any display ({e})."))?;
    let lx = (x - m.x().map_err(|e| e.to_string())?).max(0) as u32;
    let ly = (y - m.y().map_err(|e| e.to_string())?).max(0) as u32;
    let mw = m.width().map_err(|e| e.to_string())?;
    let mh = m.height().map_err(|e| e.to_string())?;
    let img = m
        .capture_region(lx.min(mw.saturating_sub(1)), ly.min(mh.saturating_sub(1)), 1, 1)
        .map_err(|e| format!("{SCREEN_PERMISSION} ({e})"))?;
    if img.width() == 0 || img.height() == 0 {
        return Err(SCREEN_PERMISSION.into());
    }
    let p = img.get_pixel(0, 0).0;
    Ok(((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32)
}

/// Capture a live area of the screen (global points) at full resolution.
pub fn capture_area(x: i32, y: i32, w: i32, h: i32) -> Result<RgbaImage, String> {
    if w < 2 || h < 2 {
        return Err("Select an area on screen first.".into());
    }
    let m = Monitor::from_point(x, y).map_err(|e| format!("That area isn't on any display ({e})."))?;
    let ox = m.x().map_err(|e| e.to_string())?;
    let oy = m.y().map_err(|e| e.to_string())?;
    let mw = m.width().map_err(|e| e.to_string())? as i32;
    let mh = m.height().map_err(|e| e.to_string())? as i32;
    let lx = (x - ox).clamp(0, mw - 1);
    let ly = (y - oy).clamp(0, mh - 1);
    let lw = w.min(mw - lx).max(1);
    let lh = h.min(mh - ly).max(1);
    m.capture_region(lx as u32, ly as u32, lw as u32, lh as u32)
        .map_err(|e| format!("{SCREEN_PERMISSION} ({e})"))
}

// ---------------------------------------------------------------------------
// Reading text (OCR)
// ---------------------------------------------------------------------------

/// Find a helper built by build.rs (or a copy next to the executable, for a packaged app).
#[cfg(target_os = "macos")]
fn mac_helper(built: Option<&'static str>, name: &str, what: &str) -> Result<PathBuf, String> {
    let built = built.unwrap_or("");
    if !built.is_empty() && Path::new(built).exists() {
        return Ok(PathBuf::from(built));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(name);
            if p.exists() {
                return Ok(p);
            }
        }
    }
    Err(format!(
        "The {what} isn't built. Run `xcode-select --install`, then `cargo build --release` again."
    ))
}

#[cfg(target_os = "macos")]
fn ocr_helper() -> Result<PathBuf, String> {
    mac_helper(option_env!("MACROFLOW_OCR_HELPER"), "macroflow-ocr", "text reader")
}

/// Printed by the OCR helpers between the results for each image.
const OCR_SEPARATOR: &str = "<<<MACROFLOW-NEXT>>>";

fn split_ocr_output(stdout: &str, count: usize) -> Vec<String> {
    let mut parts: Vec<String> = stdout
        .split(OCR_SEPARATOR)
        .map(|p| p.trim().to_string())
        .collect();
    parts.resize(count, String::new());
    parts.truncate(count);
    parts
}

/// macOS: Apple Vision, through the small Swift helper built by build.rs.
#[cfg(target_os = "macos")]
fn run_ocr_batch(paths: &[PathBuf]) -> Result<Vec<String>, String> {
    let out = Command::new(ocr_helper()?)
        .args(paths)
        .output()
        .map_err(|e| format!("Couldn't run the text reader: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "The text reader failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(split_ocr_output(&String::from_utf8_lossy(&out.stdout), paths.len()))
}

/// Windows: the built-in Windows OCR engine (Windows.Media.Ocr), through PowerShell.
#[cfg(target_os = "windows")]
const WINDOWS_OCR_SCRIPT: &str = include_str!("../helpers/ocr.ps1");

#[cfg(target_os = "windows")]
fn hidden_powershell() -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"])
        .creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(target_os = "windows")]
fn run_ocr_batch(paths: &[PathBuf]) -> Result<Vec<String>, String> {
    let script = std::env::temp_dir().join("macroflow-ocr.ps1");
    // Rewrite every time so an updated script is always used.
    std::fs::write(&script, WINDOWS_OCR_SCRIPT).map_err(|e| format!("Couldn't write the OCR script: {e}"))?;
    let out = hidden_powershell()
        .arg("-File")
        .arg(&script)
        .args(paths)
        .output()
        .map_err(|e| format!("Couldn't run PowerShell for text reading: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "The text reader failed: {}. Windows needs an OCR language pack (Settings → Time & language → Language) for your display language.",
            err.trim().lines().next().unwrap_or("unknown error")
        ));
    }
    Ok(split_ocr_output(&String::from_utf8_lossy(&out.stdout), paths.len()))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn run_ocr_batch(_paths: &[PathBuf]) -> Result<Vec<String>, String> {
    Err("Reading text is only supported on macOS and Windows.".into())
}

/// What to read the area as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrMode {
    Text,
    /// Clean the result up into a single number ("1O0" → "100", "12 345" → "12345").
    Number,
}

/// The result of reading an area.
#[derive(Clone, Debug)]
pub struct OcrRead {
    /// What was read (for numbers, the cleaned-up number, e.g. "-12345.5").
    pub text: String,
    /// How many of the reads agreed on it, out of `tries`.
    pub agreed: usize,
    pub tries: usize,
    /// Everything each read came back with, for "Read now" feedback.
    pub raw: Vec<String>,
}

// --- image clean-up --------------------------------------------------------

fn histogram(g: &GrayImage) -> [u32; 256] {
    let mut h = [0u32; 256];
    for p in g.as_raw() {
        h[*p as usize] += 1;
    }
    h
}

/// Stretch the brightness range so faint text on a similar background stands out.
fn stretch_contrast(g: &GrayImage) -> GrayImage {
    let h = histogram(g);
    let total: u32 = h.iter().sum();
    let cut = total / 50; // ignore the darkest / brightest 2%
    let (mut lo, mut hi) = (0usize, 255usize);
    let mut acc = 0;
    for (i, c) in h.iter().enumerate() {
        acc += c;
        if acc > cut {
            lo = i;
            break;
        }
    }
    acc = 0;
    for (i, c) in h.iter().enumerate().rev() {
        acc += c;
        if acc > cut {
            hi = i;
            break;
        }
    }
    if hi <= lo + 10 {
        return g.clone();
    }
    let (lo, span) = (lo as f32, (hi - lo) as f32);
    let mut out = g.clone();
    for p in out.pixels_mut() {
        p.0[0] = (((p.0[0] as f32 - lo) / span) * 255.0).clamp(0.0, 255.0) as u8;
    }
    out
}

/// Average brightness of the outer edge: in a box drawn round some text, that's the background.
fn border_mean(g: &GrayImage) -> f32 {
    let (w, h) = g.dimensions();
    if w < 3 || h < 3 {
        return 255.0;
    }
    let (mut sum, mut n) = (0f64, 0f64);
    for x in 0..w {
        for y in [0, 1, h - 2, h - 1] {
            sum += g.get_pixel(x, y).0[0] as f64;
            n += 1.0;
        }
    }
    for y in 0..h {
        for x in [0, 1, w - 2, w - 1] {
            sum += g.get_pixel(x, y).0[0] as f64;
            n += 1.0;
        }
    }
    (sum / n) as f32
}

/// Black-and-white version using Otsu's threshold.
fn binarize(g: &GrayImage) -> GrayImage {
    let h = histogram(g);
    let total: f64 = h.iter().map(|c| *c as f64).sum();
    let sum_all: f64 = h.iter().enumerate().map(|(i, c)| i as f64 * *c as f64).sum();
    let (mut sum_b, mut w_b, mut best, mut threshold) = (0f64, 0f64, 0f64, 128u8);
    for (t, c) in h.iter().enumerate() {
        w_b += *c as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += t as f64 * *c as f64;
        let (m_b, m_f) = (sum_b / w_b, (sum_all - sum_b) / w_f);
        let between = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if between > best {
            best = between;
            threshold = t as u8;
        }
    }
    let mut out = g.clone();
    for p in out.pixels_mut() {
        p.0[0] = if p.0[0] > threshold { 255 } else { 0 };
    }
    out
}

/// Surround with a white margin: OCR often drops characters that touch the image edge.
fn pad_white(g: &GrayImage, pad: u32) -> GrayImage {
    let (w, h) = g.dimensions();
    let mut out = GrayImage::from_pixel(w + pad * 2, h + pad * 2, image::Luma([255]));
    imageops::overlay(&mut out, g, pad as i64, pad as i64);
    out
}

fn scaled(g: &GrayImage, f: f32) -> GrayImage {
    if (f - 1.0).abs() < 0.05 {
        return g.clone();
    }
    let (w, h) = g.dimensions();
    let nw = ((w as f32 * f).round() as u32).max(1);
    let nh = ((h as f32 * f).round() as u32).max(1);
    imageops::resize(g, nw, nh, FilterType::CatmullRom)
}

/// A few cleaned-up versions of the area to read: dark text on a light background,
/// enlarged, with a margin. Reading several versions and voting is far more reliable
/// than one read of the raw capture.
fn ocr_variants(img: &RgbaImage, mode: OcrMode) -> Vec<GrayImage> {
    let gray = imageops::grayscale(img);
    let (w, h) = gray.dimensions();
    // OCR works best when text is roughly 40-80 px tall. A box drawn round a line of
    // text is a bit taller than the text, so aim the box at ~120-150 px.
    let target = if mode == OcrMode::Number { 150.0 } else { 120.0 };
    let max_side = 2200.0; // stays well inside both OCR engines' limits
    let mut f = (target / h.max(1) as f32).clamp(1.0, 8.0);
    f = f.min(max_side / w.max(h).max(1) as f32).max(0.25);

    let base = stretch_contrast(&scaled(&gray, f));
    // Light text on a dark background (common in games and dark themes): flip it.
    let base = if border_mean(&base) < 128.0 {
        let mut inv = base;
        imageops::invert(&mut inv);
        inv
    } else {
        base
    };
    let pad = (base.height() / 3).clamp(16, 120);
    let mut out = vec![pad_white(&base, pad), pad_white(&binarize(&base), pad)];
    // The same thing a bit smaller: engines sometimes split or merge characters at one
    // size but not another.
    let smaller = scaled(&base, 0.6);
    if smaller.height() >= 32 {
        out.push(pad_white(&smaller, (smaller.height() / 3).clamp(16, 120)));
    }
    // And a bit bigger: narrow digits like a lone "1" in "1 892 039" are easier to see.
    let bigger = scaled(&base, 1.5);
    if bigger.width().max(bigger.height()) as f32 <= max_side {
        out.push(pad_white(&bigger, (bigger.height() / 3).clamp(16, 160)));
    }
    out
}

fn normalise_text(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Read the text inside an image.
pub fn ocr_image(img: &RgbaImage, mode: OcrMode) -> Result<OcrRead, String> {
    let variants = ocr_variants(img, mode);
    let mut paths = Vec::new();
    for (i, v) in variants.iter().enumerate() {
        let path = unique_temp(&format!("ocr{i}"));
        // Save as RGB: every OCR engine accepts that.
        image::DynamicImage::ImageLuma8(v.clone())
            .to_rgb8()
            .save(&path)
            .map_err(|e| format!("Couldn't save the image to read: {e}"))?;
        paths.push(path);
    }
    let result = run_ocr_batch(&paths);
    for p in &paths {
        let _ = std::fs::remove_file(p);
    }
    let raw: Vec<String> = result?.iter().map(|r| normalise_text(r)).collect();
    Ok(vote(raw, mode))
}

/// Pick the answer most of the reads agree on.
fn vote(raw: Vec<String>, mode: OcrMode) -> OcrRead {
    let answers: Vec<String> = raw
        .iter()
        .map(|r| match mode {
            OcrMode::Text => r.clone(),
            OcrMode::Number => crate::model::clean_number(r).unwrap_or_default(),
        })
        .collect();
    // Most common non-empty answer wins; ties go to the longest, then to the earlier,
    // cleaner version. For numbers, a read that's part of a longer read ("892" inside
    // "892390", "892039" inside "1892039") counts as agreeing with the longer one:
    // OCR drops digits far more often than it invents them.
    let mut best: Option<(usize, usize, usize, String)> = None; // (votes, len, -order, text)
    for (order, a) in answers.iter().enumerate() {
        if a.is_empty() {
            continue;
        }
        let votes = answers
            .iter()
            .filter(|b| {
                *b == a
                    || (mode == OcrMode::Number && !b.is_empty() && b.len() < a.len() && a.contains(b.as_str()))
            })
            .count();
        let len = a.chars().count();
        let key = (votes, len, usize::MAX - order);
        if best.as_ref().map_or(true, |(v, l, o, _)| key > (*v, *l, *o)) {
            best = Some((votes, len, usize::MAX - order, a.clone()));
        }
    }
    let (agreed, text) = best.map(|(v, _, _, t)| (v, t)).unwrap_or((0, String::new()));
    OcrRead {
        text,
        agreed,
        tries: answers.len(),
        raw,
    }
}

/// Read the text in an area of the screen (global points), once.
pub fn read_text(x: i32, y: i32, w: i32, h: i32, mode: OcrMode) -> Result<OcrRead, String> {
    ocr_image(&capture_area(x, y, w, h)?, mode)
}

/// Read an area until at least two reads agree (up to 3 rounds), so one bad read
/// can't decide a branch. `stop` is checked between rounds.
pub fn read_text_sure(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    mode: OcrMode,
    stop: &dyn Fn() -> bool,
) -> Result<OcrRead, String> {
    read_text_seen(x, y, w, h, mode, stop).map(|(r, _)| r)
}

/// Like `read_text_sure`, and also returns the capture of the area that was read
/// (the last one, if it read more than once).
pub fn read_text_seen(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    mode: OcrMode,
    stop: &dyn Fn() -> bool,
) -> Result<(OcrRead, RgbaImage), String> {
    let mut pooled: Vec<String> = Vec::new();
    let mut result = None;
    for _ in 0..3 {
        let shot = capture_area(x, y, w, h)?;
        let r = ocr_image(&shot, mode)?;
        pooled.extend(r.raw);
        let v = vote(pooled.clone(), mode);
        let sure = v.agreed >= 2;
        result = Some((v, shot));
        if sure || stop() {
            break;
        }
    }
    Ok(result.expect("at least one read"))
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct WinInfo {
    pub app: String,
    pub title: String,
    /// Belongs to the app you're currently using.
    pub focused: bool,
    /// Top-left corner and size, in the same coordinates the mouse uses.
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Windows that are open on screen, front-most first.
/// (Minimised windows, and on macOS windows on other Spaces, aren't included.)
/// On macOS, window titles need Screen Recording permission; without it only app names are known.
pub fn list_windows() -> Result<Vec<WinInfo>, String> {
    let windows = xcap::Window::all().map_err(|e| format!("Couldn't list windows: {e}"))?;
    let mut out = Vec::new();
    for w in windows {
        let app = w.app_name().unwrap_or_default();
        let skip = [
            "Window Server",
            "Dock",
            "Control Centre",
            "Control Center",
            "Notification Centre",
            "Notification Center",
            "Program Manager",
            "Windows Input Experience",
            "TextInputHost",
        ];
        if app.is_empty() || skip.iter().any(|s| app.eq_ignore_ascii_case(s)) {
            continue;
        }
        if w.is_minimized().unwrap_or(false) {
            continue;
        }
        let (ww, wh) = (w.width().unwrap_or(0), w.height().unwrap_or(0));
        if ww < 60 || wh < 40 {
            continue;
        }
        out.push(WinInfo {
            title: w.title().unwrap_or_default(),
            focused: w.is_focused().unwrap_or(false),
            app,
            x: w.x().unwrap_or(0),
            y: w.y().unwrap_or(0),
            w: ww,
            h: wh,
        });
    }
    Ok(out)
}

/// The open window a macro follows: same app and title if possible, otherwise the
/// front-most window of the same app (titles often change, e.g. in browsers).
pub fn find_anchor_window(a: &crate::model::WindowAnchor) -> Option<WinInfo> {
    let list = list_windows().ok()?;
    let same_app: Vec<&WinInfo> = list.iter().filter(|w| w.app.eq_ignore_ascii_case(&a.app)).collect();
    same_app
        .iter()
        .find(|w| w.title == a.title)
        .or_else(|| same_app.first())
        .map(|w| (*w).clone())
}

fn missing_window(a: &crate::model::WindowAnchor) -> String {
    let name = if a.title.is_empty() {
        a.app.clone()
    } else {
        format!("{} – {}", a.app, a.title)
    };
    format!("This macro follows the window “{name}”, but it isn't open (or it's minimised).")
}

/// How far the followed window has moved since the macro's positions were saved.
pub fn anchor_offset(a: &crate::model::WindowAnchor) -> Result<(i32, i32), String> {
    let w = find_anchor_window(a).ok_or_else(|| missing_window(a))?;
    Ok((w.x - a.x, w.y - a.y))
}

/// The followed window's position and size right now.
pub fn anchor_rect(a: &crate::model::WindowAnchor) -> Result<crate::model::ScreenRect, String> {
    let w = find_anchor_window(a).ok_or_else(|| missing_window(a))?;
    Ok(crate::model::ScreenRect {
        x: w.x,
        y: w.y,
        w: w.w as i32,
        h: w.h as i32,
    })
}

/// The main display's position and size, in the coordinates the mouse uses.
pub fn primary_screen() -> Result<crate::model::ScreenRect, String> {
    let monitors = Monitor::all().map_err(|e| format!("Couldn't read the screen size ({e})."))?;
    let m = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .ok_or_else(|| "No display found.".to_string())?;
    Ok(crate::model::ScreenRect {
        x: m.x().map_err(|e| e.to_string())?,
        y: m.y().map_err(|e| e.to_string())?,
        w: m.width().map_err(|e| e.to_string())? as i32,
        h: m.height().map_err(|e| e.to_string())? as i32,
    })
}

/// The screen area a macro's saved positions should be fitted to right now:
/// the followed window if there is one, otherwise the main display.
pub fn target_rect(anchor: Option<&crate::model::WindowAnchor>) -> Result<crate::model::ScreenRect, String> {
    match anchor {
        Some(a) => anchor_rect(a),
        None => primary_screen(),
    }
}

/// How to turn a macro's saved positions into positions on this screen right now
/// (None = they're already exact screen positions).
pub fn current_mapping(
    reference: Option<crate::model::ScreenRect>,
    anchor: Option<&crate::model::WindowAnchor>,
) -> Result<Option<crate::model::Mapping>, String> {
    use crate::model::Mapping;
    match (reference, anchor) {
        (None, None) => Ok(None),
        // Following a window without scaling: just move with it.
        (None, Some(a)) => {
            let (dx, dy) = anchor_offset(a)?;
            Ok(Some(Mapping::translate(dx, dy)))
        }
        // Scaled: fit the saved screen / window onto the current one.
        (Some(from), a) => Ok(Some(Mapping {
            from,
            to: target_rect(a)?,
        })),
    }
}

/// The front window of the app you're currently using.
pub fn frontmost_window(list: &[WinInfo]) -> Option<WinInfo> {
    list.iter().find(|w| w.focused).cloned()
}

/// Bring a window's app to the front.
#[cfg(target_os = "macos")]
pub fn bring_to_front(w: &WinInfo) {
    let _ = Command::new("open").args(["-a", &w.app]).output();
}

#[cfg(target_os = "windows")]
pub fn bring_to_front(w: &WinInfo) {
    let target = if w.title.is_empty() { &w.app } else { &w.title };
    let escaped = target.replace('\'', "''");
    let _ = hidden_powershell()
        .arg("-Command")
        .arg(format!(
            "(New-Object -ComObject WScript.Shell).AppActivate('{escaped}') | Out-Null"
        ))
        .output();
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn bring_to_front(_w: &WinInfo) {}

// ---------------------------------------------------------------------------
// Template matching
// ---------------------------------------------------------------------------

struct Integral {
    stride: usize,
    sum: Vec<f64>,
    sq: Vec<f64>,
}

impl Integral {
    fn new(d: &[f32], w: usize, h: usize) -> Self {
        let stride = w + 1;
        let mut sum = vec![0.0f64; stride * (h + 1)];
        let mut sq = vec![0.0f64; stride * (h + 1)];
        for y in 0..h {
            let mut row_s = 0.0f64;
            let mut row_q = 0.0f64;
            for x in 0..w {
                let v = d[y * w + x] as f64;
                row_s += v;
                row_q += v * v;
                let i = (y + 1) * stride + (x + 1);
                sum[i] = sum[i - stride] + row_s;
                sq[i] = sq[i - stride] + row_q;
            }
        }
        Integral { stride, sum, sq }
    }

    fn rect(&self, x: usize, y: usize, w: usize, h: usize) -> (f64, f64) {
        let s = self.stride;
        let (a, b, c, d) = (y * s + x, y * s + x + w, (y + h) * s + x, (y + h) * s + x + w);
        (
            self.sum[d] - self.sum[b] - self.sum[c] + self.sum[a],
            self.sq[d] - self.sq[b] - self.sq[c] + self.sq[a],
        )
    }
}

struct Tpl {
    d: Vec<f32>,
    w: usize,
    h: usize,
    norm: f64,
}

fn prep_template(d: &[f32], w: usize, h: usize) -> Option<Tpl> {
    let n = (w * h) as f64;
    let mean = d.iter().map(|v| *v as f64).sum::<f64>() / n;
    let centred: Vec<f32> = d.iter().map(|v| (*v as f64 - mean) as f32).collect();
    let norm = centred.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>().sqrt();
    if norm < 1.0 {
        // A flat, single-colour template can't be located reliably.
        return None;
    }
    Some(Tpl {
        d: centred,
        w,
        h,
        norm,
    })
}

fn to_f32(g: &GrayImage) -> (Vec<f32>, usize, usize) {
    (
        g.as_raw().iter().map(|v| *v as f32).collect(),
        g.width() as usize,
        g.height() as usize,
    )
}

fn downscale(d: &[f32], w: usize, h: usize, f: usize) -> (Vec<f32>, usize, usize) {
    let (nw, nh) = (w / f, h / f);
    let mut out = vec![0.0f32; nw * nh];
    let area = (f * f) as f32;
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = 0.0f32;
            for dy in 0..f {
                let row = (y * f + dy) * w + x * f;
                for dx in 0..f {
                    acc += d[row + dx];
                }
            }
            out[y * nw + x] = acc / area;
        }
    }
    (out, nw, nh)
}

fn ncc(img: &[f32], iw: usize, ii: &Integral, t: &Tpl, x: usize, y: usize) -> f32 {
    let n = (t.w * t.h) as f64;
    let (s, q) = ii.rect(x, y, t.w, t.h);
    let var = q - s * s / n;
    if var <= 1e-3 {
        return 0.0;
    }
    let mut cross = 0.0f64;
    for ty in 0..t.h {
        let start = (y + ty) * iw + x;
        let row = &img[start..start + t.w];
        let trow = &t.d[ty * t.w..(ty + 1) * t.w];
        let mut acc = 0.0f32;
        for i in 0..t.w {
            acc += row[i] * trow[i];
        }
        cross += acc as f64;
    }
    (cross / (var.sqrt() * t.norm)) as f32
}

/// Best top-left position of `tpl` inside `screen`, with its score.
/// Coarse search on a shrunk copy, then refinement at full resolution.
pub fn match_template(screen: &GrayImage, tpl: &GrayImage) -> Option<(u32, u32, f32)> {
    let (sd, sw, sh) = to_f32(screen);
    let (td, tw, th) = to_f32(tpl);
    if tw == 0 || th == 0 || tw > sw || th > sh {
        return None;
    }
    let full_ii = Integral::new(&sd, sw, sh);
    let full_t = prep_template(&td, tw, th)?;
    let f = (tw.min(th) / 8).clamp(1, 4);

    let mut best: Option<(u32, u32, f32)> = None;
    let consider = |x: usize, y: usize, best: &mut Option<(u32, u32, f32)>| {
        let s = ncc(&sd, sw, &full_ii, &full_t, x, y);
        if best.map_or(true, |b| s > b.2) {
            *best = Some((x as u32, y as u32, s));
        }
    };

    if f == 1 {
        for y in 0..=(sh - th) {
            for x in 0..=(sw - tw) {
                consider(x, y, &mut best);
            }
        }
        return best;
    }

    let (cs, csw, csh) = downscale(&sd, sw, sh, f);
    let (ct, ctw, cth) = downscale(&td, tw, th, f);
    if ctw == 0 || cth == 0 || ctw > csw || cth > csh {
        return None;
    }
    let coarse_t = prep_template(&ct, ctw, cth)?;
    let coarse_ii = Integral::new(&cs, csw, csh);
    let mut scores: Vec<(f32, usize, usize)> = Vec::with_capacity((csw - ctw + 1) * (csh - cth + 1));
    for y in 0..=(csh - cth) {
        for x in 0..=(csw - ctw) {
            scores.push((ncc(&cs, csw, &coarse_ii, &coarse_t, x, y), x, y));
        }
    }
    scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Keep the best few, well-separated candidates.
    let mut picked: Vec<(usize, usize)> = Vec::new();
    for (_, x, y) in scores {
        if picked.len() >= 8 {
            break;
        }
        let far = picked.iter().all(|&(px, py)| {
            (px as i64 - x as i64).abs() > (ctw / 2) as i64
                || (py as i64 - y as i64).abs() > (cth / 2) as i64
        });
        if far {
            picked.push((x, y));
        }
    }

    for (cx, cy) in picked {
        let (bx, by) = (cx * f, cy * f);
        let x0 = bx.saturating_sub(f + 1);
        let y0 = by.saturating_sub(f + 1);
        let x1 = (bx + f + 1).min(sw - tw);
        let y1 = (by + f + 1).min(sh - th);
        for y in y0..=y1 {
            for x in x0..=x1 {
                consider(x, y, &mut best);
            }
        }
    }
    best
}
