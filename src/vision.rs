//! Screen capture and image search (normalised cross-correlation template matching).
//!
//! Everything here works in the same coordinates the mouse uses: "points" on macOS
//! (Retina screenshots are scaled down to match) and physical pixels on Windows
//! (MacroFlow is DPI-aware there, so the scale is 1).

use image::{imageops, imageops::FilterType, GrayImage, RgbaImage};
use std::path::{Path, PathBuf};
use std::process::Command;
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
}

struct Screen {
    ox: i32,
    oy: i32,
    gray: GrayImage,
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
        out.push(Screen { ox, oy, gray });
    }
    if out.is_empty() {
        return Err(SCREEN_PERMISSION.into());
    }
    Ok(out)
}

/// Search every display for the template. Also tries the template at half size,
/// so screenshots taken on a Retina display (2×) still match.
pub fn locate(template: &GrayImage, threshold: f32) -> Result<Hit, String> {
    let screens = capture_all()?;
    let mut best = Hit {
        found: false,
        x: 0,
        y: 0,
        score: -1.0,
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
    for s in &screens {
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
    /// Which display it is: the system display id (macOS) and its position
    /// in the display list (Windows), so the selection window opens on the same one.
    pub monitor_id: u32,
    pub monitor_index: usize,
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
    let monitor_id = m.id().unwrap_or(0);
    let monitor_index = Monitor::all()
        .ok()
        .and_then(|all| all.iter().position(|o| o.id().ok() == Some(monitor_id)))
        .unwrap_or(0);
    Ok(Shot {
        path,
        monitor_id,
        monitor_index,
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

#[cfg(target_os = "macos")]
fn ocr_helper() -> Result<PathBuf, String> {
    let built = option_env!("MACROFLOW_OCR_HELPER").unwrap_or("");
    if !built.is_empty() && Path::new(built).exists() {
        return Ok(PathBuf::from(built));
    }
    // Fall back to a copy next to the executable (for a packaged app).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("macroflow-ocr");
            if p.exists() {
                return Ok(p);
            }
        }
    }
    Err("The text reader isn't built. Run `xcode-select --install`, then `cargo build --release` again.".into())
}

/// macOS: Apple Vision, through the small Swift helper built by build.rs.
#[cfg(target_os = "macos")]
fn run_ocr(path: &Path) -> Result<String, String> {
    let out = Command::new(ocr_helper()?)
        .arg(path)
        .output()
        .map_err(|e| format!("Couldn't run the text reader: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "The text reader failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
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
fn run_ocr(path: &Path) -> Result<String, String> {
    let script = std::env::temp_dir().join("macroflow-ocr.ps1");
    // Rewrite every time so an updated script is always used.
    std::fs::write(&script, WINDOWS_OCR_SCRIPT).map_err(|e| format!("Couldn't write the OCR script: {e}"))?;
    let out = hidden_powershell()
        .arg("-File")
        .arg(&script)
        .arg(path)
        .output()
        .map_err(|e| format!("Couldn't run PowerShell for text reading: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "The text reader failed: {}. Windows needs an OCR language pack (Settings → Time & language → Language) for your display language.",
            err.trim().lines().next().unwrap_or("unknown error")
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn run_ocr(_path: &Path) -> Result<String, String> {
    Err("Reading text is only supported on macOS and Windows.".into())
}

/// Read the text inside an image.
pub fn ocr_image(img: &RgbaImage) -> Result<String, String> {
    // OCR reads small text much better when it's bigger.
    // (Windows OCR rejects images larger than 4096 px, so stay under that.)
    let max_f = (4000 / img.width().max(img.height()).max(1)).max(1);
    let img = if img.height() < 80 && max_f >= 2 {
        let f = (80 / img.height().max(1)).clamp(2, 6).min(max_f);
        imageops::resize(img, img.width() * f, img.height() * f, FilterType::CatmullRom)
    } else {
        img.clone()
    };
    let path = unique_temp("ocr");
    img.save(&path).map_err(|e| format!("Couldn't save the image to read: {e}"))?;
    let result = run_ocr(&path);
    let _ = std::fs::remove_file(&path);
    result
}

/// Read the text in an area of the screen (global points).
pub fn read_text(x: i32, y: i32, w: i32, h: i32) -> Result<String, String> {
    ocr_image(&capture_area(x, y, w, h)?)
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
        let big = w.width().unwrap_or(0) >= 60 && w.height().unwrap_or(0) >= 40;
        if !big {
            continue;
        }
        out.push(WinInfo {
            title: w.title().unwrap_or_default(),
            focused: w.is_focused().unwrap_or(false),
            app,
        });
    }
    Ok(out)
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
