//! "Agent connection": lets an external agent (through the `benchup-mcp`
//! bridge) edit the open project. The app connects out to the bridge on
//! `ws://127.0.0.1:<port>`, proves it knows the pairing token with the first
//! frame, then receives tool calls and answers each with a result.
//!
//! Calls run through `agent_host` (one undo step each). While a MIRA turn is
//! in flight, or a file operation is replacing the model, calls wait in a
//! FIFO queue and run afterwards, in order.
//!
//! Frames (JSON text), after the `authenticate` frame the socket sends:
//! * bridge to app: `ready`, `error`, `call {id, name, arguments, client}`
//! * app to bridge: `result {id, ok, result}`, `status {message}`
use super::*;
use benchup_core::tr;
use serde_json::{Value, json};
use std::{collections::VecDeque, time::Duration};

mod ui;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

const BACKOFF_START: f64 = 1.;
const BACKOFF_MAX: f64 = 10.;
/// The bridge must answer `authenticate` with `ready` within this long.
const READY_TIMEOUT: f64 = 15.;
/// The bridge gives up on a call after 120 s; answer first.
const QUEUE_TIMEOUT: f64 = 100.;
const QUEUE_LIMIT: usize = 64;
/// Calls run per frame, so a burst cannot freeze the UI.
const CALLS_PER_FRAME: usize = 8;
const MAX_CLIENT_CHARS: usize = 64;
const TOKEN_FILE: &str = "agent-link.token";
const TOKEN_STORAGE: &str = "benchup.agent_link.token";

pub(super) use addons::{AgentPreferences as Preferences, DEFAULT_PORT};

impl Preferences {
    pub(super) fn normalize(&mut self) {
        if self.port == 0 {
            self.port = DEFAULT_PORT;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Status {
    Off,
    Connecting,
    Connected { client: Option<String> },
    Error(String),
}

/// One live connection, abstracted so tests need no bridge.
pub(super) trait Link {
    /// Everything received since the last call. An `Err` ends the connection.
    fn events(&mut self) -> Vec<Result<Value, String>>;
    fn send(&mut self, message: Value) -> Result<(), String>;
}
pub(super) trait Connector {
    /// Start connecting; the first frame (`authenticate`) is the link's job.
    fn connect(&mut self, ctx: &egui::Context, url: &str, token: &str, connection: &str) -> Box<dyn Link>;
}

struct SocketLink(crate::realtime::Socket);
impl Link for SocketLink {
    fn events(&mut self) -> Vec<Result<Value, String>> {
        use std::sync::mpsc::TryRecvError;
        let mut events = Vec::new();
        loop {
            match self.0.events.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    events.push(Err(tr!("realtime.websocket_disconnected").into()));
                    break;
                }
            }
        }
        events
    }
    fn send(&mut self, message: Value) -> Result<(), String> {
        self.0.send(message)
    }
}
struct SocketConnector;
impl Connector for SocketConnector {
    fn connect(&mut self, ctx: &egui::Context, url: &str, token: &str, connection: &str) -> Box<dyn Link> {
        Box::new(SocketLink(crate::realtime::Socket::connect(ctx, url.into(), token.into(), connection.into())))
    }
}

/// What the bridge can send.
#[derive(Debug, PartialEq)]
pub(super) enum Frame {
    Ready,
    Error(String),
    Call { id: String, client: String, call: Result<benchup_tools::Call, String> },
    /// Unknown type, or not an object: ignored.
    Ignored,
}

fn clean_client(client: &str) -> String {
    client.chars().filter(|c| !c.is_control()).take(MAX_CLIENT_CHARS).collect::<String>().trim().to_owned()
}

/// Strict frame parsing: anything unexpected is `Ignored`, except a call that
/// has an id, which is always answered (with an error if it is malformed).
pub(super) fn parse_frame(value: &Value) -> Frame {
    let Some(object) = value.as_object() else { return Frame::Ignored };
    match object.get("type").and_then(Value::as_str) {
        Some("ready") => Frame::Ready,
        Some("error") => Frame::Error(
            object.get("message").and_then(Value::as_str).unwrap_or_default().chars().take(200).collect(),
        ),
        Some("call") => {
            let Some(id) = object.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) else {
                return Frame::Ignored;
            };
            let client = object.get("client").and_then(Value::as_str).map(clean_client).unwrap_or_default();
            let call = (|| {
                let name = object
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| tr!("agent_link.error.missing_tool").to_owned())?;
                let arguments = match object.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(arguments) if arguments.is_object() => arguments.clone(),
                    Some(_) => return Err(tr!("agent_link.error.arguments_not_object").to_owned()),
                };
                Ok(benchup_tools::Call::new(id, name, arguments))
            })();
            Frame::Call { id: id.to_owned(), client, call }
        }
        _ => Frame::Ignored,
    }
}

struct Queued {
    id: String,
    client: String,
    call: benchup_tools::Call,
    queued_at: f64,
    /// The bridge was told the call is waiting.
    notified: bool,
}

struct Active {
    link: Box<dyn Link>,
    port: u16,
    token: String,
    started: f64,
}

pub(super) struct AgentLink {
    pub status: Status,
    /// The pairing token, loaded from this device on demand.
    pub token: Option<String>,
    pub token_error: Option<String>,
    token_checked: bool,
    /// Session-only: revealing a secret is never remembered.
    pub show_token: bool,
    active: Option<Active>,
    connector: Box<dyn Connector>,
    queue: VecDeque<Queued>,
    retry_at: f64,
    backoff: f64,
}

impl Default for AgentLink {
    fn default() -> Self {
        Self::with_connector(Box::new(SocketConnector))
    }
}

/// A random version-4 UUID, from the OS (native) or the browser's crypto.
fn random_bytes() -> [u8; 16] {
    #[cfg(target_arch = "wasm32")]
    {
        use eframe::wasm_bindgen::JsCast as _;
        let array = js_sys::Uint8Array::new_with_length(16);
        let crypto = js_sys::Reflect::get(&js_sys::global(), &"crypto".into()).ok();
        let filled = crypto.is_some_and(|crypto| {
            js_sys::Reflect::get(&crypto, &"getRandomValues".into())
                .ok()
                .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
                .is_some_and(|f| f.call1(&crypto, &array).is_ok())
        });
        let mut bytes = [0u8; 16];
        array.copy_to(&mut bytes);
        if !filled {
            // No Web Crypto: refuse to make a guessable token.
            bytes = [0; 16];
        }
        bytes
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::{
            collections::hash_map::RandomState,
            hash::{BuildHasher, Hasher},
            io::Read,
        };
        let mut bytes = [0u8; 16];
        #[cfg(unix)]
        if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).is_ok() {
            return bytes;
        }
        // std seeds every `RandomState` from the OS.
        for half in bytes.chunks_mut(8) {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
            half.copy_from_slice(&hasher.finish().to_le_bytes());
        }
        bytes
    }
}
pub(super) fn new_token() -> String {
    let mut b = random_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// `claude mcp add` command for this token (and port, when not the default).
pub(super) fn claude_command(token: &str, port: u16) -> String {
    let port = if port == DEFAULT_PORT { String::new() } else { format!(" --port {port}") };
    format!("claude mcp add benchup -- benchup-mcp live{port} --token {token}")
}
/// Generic `mcpServers` entry; the token travels in the environment.
pub(super) fn mcp_json(token: &str, port: u16) -> String {
    let mut args = vec![json!("live")];
    if port != DEFAULT_PORT {
        args.extend([json!("--port"), json!(port.to_string())]);
    }
    serde_json::to_string_pretty(&json!({
        "mcpServers": {"benchup": {"command": "benchup-mcp", "args": args, "env": {"BENCHUP_AGENT_TOKEN": token}}}
    }))
    .unwrap_or_default()
}

impl AgentLink {
    pub(super) fn with_connector(connector: Box<dyn Connector>) -> Self {
        Self {
            status: Status::Off,
            token: None,
            token_error: None,
            token_checked: false,
            show_token: false,
            active: None,
            connector,
            queue: VecDeque::new(),
            retry_at: 0.,
            backoff: BACKOFF_START,
        }
    }

    /// Read the saved token, without creating one (safe to call from the UI).
    pub(super) fn load_token(&mut self) {
        if !self.token_checked {
            self.token_checked = true;
            self.token = credentials::load(TOKEN_FILE, TOKEN_STORAGE);
        }
    }
    /// The saved token, or a new one saved now.
    fn ensure_token(&mut self) -> String {
        self.load_token();
        if let Some(token) = &self.token {
            return token.clone();
        }
        self.regenerate();
        self.token.clone().unwrap_or_default()
    }
    /// Replace the token. Connections made with the old one are dropped.
    pub(super) fn regenerate(&mut self) {
        let token = new_token();
        self.token_error = credentials::save(TOKEN_FILE, TOKEN_STORAGE, &token).err();
        self.token_checked = true;
        self.token = Some(token);
    }

    /// Test hook: use this token without touching the credentials file.
    #[cfg(test)]
    pub(super) fn set_token_for_test(&mut self, token: &str) {
        self.token = Some(token.into());
        self.token_checked = true;
    }

    pub(super) fn queued(&self) -> usize {
        self.queue.len()
    }
    pub(super) fn connected(&self) -> bool {
        matches!(self.status, Status::Connected { .. })
    }

    fn send(&mut self, message: Value) {
        if let Some(active) = &mut self.active {
            let _ = active.link.send(message);
        }
    }
    fn send_result(&mut self, id: &str, ok: bool, result: Value) {
        self.send(json!({"type": "result", "id": id, "ok": ok, "result": result}));
    }
    fn refuse(&mut self, id: &str, message: &str) {
        self.send_result(id, false, json!({"ok": false, "error": message}));
    }

    /// Drop the connection; queued calls can no longer be answered.
    fn stop(&mut self) {
        self.active = None;
        self.queue.clear();
    }
    fn fail(&mut self, now: f64, message: String) {
        self.stop();
        self.status = Status::Error(message);
        self.retry_at = now + self.backoff;
        self.backoff = (self.backoff * 2.).min(BACKOFF_MAX);
    }

    /// Keep the connection in the state the preferences ask for, and collect
    /// incoming calls into the queue. Never blocks.
    pub(super) fn pump(&mut self, ctx: &egui::Context, now: f64, prefs: &Preferences) {
        if !prefs.enabled {
            if let Some(mut active) = self.active.take() {
                // Answer what is waiting, then close.
                for queued in self.queue.drain(..) {
                    let reply = json!({"type": "result", "id": queued.id, "ok": false, "result": {"ok": false, "error": tr!("agent_link.error.disabled")}});
                    let _ = active.link.send(reply);
                }
            }
            self.queue.clear();
            self.status = Status::Off;
            self.backoff = BACKOFF_START;
            self.retry_at = 0.;
            return;
        }
        let token = self.ensure_token();
        if self.active.as_ref().is_some_and(|a| a.port != prefs.port || a.token != token) {
            self.stop();
            self.backoff = BACKOFF_START;
            self.retry_at = 0.;
        }
        if self.active.is_none() {
            if now < self.retry_at {
                ctx.request_repaint_after(Duration::from_secs_f64(self.retry_at - now));
                return;
            }
            let url = format!("ws://127.0.0.1:{}", prefs.port);
            let link = self.connector.connect(ctx, &url, &token, concat!("benchup-app-", env!("CARGO_PKG_VERSION")));
            self.active = Some(Active { link, port: prefs.port, token, started: now });
            self.status = Status::Connecting;
        }
        let Some(active) = &mut self.active else { return };
        let events = active.link.events();
        let started = active.started;
        for event in events {
            let value = match event {
                Ok(value) => value,
                Err(message) => return self.fail(now, message),
            };
            match parse_frame(&value) {
                Frame::Ready => {
                    if !self.connected() {
                        self.status = Status::Connected { client: None };
                        self.backoff = BACKOFF_START;
                    }
                }
                Frame::Error(message) => return self.fail(now, tr!("agent_link.error.bridge", message = message)),
                Frame::Call { id, client, call } => {
                    if !self.connected() {
                        // Calls before `ready` are not trusted.
                        continue;
                    }
                    match call {
                        Err(message) => self.refuse(&id, &message),
                        Ok(_) if self.queue.len() >= QUEUE_LIMIT => self.refuse(&id, tr!("agent_link.error.queue_full")),
                        Ok(call) => {
                            if !client.is_empty() {
                                self.status = Status::Connected { client: Some(client.clone()) };
                            }
                            self.queue.push_back(Queued { id, client, call, queued_at: now, notified: false });
                        }
                    }
                }
                Frame::Ignored => {}
            }
        }
        if self.status == Status::Connecting {
            if now - started > READY_TIMEOUT {
                self.fail(now, tr!("agent_link.error.connect_timeout").into());
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64((READY_TIMEOUT - (now - started)).max(0.1)));
            }
        }
    }

    /// Answer calls that waited longer than the bridge will.
    fn expire(&mut self, now: f64) {
        while self.queue.front().is_some_and(|q| now - q.queued_at > QUEUE_TIMEOUT) {
            if let Some(queued) = self.queue.pop_front() {
                self.refuse(&queued.id, tr!("agent_link.error.busy"));
            }
        }
    }
    /// Tell the bridge once that a call is waiting.
    fn notify_waiting(&mut self) {
        let mut fresh = false;
        for queued in self.queue.iter_mut().filter(|q| !q.notified) {
            queued.notified = true;
            fresh = true;
        }
        if fresh {
            self.send(json!({"type": "status", "message": tr!("agent_link.queued_status")}));
        }
    }
}

impl BenchUpApp {
    /// Called once per frame from `ui`, next to `poll_assistant`.
    pub(super) fn poll_agent_link(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let prefs = self.general_settings.agent_link.clone();
        self.agent_link.pump(ctx, now, &prefs);
        self.agent_link.expire(now);
        let mut ran = 0;
        // User edits and MIRA turns never interleave with agent calls: wait
        // for a MIRA turn, or for a file operation that replaces the model.
        while !self.assistant.busy() && !self.project.blocked() {
            if ran >= CALLS_PER_FRAME {
                ctx.request_repaint();
                break;
            }
            let Some(queued) = self.agent_link.queue.pop_front() else { break };
            let (ok, result) = self.run_agent_call(queued.call, &queued.client, ctx);
            self.agent_link.send_result(&queued.id, ok, result);
            ran += 1;
        }
        if self.agent_link.queued() > 0 && (self.assistant.busy() || self.project.blocked()) {
            self.agent_link.notify_waiting();
            // MIRA repaints itself while busy; a file operation does not.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    /// Short status-bar text while an agent is connected.
    pub(super) fn agent_status_text(&self) -> Option<&'static str> {
        self.agent_link.connected().then_some(tr!("agent_link.status_bar"))
    }
}
