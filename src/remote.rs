//! Talking to the outside world: Discord / custom webhooks, and a Discord bot that
//! can start, stop and check on macros with slash commands or buttons.
//!
//! Everything here is outgoing. MacroFlow never listens on a network port: the bot
//! keeps one connection open *to* Discord (its "gateway"), and Discord delivers
//! commands down it. Discord tells us who pressed a button or ran a command, and
//! only the Discord users listed in the Remote tab are obeyed.

use crate::engine::{Tx, WorkerMsg};
use crate::model::TriggerInput;
use crate::vision;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const API: &str = "https://discord.com/api/v10";
const GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
/// Discord asks bots to say who they are like this.
const USER_AGENT: &str = "DiscordBot (https://github.com/macroflow, 0.1)";
/// Discord messages can't be longer than this.
const MAX_MESSAGE: usize = 2000;
/// An interaction's token can send follow-ups for 15 minutes; leave some slack.
const FOLLOWUP_WINDOW: Duration = Duration::from_secs(14 * 60);
/// Message flag: only the person who ran the command sees the reply.
const EPHEMERAL: u64 = 64;

// ---------------------------------------------------------------------------
// Settings (kept outside the macro files, since they hold secrets)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSettings {
    /// Used by Webhook steps with no URL of their own, and by the run notifications.
    pub webhook_url: String,
    /// Post to the default webhook when a run finishes normally.
    pub notify_finish: bool,
    /// Post to the default webhook when a run stops unexpectedly or fails.
    pub notify_stop: bool,
    /// Keep the Discord bot connected while MacroFlow is open.
    pub bot_enabled: bool,
    pub bot_token: String,
    /// Discord user IDs allowed to control MacroFlow (separated by commas or spaces).
    pub allowed_users: String,
    /// Look for a newer MacroFlow when it opens (at most once a day).
    #[serde(default = "yes")]
    pub check_updates: bool,
    /// When that last happened (ms since 1970).
    pub last_update_check_ms: u64,
    /// A version you chose to skip: it isn't offered again.
    pub skipped_update: String,
}

fn yes() -> bool {
    true
}

impl Default for RemoteSettings {
    fn default() -> Self {
        RemoteSettings {
            webhook_url: String::new(),
            notify_finish: false,
            notify_stop: false,
            bot_enabled: false,
            bot_token: String::new(),
            allowed_users: String::new(),
            check_updates: true,
            last_update_check_ms: 0,
            skipped_update: String::new(),
        }
    }
}

impl RemoteSettings {
    /// Where the settings live: the per-user app config folder, not Documents, so
    /// they aren't synced or shared along with macros.
    pub fn path() -> PathBuf {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let base = if cfg!(target_os = "windows") {
            env("APPDATA").unwrap_or_else(|| PathBuf::from("."))
        } else if cfg!(target_os = "macos") {
            env("HOME").unwrap_or_else(|| PathBuf::from(".")).join("Library").join("Application Support")
        } else {
            env("XDG_CONFIG_HOME")
                .or_else(|| env("HOME").map(|h| h.join(".config")))
                .unwrap_or_else(|| PathBuf::from("."))
        };
        base.join("MacroFlow").join("remote.json")
    }

    pub fn load() -> RemoteSettings {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        let dir = path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't save the Remote settings: {e}"))?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = dir.join("remote.json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("Couldn't save the Remote settings: {e}"))?;
        // Only you can read it: it holds the bot token and webhook URL.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &path).map_err(|e| format!("Couldn't save the Remote settings: {e}"))
    }

    /// The allowed Discord user IDs (snowflakes are all digits).
    pub fn allowed_ids(&self) -> Vec<String> {
        parse_user_ids(&self.allowed_users)
    }
}

pub fn parse_user_ids(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .map(str::trim)
        .filter(|t| t.len() >= 5 && t.chars().all(|c| c.is_ascii_digit()))
        .map(String::from)
        .collect()
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// What a server answered.
#[derive(Clone, Debug)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
}

impl HttpReply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        // Look at error statuses ourselves (Discord explains what went wrong in the body).
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build()
        .new_agent()
}

fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "The server took too long to answer.".into(),
        ureq::Error::HostNotFound => "Couldn't find that server (check the URL and your internet connection).".into(),
        ureq::Error::Io(e) => format!("Network problem: {e}"),
        e => format!("Couldn't send it: {e}"),
    }
}

#[derive(Clone, Copy)]
enum Method {
    Post,
    Put,
    Patch,
}

/// Send one request. Retries once if Discord says "slow down" (status 429).
fn request(method: Method, url: &str, auth: Option<&str>, content_type: &str, body: &[u8]) -> Result<HttpReply, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("“{}” isn't a web address (it should start with https://).", short(url, 40)));
    }
    let agent = agent();
    for attempt in 0..2 {
        let mut r = match method {
            Method::Post => agent.post(url),
            Method::Put => agent.put(url),
            Method::Patch => agent.patch(url),
        };
        if let Some(a) = auth {
            r = r.header("Authorization", a);
        }
        let result = r.header("Content-Type", content_type).send(body);
        let mut resp = result.map_err(http_err)?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        if status == 429 && attempt == 0 {
            let wait = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["retry_after"].as_f64())
                .unwrap_or(1.0)
                .clamp(0.1, 10.0);
            thread::sleep(Duration::from_secs_f64(wait));
            continue;
        }
        return Ok(HttpReply { status, body: text });
    }
    Err("Discord is rate limiting MacroFlow; try again in a moment.".into())
}

/// A plain GET (used by the update check), with extra headers.
pub fn http_get(url: &str, headers: &[(&str, &str)]) -> Result<HttpReply, String> {
    if !url.starts_with("https://") {
        return Err("Only https addresses are fetched.".into());
    }
    let mut r = agent()
        .get(url)
        .header("User-Agent", concat!("MacroFlow/", env!("CARGO_PKG_VERSION")));
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let mut resp = r.call().map_err(http_err)?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    Ok(HttpReply { status, body })
}

fn post_json(url: &str, auth: Option<&str>, body: &Value) -> Result<HttpReply, String> {
    request(Method::Post, url, auth, "application/json", body.to_string().as_bytes())
}

/// Explain a failed reply in a sentence ("401: Invalid Webhook Token").
fn explain(r: &HttpReply) -> String {
    let why = serde_json::from_str::<Value>(&r.body)
        .ok()
        .and_then(|v| v["message"].as_str().map(String::from))
        .unwrap_or_else(|| short(r.body.trim(), 160));
    let hint = match r.status {
        401 | 403 => " Check the URL / token is right and hasn't been reset.",
        404 => " The webhook or channel may have been deleted.",
        _ => "",
    };
    if why.is_empty() {
        format!("The server answered {}.{hint}", r.status)
    } else {
        format!("The server answered {}: {why}.{hint}", r.status)
    }
}

/// A form with a JSON part and (optionally) one PNG file: how Discord takes attachments.
fn multipart(payload: &Value, png: Option<&[u8]>) -> (String, Vec<u8>) {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let boundary = format!("macroflow-{stamp:x}");
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{payload}\r\n"
        )
        .as_bytes(),
    );
    if let Some(png) = png {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"screen.png\"\r\nContent-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(png);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// The message for Discord, cut to its 2000-character limit.
fn discord_text(text: &str) -> String {
    if text.chars().count() <= MAX_MESSAGE {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(MAX_MESSAGE - 1).collect::<String>())
    }
}

/// Post a message (and maybe a screenshot) to a Discord webhook.
pub fn send_discord(url: &str, text: &str, png: Option<Vec<u8>>) -> Result<(), String> {
    let text = if text.trim().is_empty() && png.is_none() { "(empty message)" } else { text };
    let mut payload = json!({
        "content": discord_text(text),
        "username": "MacroFlow",
        // Never ping @everyone / roles from a macro by accident.
        "allowed_mentions": { "parse": [] },
    });
    let reply = match png {
        Some(png) => {
            payload["attachments"] = json!([{ "id": 0, "filename": "screen.png" }]);
            let (ct, body) = multipart(&payload, Some(&png));
            request(Method::Post, url, None, &ct, &body)?
        }
        None => post_json(url, None, &payload)?,
    };
    if reply.ok() {
        Ok(())
    } else {
        Err(explain(&reply))
    }
}

/// POST `body` to any URL (as JSON if it is JSON, otherwise as plain text).
pub fn send_custom(url: &str, body: &str) -> Result<HttpReply, String> {
    let ct = if serde_json::from_str::<Value>(body).is_ok() { "application/json" } else { "text/plain; charset=utf-8" };
    let reply = request(Method::Post, url, None, ct, body.as_bytes())?;
    if reply.ok() {
        Ok(reply)
    } else {
        Err(explain(&reply))
    }
}

/// A PNG of the main screen, shrunk so it uploads quickly and fits Discord's limits.
pub fn screenshot_png() -> Result<Vec<u8>, String> {
    let screen = vision::primary_screen()?;
    let shot = vision::screenshot_display_at(screen.x + 1, screen.y + 1)?;
    let img = image::open(&shot.path).map_err(|e| format!("Couldn't open the screenshot: {e}"));
    let _ = std::fs::remove_file(&shot.path);
    let img = img?.to_rgba8();
    let (w, h) = img.dimensions();
    let max = 1920u32;
    let img = if w > max || h > max {
        let f = max as f32 / w.max(h) as f32;
        image::imageops::resize(
            &img,
            ((w as f32 * f).round() as u32).max(1),
            ((h as f32 * f).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("Couldn't save the screenshot: {e}"))?;
    Ok(out.into_inner())
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

// ---------------------------------------------------------------------------
// Replying to Discord commands
// ---------------------------------------------------------------------------

/// What a Discord user asked MacroFlow to do.
#[derive(Clone, Debug, PartialEq)]
pub enum RemoteCmd {
    /// Start the macro (None) or one of its triggers.
    Run(Option<String>, Vec<(String, String)>),
    Stop,
    Pause,
    Status,
}

/// How to answer the Discord command that asked for something. Answers can be sent
/// for 15 minutes; after that, use the bot to post in the channel instead.
#[derive(Clone, Debug)]
pub struct Reply {
    app_id: String,
    token: String,
    pub channel_id: String,
    /// The Discord user who asked (shown as a mention when the run ends).
    pub user_id: String,
    at: Instant,
}

impl Reply {
    pub fn is_fresh(&self) -> bool {
        self.at.elapsed() < FOLLOWUP_WINDOW
    }

    /// Replace the "MacroFlow is thinking…" placeholder with the answer.
    pub fn answer(&self, text: &str) {
        let url = format!("{API}/webhooks/{}/{}/messages/@original", self.app_id, self.token);
        let body = json!({ "content": discord_text(text), "allowed_mentions": { "parse": [] } });
        thread::spawn(move || {
            let _ = request(Method::Patch, &url, None, "application/json", body.to_string().as_bytes());
        });
    }

    /// Answer with a picture.
    fn answer_png(&self, text: &str, png: Vec<u8>) -> Result<(), String> {
        let url = format!("{API}/webhooks/{}/{}/messages/@original", self.app_id, self.token);
        let payload = json!({
            "content": discord_text(text),
            "attachments": [{ "id": 0, "filename": "screen.png" }],
        });
        let (ct, body) = multipart(&payload, Some(&png));
        let r = request(Method::Patch, &url, None, &ct, &body)?;
        if r.ok() {
            Ok(())
        } else {
            Err(explain(&r))
        }
    }

    /// A new message under the original answer (e.g. "the run finished"). It pings
    /// the person who started the run, so they hear about it.
    pub fn follow_up(&self, text: &str) {
        let url = format!("{API}/webhooks/{}/{}", self.app_id, self.token);
        let body = json!({
            "content": discord_text(&format!("<@{}> {text}", self.user_id)),
            "allowed_mentions": { "users": [self.user_id] },
        });
        thread::spawn(move || {
            let _ = post_json(&url, None, &body);
        });
    }
}

// ---------------------------------------------------------------------------
// The Discord bot
// ---------------------------------------------------------------------------

/// How the bot's connection is doing (shown in the Remote tab).
#[derive(Clone, Debug, PartialEq)]
pub enum BotState {
    Connecting,
    Online(String),
    /// Lost the connection; trying again.
    Retrying(String),
    /// Gave up (e.g. a wrong token). Needs a change in the Remote tab.
    Failed(String),
    Off,
}

/// The trigger command names, shared with the bot's thread.
struct Triggers {
    cmds: Vec<TriggerCmd>,
    /// When they last changed and haven't been sent to Discord yet.
    changed: Option<Instant>,
}

/// A Trigger as a Discord command: its name and the values it asks for.
pub type TriggerCmd = (String, Vec<TriggerInput>);

/// Wait this long after the last change before updating Discord's command list
/// (renaming a trigger letter by letter shouldn't update it every keystroke).
const REGISTER_AFTER: Duration = Duration::from_secs(3);

/// The running bot. Dropping it disconnects.
pub struct Bot {
    stop: Arc<AtomicBool>,
    triggers: Arc<Mutex<Triggers>>,
    token: String,
}

/// Trigger names as Discord commands: valid, without duplicates or the bot's own names.
fn command_names(triggers: &[TriggerCmd]) -> Vec<TriggerCmd> {
    let mut out: Vec<TriggerCmd> = Vec::new();
    for (t, inputs) in triggers {
        let c = crate::model::command_name(t);
        if !c.is_empty() && !crate::model::BOT_COMMANDS.contains(&c.as_str()) && !out.iter().any(|(n, _)| *n == c) {
            out.push((c, inputs.clone()));
        }
    }
    out
}

impl Bot {
    /// Connect to Discord in the background. Each trigger becomes a slash command.
    pub fn start(token: &str, allowed: Vec<String>, triggers: Vec<TriggerCmd>, tx: Tx) -> Bot {
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(Triggers { cmds: command_names(&triggers), changed: None }));
        let token = token.trim().trim_start_matches("Bot ").to_string();
        let ctx = BotCtx {
            token: token.clone(),
            allowed,
            tx,
            stop: stop.clone(),
            triggers: shared.clone(),
            app_id: String::new(),
            name: String::new(),
        };
        let _ = thread::Builder::new().name("macroflow-discord".into()).spawn(move || bot_main(ctx));
        Bot { stop, triggers: shared, token }
    }

    /// Tell the bot the macro's triggers changed: their slash commands are updated
    /// a few seconds later.
    pub fn set_triggers(&self, cmds: Vec<TriggerCmd>) {
        let cmds = command_names(&cmds);
        if let Ok(mut t) = self.triggers.lock() {
            if t.cmds != cmds {
                t.cmds = cmds;
                t.changed = Some(Instant::now());
            }
        }
    }

    /// Post in a channel as the bot (when a command's reply window has passed).
    pub fn post_channel(&self, channel_id: &str, user_id: &str, text: &str) {
        let url = format!("{API}/channels/{channel_id}/messages");
        let auth = format!("Bot {}", self.token);
        let body = json!({
            "content": discord_text(&format!("<@{user_id}> {text}")),
            "allowed_mentions": { "users": [user_id] },
        });
        thread::spawn(move || {
            let _ = post_json(&url, Some(&auth), &body);
        });
    }
}

impl Drop for Bot {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct BotCtx {
    token: String,
    allowed: Vec<String>,
    tx: Tx,
    stop: Arc<AtomicBool>,
    triggers: Arc<Mutex<Triggers>>,
    app_id: String,
    /// The bot's name, once connected.
    name: String,
}

impl BotCtx {
    fn state(&self, s: BotState) {
        let _ = self.tx.send(WorkerMsg::Bot(s));
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Sleep, waking early if the bot is turned off. Returns false if it was.
    fn nap(&self, d: Duration) -> bool {
        let end = Instant::now() + d;
        while Instant::now() < end {
            if self.stopped() {
                return false;
            }
            thread::sleep(Duration::from_millis(100));
        }
        !self.stopped()
    }

    /// The trigger command names (for the panel).
    fn triggers(&self) -> Vec<String> {
        self.trigger_cmds().into_iter().map(|(n, _)| n).collect()
    }

    fn trigger_cmds(&self) -> Vec<TriggerCmd> {
        self.triggers.lock().map(|t| t.cmds.clone()).unwrap_or_default()
    }

    /// Triggers changed a while ago and Discord hasn't heard yet?
    fn triggers_settled(&self) -> bool {
        self.triggers
            .lock()
            .map(|t| t.changed.map_or(false, |at| at.elapsed() >= REGISTER_AFTER))
            .unwrap_or(false)
    }

    /// Send the command list to Discord and report how it went.
    fn sync_commands(&self) {
        if let Ok(mut t) = self.triggers.lock() {
            t.changed = None;
        }
        match register_commands(self) {
            Ok(()) => self.state(BotState::Online(self.name.clone())),
            Err(e) => self.state(BotState::Online(format!("{} (couldn't update the slash commands: {e})", self.name))),
        }
    }
}

enum SessionEnd {
    /// The bot was turned off.
    Stopped,
    /// Connect again (Discord asked, or the connection dropped).
    Again(String),
    /// Don't try again: it can't work as set up.
    Fatal(String),
}

fn bot_main(mut ctx: BotCtx) {
    if ctx.token.is_empty() {
        ctx.state(BotState::Failed("Paste the bot token first.".into()));
        return;
    }
    let mut backoff = Duration::from_secs(2);
    loop {
        if ctx.stopped() {
            break;
        }
        ctx.state(BotState::Connecting);
        let began = Instant::now();
        match session(&mut ctx) {
            SessionEnd::Stopped => break,
            SessionEnd::Fatal(why) => {
                ctx.state(BotState::Failed(why));
                return;
            }
            SessionEnd::Again(why) => {
                // A connection that lasted a while was fine: start the backoff over.
                if began.elapsed() > Duration::from_secs(60) {
                    backoff = Duration::from_secs(2);
                }
                ctx.state(BotState::Retrying(why));
                if !ctx.nap(backoff) {
                    break;
                }
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        }
    }
    ctx.state(BotState::Off);
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

fn set_read_timeout(ws: &mut Socket, d: Duration) {
    #[allow(unreachable_patterns)]
    let _ = match ws.get_mut() {
        MaybeTlsStream::Plain(s) => s.set_read_timeout(Some(d)),
        MaybeTlsStream::Rustls(s) => s.sock.set_read_timeout(Some(d)),
        _ => Ok(()),
    };
}

fn send_ws(ws: &mut Socket, v: Value) -> Result<(), String> {
    ws.send(Message::text(v.to_string())).map_err(|e| format!("Lost the connection to Discord ({e})."))
}

/// One connection to Discord's gateway, until it ends.
fn session(ctx: &mut BotCtx) -> SessionEnd {
    let (mut ws, _) = match tungstenite::connect(GATEWAY) {
        Ok(c) => c,
        Err(e) => return SessionEnd::Again(format!("Couldn't reach Discord ({e}).")),
    };
    // Short reads, so the loop can send heartbeats and notice being turned off.
    set_read_timeout(&mut ws, Duration::from_millis(250));
    let mut seq: Option<u64> = None;
    let mut interval: Option<Duration> = None;
    let mut next_beat = Instant::now() + Duration::from_secs(3600);
    let mut acked = true;
    let hello_by = Instant::now() + Duration::from_secs(15);
    loop {
        if ctx.stopped() {
            let _ = ws.close(None);
            let _ = ws.flush();
            return SessionEnd::Stopped;
        }
        if interval.is_none() && Instant::now() > hello_by {
            return SessionEnd::Again("Discord didn't say hello.".into());
        }
        if !ctx.app_id.is_empty() && ctx.triggers_settled() {
            ctx.sync_commands();
        }
        if let Some(every) = interval {
            if Instant::now() >= next_beat {
                if !acked {
                    return SessionEnd::Again("Discord stopped answering.".into());
                }
                if let Err(e) = send_ws(&mut ws, json!({ "op": 1, "d": seq })) {
                    return SessionEnd::Again(e);
                }
                acked = false;
                next_beat = Instant::now() + every;
            }
        }
        let msg = match ws.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) => return SessionEnd::Again(format!("Lost the connection to Discord ({e}).")),
        };
        let text = match msg {
            Message::Text(t) => t.as_str().to_string(),
            Message::Close(frame) => {
                let code: u16 = frame.as_ref().map(|f| f.code.into()).unwrap_or(1000);
                let reason = frame.map(|f| f.reason.as_str().to_string()).unwrap_or_default();
                return match code {
                    4004 => SessionEnd::Fatal("Discord rejected the bot token. Copy it again from the Developer Portal (Bot → Reset Token).".into()),
                    4010..=4014 => SessionEnd::Fatal(format!("Discord refused the connection ({code}: {reason}).")),
                    _ => SessionEnd::Again(format!("Discord closed the connection ({code}).")),
                };
            }
            _ => continue,
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        if let Some(s) = v["s"].as_u64() {
            seq = Some(s);
        }
        match v["op"].as_u64() {
            // Hello: start heartbeating and say who we are.
            Some(10) => {
                let ms = v["d"]["heartbeat_interval"].as_u64().unwrap_or(41_250).max(1000);
                let every = Duration::from_millis(ms);
                interval = Some(every);
                // Discord asks for a random first beat, so many clients don't beat at once.
                let jitter = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_millis()).unwrap_or(500) as u64;
                next_beat = Instant::now() + Duration::from_millis(ms * jitter / 1000);
                let identify = json!({
                    "op": 2,
                    "d": {
                        "token": ctx.token,
                        // Slash commands and buttons need no extra intents.
                        "intents": 0,
                        "properties": { "os": std::env::consts::OS, "browser": "macroflow", "device": "macroflow" },
                    }
                });
                if let Err(e) = send_ws(&mut ws, identify) {
                    return SessionEnd::Again(e);
                }
            }
            // Heartbeat ack.
            Some(11) => acked = true,
            // Discord wants a heartbeat now.
            Some(1) => {
                if let Err(e) = send_ws(&mut ws, json!({ "op": 1, "d": seq })) {
                    return SessionEnd::Again(e);
                }
            }
            // Reconnect / invalid session: start a fresh connection.
            Some(7) => return SessionEnd::Again("Discord asked to reconnect.".into()),
            Some(9) => {
                ctx.nap(Duration::from_secs(3));
                return SessionEnd::Again("Discord ended the session.".into());
            }
            Some(0) => match v["t"].as_str() {
                Some("READY") => {
                    ctx.name = v["d"]["user"]["username"].as_str().unwrap_or("bot").to_string();
                    ctx.app_id = v["d"]["application"]["id"].as_str().unwrap_or_default().to_string();
                    ctx.sync_commands();
                }
                Some("INTERACTION_CREATE") => on_interaction(ctx, &v["d"]),
                _ => {}
            },
            _ => {}
        }
    }
}

/// The slash commands MacroFlow offers (replaces whatever it set up before).
/// The command list for Discord: the bot's own commands, then one per trigger.
fn commands_json(triggers: Vec<TriggerCmd>) -> Value {
    let cmd = |name: &str, description: &str| {
        json!({ "name": name, "description": short(description, 100), "type": 1, "contexts": [0, 1] })
    };
    let mut commands = vec![
        cmd("start", "Run the macro from its Start step"),
        cmd("stop", "Stop the macro that's running"),
        cmd("pause", "Pause or resume the macro that's running"),
        cmd("status", "What MacroFlow is doing right now"),
        cmd("screenshot", "A picture of the main screen right now"),
        cmd("panel", "Post buttons that control MacroFlow"),
    ];
    // Each Trigger step is a command of its own (Discord allows 100).
    for (t, inputs) in triggers.into_iter().take(90) {
        let mut c = cmd(&t, &format!("⚡ Run the “{t}” trigger in MacroFlow"));
        // Discord wants the required values listed before the optional ones.
        let mut inputs = inputs;
        inputs.sort_by_key(|i| i.default.is_some());
        let options: Vec<Value> = inputs
            .iter()
            .map(|i| {
                let description = match &i.default {
                    Some(d) if !d.is_empty() => format!("Stored in {{{}}} (default {d})", i.name),
                    _ => format!("Stored in {{{}}}", i.name),
                };
                json!({
                    // 10 = number, 3 = text
                    "type": if i.number { 10 } else { 3 },
                    "name": i.name,
                    "description": short(&description, 100),
                    "required": i.default.is_none(),
                })
            })
            .collect();
        if !options.is_empty() {
            c["options"] = Value::Array(options);
        }
        commands.push(c);
    }
    Value::Array(commands)
}

/// Send the command list to Discord (replacing what it had).
fn register_commands(ctx: &BotCtx) -> Result<(), String> {
    if ctx.app_id.is_empty() {
        return Err("Discord didn't say which app this is".into());
    }
    let commands = commands_json(ctx.trigger_cmds());
    let url = format!("{API}/applications/{}/commands", ctx.app_id);
    let auth = format!("Bot {}", ctx.token);
    let r = request(Method::Put, &url, Some(&auth), "application/json", commands.to_string().as_bytes())?;
    if r.ok() {
        Ok(())
    } else {
        Err(explain(&r))
    }
}

/// Answer an interaction straight away (Discord gives us 3 seconds).
fn respond(id: &str, token: &str, body: Value) {
    let url = format!("{API}/interactions/{id}/{token}/callback");
    let _ = post_json(&url, None, &body);
}

fn ephemeral(text: &str) -> Value {
    json!({ "type": 4, "data": { "content": discord_text(text), "flags": EPHEMERAL } })
}

fn on_interaction(ctx: &BotCtx, d: &Value) {
    let (Some(id), Some(token)) = (d["id"].as_str(), d["token"].as_str()) else { return };
    let kind = d["type"].as_u64().unwrap_or(0);
    // In a server the user is under "member"; in a DM it's "user".
    let user_id = d["member"]["user"]["id"].as_str().or_else(|| d["user"]["id"].as_str()).unwrap_or("");
    let allowed = !user_id.is_empty() && ctx.allowed.iter().any(|a| a == user_id);

    if !allowed {
        let text = format!(
            "You're not allowed to control this MacroFlow. To allow yourself, add your Discord user ID `{user_id}` under Remote → Allowed user IDs in MacroFlow."
        );
        respond(id, token, ephemeral(&text));
        return;
    }

    // What was asked for: a slash command (2) or a button (3).
    let action = match kind {
        2 => {
            let name = d["data"]["name"].as_str().unwrap_or("");
            match name {
                "start" => Action::Cmd(RemoteCmd::Run(None, Vec::new())),
                "stop" => Action::Cmd(RemoteCmd::Stop),
                "pause" => Action::Cmd(RemoteCmd::Pause),
                "status" => Action::Cmd(RemoteCmd::Status),
                "screenshot" => Action::Screenshot,
                "panel" => Action::Panel,
                // Anything else is a trigger's own command (the editor checks it
                // still exists in the open macro).
                trigger if !trigger.is_empty() => {
                    // The values typed into the command's options, as text.
                    let given: Vec<(String, String)> = d["data"]["options"]
                        .as_array()
                        .map(|opts| {
                            opts.iter()
                                .filter_map(|o| {
                                    let name = o["name"].as_str()?.to_string();
                                    let value = match &o["value"] {
                                        Value::String(s) => s.clone(),
                                        Value::Number(n) => match n.as_f64() {
                                            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
                                            _ => n.to_string(),
                                        },
                                        Value::Bool(b) => b.to_string(),
                                        _ => return None,
                                    };
                                    Some((name, value))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Action::Cmd(RemoteCmd::Run(Some(trigger.to_string()), given))
                }
                _ => return,
            }
        }
        3 => {
            let custom = d["data"]["custom_id"].as_str().unwrap_or("");
            match custom {
                "mf:start" => Action::Cmd(RemoteCmd::Run(None, Vec::new())),
                "mf:stop" => Action::Cmd(RemoteCmd::Stop),
                "mf:pause" => Action::Cmd(RemoteCmd::Pause),
                "mf:status" => Action::Cmd(RemoteCmd::Status),
                "mf:shot" => Action::Screenshot,
                c => match c.strip_prefix("mf:run:") {
                    Some(name) => Action::Cmd(RemoteCmd::Run(Some(name.to_string()), Vec::new())),
                    None => return,
                },
            }
        }
        _ => return,
    };

    if let Action::Panel = action {
        respond(id, token, panel(&ctx.triggers()));
        return;
    }
    // "MacroFlow is thinking…" now; the real answer replaces it.
    respond(id, token, json!({ "type": 5, "data": { "flags": EPHEMERAL } }));
    let reply = Reply {
        app_id: d["application_id"].as_str().unwrap_or(&ctx.app_id).to_string(),
        token: token.to_string(),
        channel_id: d["channel_id"].as_str().or_else(|| d["channel"]["id"].as_str()).unwrap_or("").to_string(),
        user_id: user_id.to_string(),
        at: Instant::now(),
    };
    match action {
        Action::Cmd(cmd) => {
            let _ = ctx.tx.send(WorkerMsg::Remote { cmd, reply });
        }
        Action::Screenshot => {
            thread::spawn(move || {
                let result = screenshot_png().and_then(|png| reply.answer_png("📷 The main screen right now:", png));
                if let Err(e) = result {
                    reply.answer(&format!("Couldn't take a screenshot: {e}"));
                }
            });
        }
        Action::Panel => {}
    }
}

enum Action {
    Cmd(RemoteCmd),
    Screenshot,
    Panel,
}

/// A message with buttons: Start / Pause / Stop / Status / Screenshot, then one
/// button per trigger.
fn panel(triggers: &[String]) -> Value {
    let button = |style: u8, label: &str, id: String| json!({ "type": 2, "style": style, "label": short(label, 78), "custom_id": id });
    let mut rows = vec![json!({
        "type": 1,
        "components": [
            button(3, "▶ Start", "mf:start".into()),
            button(2, "❚❚ Pause / resume", "mf:pause".into()),
            button(4, "■ Stop", "mf:stop".into()),
            button(2, "Status", "mf:status".into()),
            button(2, "📷 Screenshot", "mf:shot".into()),
        ]
    })];
    // Up to 4 more rows of 5 trigger buttons (Discord's limit is 5 rows).
    let usable: Vec<&String> = triggers.iter().filter(|t| format!("mf:run:{t}").len() <= 100).take(20).collect();
    for chunk in usable.chunks(5) {
        rows.push(json!({
            "type": 1,
            "components": chunk.iter().map(|t| button(1, &format!("⚡ /{t}"), format!("mf:run:{t}"))).collect::<Vec<_>>(),
        }));
    }
    let text = if triggers.is_empty() {
        "**MacroFlow controls**\nAdd Trigger steps to your macro to get a button (and a slash command) for each one. Run `/panel` again afterwards.".to_string()
    } else {
        "**MacroFlow controls**".to_string()
    };
    json!({ "type": 4, "data": { "content": text, "components": rows } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ids_are_parsed_loosely() {
        assert_eq!(
            parse_user_ids("123456789012345678, 987654321098765432\n  abc 12"),
            vec!["123456789012345678".to_string(), "987654321098765432".to_string()]
        );
        assert!(parse_user_ids("").is_empty());
    }

    #[test]
    fn long_messages_are_cut_for_discord() {
        let long = "x".repeat(3000);
        assert_eq!(discord_text(&long).chars().count(), MAX_MESSAGE);
        assert_eq!(discord_text("hi"), "hi");
    }

    #[test]
    fn multipart_has_both_parts() {
        let (ct, body) = multipart(&json!({ "content": "hi" }), Some(b"PNGDATA"));
        let boundary = ct.split("boundary=").nth(1).unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("name=\"payload_json\""));
        assert!(text.contains("name=\"files[0]\"; filename=\"screen.png\""));
        assert!(text.contains("PNGDATA"));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }

    #[test]
    fn trigger_inputs_become_command_options() {
        let inputs = crate::model::parse_inputs("times:number=1, message").unwrap();
        let cmds = commands_json(vec![("farm".into(), inputs)]);
        let farm = cmds.as_array().unwrap().iter().find(|c| c["name"] == "farm").unwrap();
        let opts = farm["options"].as_array().unwrap();
        // Required first, as Discord insists.
        assert_eq!(opts[0]["name"], "message");
        assert_eq!(opts[0]["type"], 3);
        assert_eq!(opts[0]["required"], true);
        assert_eq!(opts[1]["name"], "times");
        assert_eq!(opts[1]["type"], 10);
        assert_eq!(opts[1]["required"], false);
        // The bot's own commands are always there.
        assert!(cmds.as_array().unwrap().iter().any(|c| c["name"] == "start"));
    }

    #[test]
    fn trigger_commands_are_cleaned_up() {
        let t = |n: &str| (n.to_string(), Vec::new());
        let names: Vec<String> = command_names(&[t("Farm"), t("farm"), t("stop"), t("!!"), t("Collect gold")]).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["farm".to_string(), "collect-gold".to_string()]);
    }

    #[test]
    fn panel_has_a_button_per_trigger() {
        let p = panel(&["farm".into(), "collect".into()]);
        let rows = p["data"]["components"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["components"][0]["custom_id"], "mf:run:farm");
    }

    /// Needs the internet: `cargo test -- --ignored gateway_says_hello`.
    #[test]
    #[ignore]
    fn gateway_says_hello() {
        let (mut ws, _) = tungstenite::connect(GATEWAY).expect("connect");
        let msg = ws.read().expect("read");
        let v: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        assert_eq!(v["op"], 10);
        // And plain HTTPS through ureq: an anonymous call is refused, but TLS worked.
        let r = request(Method::Post, &format!("{API}/webhooks/0/x"), None, "application/json", b"{}").unwrap();
        assert!(r.status >= 400);
    }

    #[test]
    fn bad_urls_are_refused_before_sending() {
        assert!(send_custom("not a url", "{}").is_err());
    }
}
