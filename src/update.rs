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
    Some(UpdateInfo { kind: UpdateKind::Release, version, notes, url })
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
    })
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
