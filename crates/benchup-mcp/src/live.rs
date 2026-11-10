//! `live` mode: forward tool calls to the running BenchUp app over a
//! loopback-only WebSocket. The app connects to us and authenticates with a
//! shared token; one authenticated app at a time.
use crate::protocol::Backend;
use serde_json::{Value, json};
use benchup_tools::pairing;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, handshake::server::{ErrorResponse, Request, Response}};

pub const DEFAULT_PORT: u16 = 47615;
pub const DEFAULT_ORIGIN: &str = "https://benchup3d.github.io";
pub const MIN_TOKEN_LEN: usize = 16;
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(25);
/// Largest message accepted before the app has proved the pairing token.
pub const PRE_AUTH_MAX_BYTES: usize = 1024 * 1024;
/// Text frames before authentication are tiny; anything bigger is hostile.
const PRE_AUTH_TEXT_MAX: usize = 64 * 1024;
/// Largest message accepted from an authenticated app (scene dumps, results).
pub const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
/// Connections that have not finished authenticating; extras are closed at once.
pub const MAX_UNAUTHENTICATED: usize = 8;

#[derive(Clone, Debug)]
pub struct LiveConfig {
    /// 0 picks a free port (tests).
    pub port: u16,
    pub token: String,
    pub extra_origins: Vec<String>,
    pub call_timeout: Duration,
}

impl LiveConfig {
    pub fn new(token: impl Into<String>) -> Self {
        Self { port: DEFAULT_PORT, token: token.into(), extra_origins: Vec::new(), call_timeout: Duration::from_secs(120) }
    }
}

struct App {
    id: u64,
    tx: Sender<String>,
}

#[derive(Default)]
struct Shared {
    app: Option<App>,
    next_conn: u64,
    next_call: u64,
    /// call id -> (connection id, where to deliver the answer)
    pending: HashMap<String, (u64, Sender<Result<(Value, bool), String>>)>,
}

/// Counts a connection against the unauthenticated-connection cap until released.
struct Slot(Option<Arc<AtomicUsize>>);
impl Slot {
    fn release(&mut self) {
        if let Some(counter) = self.0.take() {
            counter.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.release();
    }
}

/// A stream whose reads stop at a deadline, so slow or silent peers cannot hold
/// a connection slot, and whose poll interval is adjustable.
struct Guarded {
    stream: TcpStream,
    deadline: Option<Instant>,
    poll: Duration,
}
impl Read for Guarded {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut wait = self.poll;
        if let Some(deadline) = self.deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            wait = wait.min(remaining);
        }
        self.stream.set_read_timeout(Some(wait))?;
        self.stream.read(buf)
    }
}
impl Write for Guarded {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

fn random_nonce() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("the operating system has no secure randomness: {e}"))?;
    Ok(pairing::to_hex(&bytes))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub struct Bridge {
    shared: Arc<Mutex<Shared>>,
    port: u16,
    timeout: Duration,
}

/// Compare without leaking where the first difference is.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

pub fn origin_allowed(origin: Option<&str>, extra: &[String]) -> bool {
    let Some(origin) = origin else { return true };
    if origin == DEFAULT_ORIGIN || extra.iter().any(|o| o == origin) {
        return true;
    }
    ["http://localhost", "http://127.0.0.1"].iter().any(|base| {
        origin.strip_prefix(base).is_some_and(|rest| {
            rest.is_empty() || rest.strip_prefix(':').is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
    })
}

impl Bridge {
    /// Bind 127.0.0.1 only and start accepting in the background.
    pub fn start(config: LiveConfig) -> Result<Self, String> {
        if config.token.len() < MIN_TOKEN_LEN {
            return Err(format!("the pairing token must be at least {MIN_TOKEN_LEN} characters"));
        }
        random_nonce()?; // refuse to start without OS randomness
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, config.port)))
            .map_err(|e| format!("cannot listen on 127.0.0.1:{}: {e}", config.port))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (accept_shared, config_for_accept) = (shared.clone(), Arc::new(config.clone()));
        let unauthenticated = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if unauthenticated.fetch_add(1, Ordering::SeqCst) >= MAX_UNAUTHENTICATED {
                    unauthenticated.fetch_sub(1, Ordering::SeqCst);
                    continue; // dropping the stream closes it
                }
                let slot = Slot(Some(unauthenticated.clone()));
                let (shared, config) = (accept_shared.clone(), config_for_accept.clone());
                std::thread::spawn(move || handle_connection(stream, shared, config, slot));
            }
        });
        Ok(Self { shared, port, timeout: config.call_timeout })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    fn forward(&self, client: &str, name: &str, arguments: Value) -> Result<(Value, bool), String> {
        let (answer_tx, answer_rx) = mpsc::channel();
        let call_id = {
            let mut shared = self.shared.lock().unwrap();
            let Some(app) = &shared.app else { return Err("not connected".into()) };
            let (conn, tx) = (app.id, app.tx.clone());
            shared.next_call += 1;
            let call_id = format!("c{}", shared.next_call);
            let deadline_ms = now_ms() + self.timeout.as_millis() as u64;
            let frame = json!({"type": "call", "id": call_id, "name": name, "arguments": arguments,
                "client": client, "deadline_ms": deadline_ms});
            if tx.send(frame.to_string()).is_err() {
                return Err("not connected".into());
            }
            shared.pending.insert(call_id.clone(), (conn, answer_tx));
            (call_id, tx)
        };
        let (call_id, tx) = call_id;
        match answer_rx.recv_timeout(self.timeout) {
            Ok(answer) => answer,
            Err(RecvTimeoutError::Timeout) => {
                // A result that arrives later finds no pending call and is ignored.
                self.shared.lock().unwrap().pending.remove(&call_id);
                let _ = tx.send(json!({"type": "cancel", "id": call_id}).to_string());
                Err("timeout".into())
            }
            Err(RecvTimeoutError::Disconnected) => Err("disconnected".into()),
        }
    }
}

impl Backend for Bridge {
    fn call(&mut self, client: &str, name: &str, arguments: Value) -> (Value, bool) {
        match self.forward(client, name, arguments) {
            Ok(answer) => answer,
            Err(e) if e == "not connected" => (
                json!({"ok": false, "error": "BenchUp is not connected",
                    "hint": "Open BenchUp, then Settings → Add-ons → Agent connection, and turn on Allow agent control."}),
                false,
            ),
            Err(e) if e == "timeout" => (
                json!({"ok": false, "error": "BenchUp did not answer in time",
                    "hint": "The call was cancelled. Call get_scene to check the current state before retrying."}),
                false,
            ),
            Err(_) => (
                json!({"ok": false, "error": "BenchUp disconnected before answering",
                    "hint": "Reconnect BenchUp (Settings → Add-ons → Agent connection) and check whether the change was applied with get_scene."}),
                false,
            ),
        }
    }
}

fn is_timeout(error: &tungstenite::Error) -> bool {
    matches!(error, tungstenite::Error::Io(e)
        if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

fn handle_connection(stream: TcpStream, shared: Arc<Mutex<Shared>>, config: Arc<LiveConfig>, mut slot: Slot) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(AUTH_TIMEOUT));
    // The whole exchange, handshake included, must finish within AUTH_TIMEOUT.
    let deadline = Instant::now() + AUTH_TIMEOUT;
    let guarded = Guarded { stream, deadline: Some(deadline), poll: AUTH_TIMEOUT };
    let check_origin = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let origin = request.headers().get("origin").map(|v| v.to_str().unwrap_or("\u{0}invalid"));
        if origin_allowed(origin, &config.extra_origins) {
            Ok(response)
        } else {
            eprintln!("benchup-mcp: rejected connection from origin {origin:?}");
            let mut rejection = ErrorResponse::new(Some("origin not allowed".to_owned()));
            *rejection.status_mut() = tungstenite::http::StatusCode::FORBIDDEN;
            Err(rejection)
        }
    };
    let limits = WebSocketConfig::default()
        .max_message_size(Some(PRE_AUTH_MAX_BYTES))
        .max_frame_size(Some(PRE_AUTH_MAX_BYTES));
    let Ok(mut ws) = tungstenite::accept_hdr_with_config(guarded, check_origin, Some(limits)) else { return };
    ws.get_mut().poll = POLL;

    let reject = |ws: &mut tungstenite::WebSocket<Guarded>, message: &str| {
        let _ = ws.send(Message::text(json!({"type": "error", "message": message}).to_string()));
        let _ = ws.close(None);
        let _ = ws.flush();
    };
    let Ok(bridge_nonce) = random_nonce() else { return };
    let mut hello: Option<Value> = None;
    let connection = loop {
        // Checked on every iteration so traffic cannot stretch the window.
        if Instant::now() >= deadline {
            return;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if text.len() > PRE_AUTH_TEXT_MAX {
                    return;
                }
                let frame: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                match (&hello, frame.get("type").and_then(Value::as_str)) {
                    (None, Some("authenticate")) => {
                        let challenge = json!({"type": "challenge", "nonce": bridge_nonce, "server": "benchup-mcp",
                            "version": env!("CARGO_PKG_VERSION"), "protocol": pairing::PROTOCOL});
                        if ws.send(Message::text(challenge.to_string())).is_err() {
                            return;
                        }
                        hello = Some(frame);
                    }
                    (Some(hello), Some("prove")) => {
                        let app_nonce = frame.get("nonce").and_then(Value::as_str).unwrap_or("");
                        let proof = frame.get("proof").and_then(Value::as_str).unwrap_or("");
                        let expected = pairing::app_proof(&config.token, &bridge_nonce, app_nonce);
                        if !pairing::valid_nonce(app_nonce) || !pairing::proofs_match(proof, &expected) {
                            reject(&mut ws, "pairing token does not match");
                            return;
                        }
                        let ready = json!({"type": "ready", "server": "benchup-mcp", "version": env!("CARGO_PKG_VERSION"),
                            "proof": pairing::bridge_proof(&config.token, &bridge_nonce, app_nonce)});
                        if ws.send(Message::text(ready.to_string())).is_err() {
                            return;
                        }
                        break hello.get("connection").and_then(Value::as_str).map(str::to_owned);
                    }
                    _ => {
                        reject(&mut ws, "unexpected message during pairing");
                        return;
                    }
                }
            }
            Ok(Message::Close(_)) => return,
            // Before the hello these are ignored (and never extend the deadline);
            // afterwards anything but the proof is a protocol error.
            Ok(_) if hello.is_some() => {
                reject(&mut ws, "unexpected message during pairing");
                return;
            }
            Ok(_) => {}
            Err(e) if is_timeout(&e) => {}
            Err(_) => return,
        }
    };
    // Authenticated: the unauthenticated slot and the small limits go away.
    slot.release();
    ws.get_mut().deadline = None;
    ws.set_config(|c| {
        c.max_message_size = Some(MAX_MESSAGE_BYTES);
        c.max_frame_size = Some(MAX_MESSAGE_BYTES);
    });
    if let Some(connection) = connection {
        eprintln!("benchup-mcp: BenchUp connected ({connection})");
    }
    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();
    let conn_id = {
        let mut guard = shared.lock().unwrap();
        guard.next_conn += 1;
        let id = guard.next_conn;
        // Replacing drops the older app's sender, which makes its thread close.
        guard.app = Some(App { id, tx });
        id
    };
    let mut alive = true;
    while alive {
        loop {
            match rx.try_recv() {
                Ok(text) => {
                    if ws.send(Message::text(text)).is_err() {
                        alive = false;
                        break;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Replaced by a newer connection.
                    let _ = ws.close(None);
                    let _ = ws.flush();
                    alive = false;
                    break;
                }
            }
        }
        if !alive {
            break;
        }
        match ws.read() {
            Ok(Message::Text(text)) => handle_app_frame(&text, conn_id, &shared),
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(e) if is_timeout(&e) => {}
            Err(_) => break,
        }
    }
    let mut guard = shared.lock().unwrap();
    if guard.app.as_ref().is_some_and(|a| a.id == conn_id) {
        guard.app = None;
    }
    let lost: Vec<String> = guard.pending.iter().filter(|(_, (c, _))| *c == conn_id).map(|(k, _)| k.clone()).collect();
    for key in lost {
        if let Some((_, answer)) = guard.pending.remove(&key) {
            let _ = answer.send(Err("disconnected".into()));
        }
    }
}

fn handle_app_frame(text: &str, conn_id: u64, shared: &Mutex<Shared>) {
    let Ok(frame) = serde_json::from_str::<Value>(text) else { return };
    match frame.get("type").and_then(Value::as_str) {
        Some("result") => {
            let Some(id) = frame.get("id").and_then(Value::as_str) else { return };
            let mut guard = shared.lock().unwrap();
            // Only the connection that was asked may answer.
            if guard.pending.get(id).is_some_and(|(c, _)| *c == conn_id) {
                if let Some((_, answer)) = guard.pending.remove(id) {
                    let ok = frame.get("ok").and_then(Value::as_bool).unwrap_or(false);
                    let _ = answer.send(Ok((frame.get("result").cloned().unwrap_or(Value::Null), ok)));
                }
            }
        }
        Some("status") => eprintln!("benchup-mcp: app status: {}", frame.get("message").and_then(Value::as_str).unwrap_or("")),
        _ => {}
    }
}
