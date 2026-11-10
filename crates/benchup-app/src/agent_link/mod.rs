//! "Agent connection": lets an external agent (through the `benchup-mcp`
//! bridge) edit the open project. The app connects out to the bridge on
//! `ws://127.0.0.1:<port>`, proves it knows the pairing token (protocol 2: a
//! mutual challenge, the token itself is never sent), then receives tool calls
//! and answers each with a result.
//!
//! Calls run through `agent_host` (one undo step each). While a MIRA turn is
//! in flight, or a file operation is replacing the model, calls wait in a
//! FIFO queue and run afterwards, in order.
//!
//! Frames (JSON text). The socket first sends `authenticate` with an EMPTY
//! ticket; the handshake then runs:
//! * bridge to app: `challenge {nonce, protocol}`
//! * app to bridge: `prove {nonce, proof}`
//! * bridge to app: `ready {proof}`, verified before anything else is trusted
//!
//! After that:
//! * bridge to app: `error`, `cancel {id}`,
//!   `call {id, name, arguments, client, deadline_ms}`
//! * app to bridge: `result {id, ok, result}`, `status {message}`
//!
//! A call whose deadline has (almost) passed is dropped unanswered, and one
//! queued for a project that has since been replaced is refused.
use super::*;
use benchup_core::tr;
use serde_json::{Value, json};
use benchup_tools::pairing;
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
/// The largest message accepted from the bridge (the browser build allows
/// about the same).
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
/// A call is not run in the last second before its deadline.
const DEADLINE_MARGIN_MS: u64 = 1000;
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
        // The raw token is never sent: the ticket is empty and the bridge proves
        // itself with the challenge handshake instead.
        let _ = token;
        Box::new(SocketLink(crate::realtime::Socket::connect_limited(
            ctx,
            url.into(),
            String::new(),
            connection.into(),
            Some(MAX_MESSAGE_BYTES),
        )))
    }
}

/// What the bridge can send.
#[derive(Debug, PartialEq)]
pub(super) enum Frame {
    Challenge { nonce: String, protocol: Option<u64> },
    Ready { proof: Option<String>, protocol: Option<u64> },
    Error(String),
    Cancel { id: String },
    Call { id: String, client: String, deadline_ms: Option<u64>, call: Result<benchup_tools::Call, String> },
    /// Unknown type, or not an object: ignored.
    Ignored,
}

/// Characters shown in the UI: not control (Cc) and not invisible formatting
/// (Cf: bidi overrides and isolates, zero-width characters, byte-order mark,
/// soft hyphen, tag characters) or line/paragraph separators.
fn printable(c: char) -> bool {
    !c.is_control()
        && !matches!(c as u32,
            0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x180E
            | 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x206F
            | 0xFEFF | 0xFFF9..=0xFFFB | 0xE0001 | 0xE0020..=0xE007F)
}
fn clean_client(client: &str) -> String {
    let clean: String = client.chars().filter(|c| printable(*c)).collect();
    clean.trim().chars().take(MAX_CLIENT_CHARS).collect::<String>().trim().to_owned()
}

/// Unix time in milliseconds (the unit of a call's `deadline_ms`).
fn now_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
}

/// 32 random bytes as 64 lowercase hex characters, or an error if the system
/// has no secure randomness.
fn new_nonce() -> Result<String, String> {
    std::panic::catch_unwind(|| {
        let (a, b) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let bytes: Vec<u8> = a.as_bytes().iter().chain(b.as_bytes()).copied().collect();
        pairing::to_hex(&bytes)
    })
    .map_err(|_| tr!("agent_link.error.no_randomness").to_owned())
}

/// Strict frame parsing: anything unexpected is `Ignored`, except a call that
/// has an id, which is always answered (with an error if it is malformed).
pub(super) fn parse_frame(value: &Value) -> Frame {
    let Some(object) = value.as_object() else { return Frame::Ignored };
    match object.get("type").and_then(Value::as_str) {
        Some("challenge") => Frame::Challenge {
            nonce: object.get("nonce").and_then(Value::as_str).unwrap_or_default().to_owned(),
            protocol: object.get("protocol").and_then(Value::as_u64),
        },
        Some("ready") => Frame::Ready {
            proof: object.get("proof").and_then(Value::as_str).map(str::to_owned),
            protocol: object.get("protocol").and_then(Value::as_u64),
        },
        Some("cancel") => match object.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) {
            Some(id) => Frame::Cancel { id: id.to_owned() },
            None => Frame::Ignored,
        },
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
            let deadline_ms = object.get("deadline_ms").and_then(|v| {
                v.as_u64().or_else(|| v.as_f64().filter(|f| f.is_finite() && *f >= 0.).map(|f| f as u64))
            });
            Frame::Call { id: id.to_owned(), client, deadline_ms, call }
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
    /// Unix ms after which the bridge no longer wants an answer.
    deadline_ms: u64,
    /// The document epoch when the call arrived.
    epoch: u64,
}

/// Where the pairing handshake is.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// Waiting for the bridge's `challenge`.
    Challenge,
    /// `prove` sent; waiting for a `ready` carrying the bridge's proof.
    Proved { bridge_nonce: String, app_nonce: String },
    /// The bridge proved the token. Only now are calls accepted.
    Ready,
}

struct Active {
    link: Box<dyn Link>,
    port: u16,
    token: String,
    started: f64,
    phase: Phase,
}

pub(super) struct AgentLink {
    pub status: Status,
    /// The pairing token, loaded from this device on demand.
    pub token: Option<String>,
    pub token_error: Option<String>,
    token_checked: bool,
    /// Makes tokens; replaced in tests to simulate a missing RNG.
    generator: fn() -> Result<String, String>,
    /// Session-only: revealing a secret is never remembered.
    pub show_token: bool,
    active: Option<Active>,
    connector: Box<dyn Connector>,
    queue: VecDeque<Queued>,
    retry_at: f64,
    backoff: f64,
    /// The open document, refreshed by the app every frame: replies carry it.
    document_name: String,
    epoch: u64,
    /// Makes the app's handshake nonce; replaced in tests.
    nonce_source: fn() -> Result<String, String>,
}

impl Default for AgentLink {
    fn default() -> Self {
        Self::with_connector(Box::new(SocketConnector))
    }
}

/// A new random version-4 UUID from the OS (native) or Web Crypto (browser).
/// If no secure randomness is available no token is made; it is never guessable.
pub(super) fn new_token() -> Result<String, String> {
    // `uuid` panics rather than return weak bytes when the system RNG fails.
    std::panic::catch_unwind(|| uuid::Uuid::new_v4().to_string())
        .map_err(|_| tr!("agent_link.error.no_randomness").to_owned())
}

/// `claude mcp add` command for this token (and port, when not the default).
/// The token goes in the environment, never in the program's arguments (which
/// other local users can read).
pub(super) fn claude_command(token: &str, port: u16) -> String {
    let port = if port == DEFAULT_PORT { String::new() } else { format!(" --port {port}") };
    format!("claude mcp add benchup -e BENCHUP_AGENT_TOKEN={token} -- benchup-mcp live{port}")
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
            generator: new_token,
            show_token: false,
            active: None,
            connector,
            queue: VecDeque::new(),
            retry_at: 0.,
            backoff: BACKOFF_START,
            document_name: String::new(),
            epoch: 0,
            nonce_source: new_nonce,
        }
    }

    /// Read the saved token, without creating one (safe to call from the UI).
    pub(super) fn load_token(&mut self) {
        if !self.token_checked {
            self.token_checked = true;
            self.token = credentials::load(TOKEN_FILE, TOKEN_STORAGE);
        }
    }
    /// The saved token, or a new one saved now; `None` if none could be made
    /// (the reason is in `token_error`).
    fn ensure_token(&mut self) -> Option<String> {
        self.load_token();
        if self.token.is_none() {
            self.regenerate();
        }
        self.token.clone()
    }
    /// Replace the token. Connections made with the old one are dropped. If a
    /// new token cannot be made the old one stays and `token_error` says why.
    pub(super) fn regenerate(&mut self) {
        match (self.generator)() {
            Ok(token) => {
                self.token_error = credentials::save(TOKEN_FILE, TOKEN_STORAGE, &token).err();
                self.token_checked = true;
                self.token = Some(token);
            }
            Err(error) => self.token_error = Some(error),
        }
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
    /// A `result` frame; every result object says which project was open, so
    /// an agent can notice a switch.
    fn result_frame(&self, id: &str, ok: bool, mut result: Value) -> Value {
        if let Some(object) = result.as_object_mut() {
            object.insert("document".into(), json!({"name": self.document_name, "epoch": self.epoch}));
        }
        json!({"type": "result", "id": id, "ok": ok, "result": result})
    }
    fn send_result(&mut self, id: &str, ok: bool, result: Value) {
        let frame = self.result_frame(id, ok, result);
        self.send(frame);
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
                for queued in std::mem::take(&mut self.queue) {
                    let reply = self.result_frame(&queued.id, false, json!({"ok": false, "error": tr!("agent_link.error.disabled")}));
                    let _ = active.link.send(reply);
                }
            }
            self.queue.clear();
            self.status = Status::Off;
            self.backoff = BACKOFF_START;
            self.retry_at = 0.;
            return;
        }
        let Some(token) = self.ensure_token() else {
            // No secure token, so nothing connects.
            self.stop();
            self.status = Status::Error(self.token_error.clone().unwrap_or_default());
            return;
        };
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
            self.active = Some(Active { link, port: prefs.port, token: token.clone(), started: now, phase: Phase::Challenge });
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
            let Some(phase) = self.active.as_ref().map(|a| a.phase.clone()) else { return };
            match parse_frame(&value) {
                Frame::Challenge { nonce, protocol } => {
                    // Exactly one challenge, first.
                    if phase != Phase::Challenge {
                        return self.fail(now, tr!("agent_link.error.handshake").into());
                    }
                    if protocol != Some(u64::from(pairing::PROTOCOL)) {
                        return self.fail(now, tr!("agent_link.error.protocol").into());
                    }
                    if !pairing::valid_nonce(&nonce) {
                        return self.fail(now, tr!("agent_link.error.handshake").into());
                    }
                    let app_nonce = match (self.nonce_source)() {
                        Ok(nonce) => nonce,
                        Err(message) => return self.fail(now, message),
                    };
                    let proof = pairing::app_proof(&token, &nonce, &app_nonce);
                    self.send(json!({"type": "prove", "nonce": app_nonce, "proof": proof}));
                    if let Some(active) = &mut self.active {
                        active.phase = Phase::Proved { bridge_nonce: nonce, app_nonce };
                    }
                }
                Frame::Ready { proof, protocol } => match phase {
                    // A bridge that skips the challenge speaks an older protocol.
                    Phase::Challenge => return self.fail(now, tr!("agent_link.error.protocol").into()),
                    Phase::Proved { bridge_nonce, app_nonce } => {
                        if protocol.is_some_and(|p| p != u64::from(pairing::PROTOCOL)) {
                            return self.fail(now, tr!("agent_link.error.protocol").into());
                        }
                        let expected = pairing::bridge_proof(&token, &bridge_nonce, &app_nonce);
                        if !proof.as_deref().is_some_and(|proof| pairing::proofs_match(proof, &expected)) {
                            return self.fail(now, tr!("agent_link.error.unproven").into());
                        }
                        if let Some(active) = &mut self.active {
                            active.phase = Phase::Ready;
                        }
                        self.status = Status::Connected { client: None };
                        self.backoff = BACKOFF_START;
                    }
                    Phase::Ready => {}
                },
                Frame::Error(message) => return self.fail(now, tr!("agent_link.error.bridge", message = message)),
                Frame::Cancel { id } => {
                    if phase == Phase::Ready {
                        self.queue.retain(|queued| queued.id != id);
                    }
                }
                Frame::Call { id, client, deadline_ms, call } => {
                    if phase != Phase::Ready {
                        // Calls before a verified `ready` are not trusted.
                        continue;
                    }
                    let received = now_ms();
                    // No deadline: the bridge's own limit, as a cap.
                    let deadline_ms = deadline_ms.unwrap_or(received + (QUEUE_TIMEOUT * 1000.) as u64);
                    match call {
                        Err(message) => self.refuse(&id, &message),
                        Ok(_) if self.queue.len() >= QUEUE_LIMIT => self.refuse(&id, tr!("agent_link.error.queue_full")),
                        Ok(call) => {
                            if !client.is_empty() {
                                self.status = Status::Connected { client: Some(client.clone()) };
                            }
                            let epoch = self.epoch;
                            self.queue.push_back(Queued { id, client, call, queued_at: now, notified: false, deadline_ms, epoch });
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
        // The bridge has already answered calls that are about to pass their
        // deadline; running them would only change the model unseen.
        let limit = now_ms() + DEADLINE_MARGIN_MS;
        self.queue.retain(|queued| queued.deadline_ms > limit);
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
        self.agent_link.epoch = self.doc_epoch;
        self.agent_link.document_name = self.doc.name.clone();
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
            if queued.deadline_ms <= now_ms() + DEADLINE_MARGIN_MS {
                // The bridge has already answered; do not run it.
                continue;
            }
            if queued.epoch != self.doc_epoch {
                let refusal = json!({
                    "ok": false,
                    "error": tr!("agent_link.error.document_changed"),
                    "hint": tr!("agent_link.error.document_changed_hint"),
                });
                self.agent_link.send_result(&queued.id, false, refusal);
                continue;
            }
            let (ok, result) = self.run_agent_call(queued.call, &queued.client, ctx);
            self.agent_link.document_name = self.doc.name.clone();
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
