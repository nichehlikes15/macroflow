//! Checking GitHub for a newer MacroFlow.
//!
//! If the project has published releases, the newest release's version is compared
//! with this build's version. If it hasn't (yet), the commit this build was made
//! from is compared with the latest commit on `main`, so builds made straight from
//! the source still hear about new changes.
//!
//! Only public information is fetched from GitHub; nothing about you or your macros
//! is sent.

use crate::remote;
use serde_json::Value;

/// Where MacroFlow lives on GitHub.
pub const REPO: &str = "nichehlikes15/macroflow";
const BRANCH: &str = "main";

/// This build's version (Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The commit this build was made from (empty if it wasn't built from a git checkout).
pub fn build_commit() -> &'static str {
    option_env!("MACROFLOW_GIT_SHA").unwrap_or("")
}

/// "0.1.0 (812a316)".
pub fn version_label() -> String {
    match build_commit() {
        "" => VERSION.to_string(),
        sha => format!("{VERSION} ({})", &sha[..sha.len().min(7)]),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateKind {
    /// A newer published release.
    Release,
    /// This many newer commits on `main` (no releases published yet).
    Commits(u64),
}

/// A newer MacroFlow.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    pub kind: UpdateKind,
    /// "1.2.0", or the latest commit's short id. Also what "Skip this version" remembers.
    pub version: String,
    /// What changed (release notes, or the newest commit messages).
    pub notes: Vec<String>,
    /// Where to get it.
    pub url: String,
    /// The file for this computer in the release, if there is one, so MacroFlow can
    /// install it itself.
    pub asset: Option<Asset>,
}

/// One downloadable file in a release.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    /// Size in bytes, as GitHub reports it (0 = unknown).
    pub size: u64,
    /// "sha256:…", when GitHub reports it.
    pub digest: Option<String>,
}

/// Is `name` the release file for this kind of computer?
fn asset_fits(name: &str) -> bool {
    let n = name.to_lowercase();
    if cfg!(target_os = "macos") {
        // Only an Apple Silicon build is published so far.
        n.ends_with(".dmg") && cfg!(target_arch = "aarch64") && !n.contains("x64") && !n.contains("intel")
    } else if cfg!(windows) {
        n.ends_with(".exe") && cfg!(target_arch = "x86_64")
    } else {
        false
    }
}

fn pick_asset(v: &Value) -> Option<Asset> {
    v["assets"].as_array()?.iter().find_map(|a| {
        let name = a["name"].as_str()?;
        if !asset_fits(name) {
            return None;
        }
        let url = a["browser_download_url"].as_str()?;
        if !url.starts_with("https://") {
            return None;
        }
        Some(Asset {
            name: name.to_string(),
            url: url.to_string(),
            size: a["size"].as_u64().unwrap_or(0),
            digest: a["digest"].as_str().map(String::from),
        })
    })
}

/// Ask GitHub. Ok(None) = this build is up to date (or there's nothing to compare with).
pub fn check() -> Result<Option<UpdateInfo>, String> {
    let latest = remote::http_get(&format!("https://api.github.com/repos/{REPO}/releases/latest"), GITHUB_JSON)?;
    match latest.status {
        200 => {
            let v: Value = serde_json::from_str(&latest.body).map_err(|e| format!("GitHub's answer didn't make sense ({e})."))?;
            Ok(from_release(&v, VERSION))
        }
        // No releases yet: compare commits instead.
        404 => check_commits(),
        403 | 429 => Err("GitHub asked MacroFlow to slow down; try again in an hour.".into()),
        s => Err(format!("GitHub answered {s}.")),
    }
}

const GITHUB_JSON: &[(&str, &str)] = &[("Accept", "application/vnd.github+json"), ("X-GitHub-Api-Version", "2022-11-28")];

fn check_commits() -> Result<Option<UpdateInfo>, String> {
    let sha = build_commit();
    if sha.is_empty() {
        return Ok(None);
    }
    let r = remote::http_get(&format!("https://api.github.com/repos/{REPO}/compare/{sha}...{BRANCH}"), GITHUB_JSON)?;
    match r.status {
        200 => {
            let v: Value = serde_json::from_str(&r.body).map_err(|e| format!("GitHub's answer didn't make sense ({e})."))?;
            Ok(from_compare(&v))
        }
        // GitHub doesn't know this build's commit (a local change that was never pushed).
        404 => Ok(None),
        403 | 429 => Err("GitHub asked MacroFlow to slow down; try again in an hour.".into()),
        s => Err(format!("GitHub answered {s}.")),
    }
}

/// A release newer than `current`, from GitHub's "latest release" answer.
fn from_release(v: &Value, current: &str) -> Option<UpdateInfo> {
    let tag = v["tag_name"].as_str()?.trim();
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, current) {
        return None;
    }
    let notes = v["body"]
        .as_str()
        .unwrap_or("")
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*']).trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .take(12)
        .collect();
    let url = v["html_url"].as_str().map(String::from).unwrap_or_else(|| format!("https://github.com/{REPO}/releases/latest"));
    let asset = pick_asset(v);
    Some(UpdateInfo { kind: UpdateKind::Release, version, notes, url, asset })
}

/// Newer commits on `main`, from GitHub's "compare" answer.
fn from_compare(v: &Value) -> Option<UpdateInfo> {
    let ahead = v["ahead_by"].as_u64().unwrap_or(0);
    if ahead == 0 {
        return None;
    }
    let commits = v["commits"].as_array()?;
    let newest = commits.last()?;
    let sha = newest["sha"].as_str().unwrap_or("");
    // Newest first, first line of each message.
    let notes = commits
        .iter()
        .rev()
        .filter_map(|c| c["commit"]["message"].as_str())
        .filter_map(|m| m.lines().next())
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .take(8)
        .collect();
    Some(UpdateInfo {
        kind: UpdateKind::Commits(ahead),
        version: sha[..sha.len().min(7)].to_string(),
        notes,
        // The builds CI made for each commit are downloadable from here.
        url: format!("https://github.com/{REPO}/actions/workflows/build.yml?query=branch%3A{BRANCH}"),
        asset: None,
    })
}

// ---------------------------------------------------------------------------
// Installing an update from inside MacroFlow
// ---------------------------------------------------------------------------

/// The installed app this copy runs from: the .app bundle on a Mac, the .exe on
/// Windows. Err explains why this copy can't update itself (e.g. it was started
/// with `cargo run`).
pub fn installed_app() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Couldn't tell where MacroFlow is ({e})."))?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let from_source = exe.components().any(|c| c.as_os_str() == "target");
    if cfg!(target_os = "macos") {
        // …/MacroFlow.app/Contents/MacOS/MacroFlow
        let bundle = exe.ancestors().nth(3).filter(|b| b.extension().is_some_and(|x| x == "app"));
        return match bundle {
            Some(b) => Ok(b.to_path_buf()),
            None if from_source => Err("This copy was built from the source code. Run git pull, then cargo run --release.".into()),
            None => Err("This copy isn't inside MacroFlow.app, so it can't replace itself.".into()),
        };
    }
    if cfg!(windows) {
        if from_source {
            return Err("This copy was built from the source code. Run git pull, then cargo run --release.".into());
        }
        return Ok(exe);
    }
    Err("Updating from inside MacroFlow only works on macOS and Windows.".into())
}

/// Can this copy install `u` itself? (A release file for this computer, and an
/// installed app to replace.)
pub fn can_install(u: &UpdateInfo) -> bool {
    u.asset.is_some() && installed_app().is_ok()
}

/// Download the update, check it, and put it in place of the running app (which
/// carries on running until it quits). `progress` hears how far the download got.
/// Returns what to start to run the new version.
pub fn install(u: &UpdateInfo, progress: &dyn Fn(String)) -> Result<std::path::PathBuf, String> {
    let asset = u.asset.as_ref().ok_or("This release has no file for this computer.")?;
    let target = installed_app()?;
    let dir = std::env::temp_dir().join(format!("macroflow-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make a folder for the download ({e})."))?;
    let file = dir.join(asset.name.replace(['/', '\\'], "_"));
    let result = download(asset, &file, progress).and_then(|_| {
        progress("Installing…".into());
        put_in_place(&file, &dir, &target)
    });
    let _ = std::fs::remove_dir_all(&dir);
    result.map(|_| target)
}

fn download(asset: &Asset, to: &std::path::Path, progress: &dyn Fn(String)) -> Result<(), String> {
    use std::io::{Read, Write};
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15 * 60)))
        .timeout_connect(Some(std::time::Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut resp = agent
        .get(&asset.url)
        .header("User-Agent", concat!("MacroFlow/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/octet-stream")
        .call()
        .map_err(|e| format!("Couldn't download the update: {e}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!("GitHub answered {status} to the download."));
    }
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(asset.size);
    let mut reader = resp.body_mut().with_config().limit(1 << 31).reader();
    let mut out = std::fs::File::create(to).map_err(|e| format!("Couldn't save the download ({e})."))?;
    let mut hasher = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 256 * 1024];
    let (mut got, mut shown) = (0u64, u64::MAX);
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("The download stopped ({e})."))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| format!("Couldn't save the download ({e})."))?;
        hasher.update(&buf[..n]);
        got += n as u64;
        let pct = if total > 0 { got * 100 / total } else { 0 };
        if pct != shown {
            shown = pct;
            progress(if total > 0 {
                format!("Downloading the update… {pct}% of {:.0} MB", total as f64 / 1_048_576.0)
            } else {
                format!("Downloading the update… {:.1} MB", got as f64 / 1_048_576.0)
            });
        }
    }
    out.flush().map_err(|e| format!("Couldn't save the download ({e})."))?;
    if asset.size > 0 && got != asset.size {
        return Err(format!("The download is incomplete ({got} of {} bytes). Try again.", asset.size));
    }
    if let Some(want) = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        let have: String = hasher.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect();
        if !have.eq_ignore_ascii_case(want.trim()) {
            return Err("The download doesn't match the release's checksum, so it wasn't installed. Try again.".into());
        }
    }
    Ok(())
}

/// Swap the new version in: the old one is moved aside (not deleted while it runs)
/// and cleared away the next time MacroFlow starts.
#[cfg(target_os = "macos")]
fn put_in_place(file: &std::path::Path, dir: &std::path::Path, bundle: &std::path::Path) -> Result<(), String> {
    use std::process::Command;
    let mount = dir.join("mnt");
    let attach = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"])
        .arg(&mount)
        .arg(file)
        .output()
        .map_err(|e| format!("Couldn't open the disk image ({e})."))?;
    if !attach.status.success() {
        return Err(format!("Couldn't open the disk image: {}", String::from_utf8_lossy(&attach.stderr).trim()));
    }
    let detach = || {
        let _ = Command::new("hdiutil").args(["detach", "-quiet", "-force"]).arg(&mount).output();
    };
    let found = std::fs::read_dir(&mount)
        .ok()
        .and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "app")));
    let Some(new_app) = found.filter(|a| a.join("Contents/MacOS/MacroFlow").exists()) else {
        detach();
        return Err("The disk image doesn't contain MacroFlow.app.".into());
    };
    let parent = bundle.parent().ok_or("Couldn't tell which folder MacroFlow is in.")?;
    let staged = parent.join(".MacroFlow-update.app");
    let old = parent.join(".MacroFlow-old.app");
    let _ = std::fs::remove_dir_all(&staged);
    let copy = Command::new("ditto").arg(&new_app).arg(&staged).output();
    detach();
    match copy {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(format!(
                "Couldn't copy the new version into {} ({}). Is that folder writable?",
                parent.display(),
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        Err(e) => return Err(format!("Couldn't copy the new version ({e}).")),
    }
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(bundle, &old).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staged);
        format!("Couldn't move the old version aside ({e}).")
    })?;
    if let Err(e) = std::fs::rename(&staged, bundle) {
        // Put the old one back, so MacroFlow still opens.
        let _ = std::fs::rename(&old, bundle);
        let _ = std::fs::remove_dir_all(&staged);
        return Err(format!("Couldn't put the new version in place ({e})."));
    }
    Ok(())
}

#[cfg(windows)]
fn put_in_place(file: &std::path::Path, _dir: &std::path::Path, exe: &std::path::Path) -> Result<(), String> {
    let folder = exe.parent().ok_or("Couldn't tell which folder MacroFlow is in.")?;
    let staged = folder.join("macroflow.new.exe");
    std::fs::copy(file, &staged).map_err(|e| {
        format!("Couldn't write to {} ({e}). Move macroflow.exe to a folder you can write to, or run it as administrator.", folder.display())
    })?;
    // A running .exe can't be overwritten, but it can be renamed.
    let mut old = folder.join("macroflow.old.exe");
    if old.exists() && std::fs::remove_file(&old).is_err() {
        old = folder.join(format!("macroflow.old-{}.exe", std::process::id()));
    }
    std::fs::rename(exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("Couldn't move the old version aside ({e}).")
    })?;
    if let Err(e) = std::fs::rename(&staged, exe) {
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&staged);
        return Err(format!("Couldn't put the new version in place ({e})."));
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn put_in_place(_file: &std::path::Path, _dir: &std::path::Path, _target: &std::path::Path) -> Result<(), String> {
    Err("Updating from inside MacroFlow only works on macOS and Windows.".into())
}

/// Start the newly installed version (after this one has had a moment to quit).
pub fn relaunch(app: &std::path::Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 1; /usr/bin/open -n \"$0\"")
            .arg(app)
            .spawn();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new(app).spawn();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = app;
}

/// Remove the old version an update left behind.
pub fn clean_up_after_update() {
    let Ok(app) = installed_app() else { return };
    #[cfg(target_os = "macos")]
    if let Some(parent) = app.parent() {
        let _ = std::fs::remove_dir_all(parent.join(".MacroFlow-old.app"));
        let _ = std::fs::remove_dir_all(parent.join(".MacroFlow-update.app"));
    }
    #[cfg(windows)]
    if let Some(folder) = app.parent() {
        if let Ok(entries) = std::fs::read_dir(folder) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_lowercase();
                if (name.starts_with("macroflow.old") && name.ends_with(".exe")) || name == "macroflow.new.exe" {
                    // Fails while the old one is still closing; it goes next time.
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = app;
}

/// "1.10.0" is newer than "1.9.2". Missing parts count as 0; anything after a "-"
/// (e.g. "-beta") is ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> {
        s.split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Needs the internet: `cargo test -- --ignored github_answers`.
    #[test]
    #[ignore]
    fn github_answers() {
        // Either "up to date" or an update; what matters is that GitHub was reached
        // and its answer understood.
        let r = check();
        assert!(r.is_ok(), "{r:?}");
        println!("build {} -> {:?}", version_label(), r);
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("1.10.0", "1.9.2"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn a_newer_release_is_offered() {
        let v = json!({ "tag_name": "v0.3.0", "html_url": "https://example/r", "body": "## Changes\n- Faster OCR\n* Fixed a crash\n" });
        let u = from_release(&v, "0.1.0").unwrap();
        assert_eq!(u.version, "0.3.0");
        assert_eq!(u.notes, vec!["Faster OCR".to_string(), "Fixed a crash".to_string()]);
        assert!(from_release(&v, "0.3.0").is_none());
    }

    #[test]
    fn the_file_for_this_computer_is_picked() {
        let v = json!({ "tag_name": "v9.0.0", "html_url": "https://example/r", "body": "", "assets": [
            { "name": "README.md", "browser_download_url": "https://example/README.md", "size": 3 },
            { "name": "MacroFlow-macos-arm64.dmg", "browser_download_url": "https://example/m.dmg", "size": 10, "digest": "sha256:ab" },
            { "name": "MacroFlow-windows-x64.exe", "browser_download_url": "https://example/w.exe", "size": 20 },
        ]});
        let u = from_release(&v, "0.1.0").unwrap();
        let name = u.asset.as_ref().map(|a| a.name.as_str());
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(name, Some("MacroFlow-macos-arm64.dmg"));
            assert_eq!(u.asset.as_ref().unwrap().digest.as_deref(), Some("sha256:ab"));
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            assert_eq!(name, Some("MacroFlow-windows-x64.exe"));
        }
        // Built with cargo, so it can't replace itself.
        assert!(installed_app().is_err());
        assert!(!can_install(&u));
    }

    /// Swaps a real disk image's app in place of another (uses hdiutil):
    /// `cargo test -- --ignored a_dmg_replaces_the_app`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn a_dmg_replaces_the_app() {
        use std::process::Command;
        let root = std::env::temp_dir().join(format!("mf-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let make_app = |at: &std::path::Path, text: &str| {
            std::fs::create_dir_all(at.join("Contents/MacOS")).unwrap();
            std::fs::write(at.join("Contents/MacOS/MacroFlow"), text).unwrap();
        };
        let installed = root.join("Applications/MacroFlow.app");
        make_app(&installed, "old");
        let src = root.join("src");
        make_app(&src.join("MacroFlow.app"), "new");
        let dmg = root.join("MacroFlow.dmg");
        let ok = Command::new("hdiutil")
            .args(["create", "-quiet", "-volname", "MacroFlow", "-format", "UDZO", "-srcfolder"])
            .arg(&src)
            .arg(&dmg)
            .status()
            .unwrap();
        assert!(ok.success());
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        put_in_place(&dmg, &work, &installed).unwrap();
        assert_eq!(std::fs::read_to_string(installed.join("Contents/MacOS/MacroFlow")).unwrap(), "new");
        assert!(root.join("Applications/.MacroFlow-old.app").exists());
        assert!(!root.join("Applications/.MacroFlow-update.app").exists());
        assert!(!work.join("mnt").join("MacroFlow.app").exists(), "the disk image is still mounted");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn newer_commits_are_offered() {
        let v = json!({
            "ahead_by": 2,
            "commits": [
                { "sha": "aaaaaaa111", "commit": { "message": "Older change\n\nDetails" } },
                { "sha": "bbbbbbb222", "commit": { "message": "Newest change" } },
            ]
        });
        let u = from_compare(&v).unwrap();
        assert_eq!(u.kind, UpdateKind::Commits(2));
        assert_eq!(u.version, "bbbbbbb");
        assert_eq!(u.notes, vec!["Newest change".to_string(), "Older change".to_string()]);
        assert!(from_compare(&json!({ "ahead_by": 0, "commits": [] })).is_none());
    }
}
