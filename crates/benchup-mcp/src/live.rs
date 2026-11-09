//! `live` mode: forward tool calls to the running BenchUp app over a
//! loopback-only WebSocket. The app connects to us and authenticates with a
//! shared token; one authenticated app at a time.
use crate::protocol::Backend;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{Message, handshake::server::{ErrorResponse, Request, Response}};

pub const DEFAULT_PORT: u16 = 47615;
pub const DEFAULT_ORIGIN: &str = "https://benchup3d.github.io";
pub const MIN_TOKEN_LEN: usize = 16;
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(25);

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
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, config.port)))
            .map_err(|e| format!("cannot listen on 127.0.0.1:{}: {e}", config.port))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (accept_shared, config_for_accept) = (shared.clone(), Arc::new(config.clone()));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (shared, config) = (accept_shared.clone(), config_for_accept.clone());
                std::thread::spawn(move || handle_connection(stream, shared, config));
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
            let frame = json!({"type": "call", "id": call_id, "name": name, "arguments": arguments, "client": client});
            if tx.send(frame.to_string()).is_err() {
                return Err("not connected".into());
            }
            shared.pending.insert(call_id.clone(), (conn, answer_tx));
            call_id
        };
        match answer_rx.recv_timeout(self.timeout) {
            Ok(answer) => answer,
            Err(RecvTimeoutError::Timeout) => {
                self.shared.lock().unwrap().pending.remove(&call_id);
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
                    "hint": "BenchUp may be busy with a MIRA turn or a dialog; retry."}),
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

fn handle_connection(stream: TcpStream, shared: Arc<Mutex<Shared>>, config: Arc<LiveConfig>) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(AUTH_TIMEOUT));
    let _ = stream.set_write_timeout(Some(AUTH_TIMEOUT));
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
    let Ok(mut ws) = tungstenite::accept_hdr(stream, check_origin) else { return };
    let _ = ws.get_ref().set_read_timeout(Some(POLL));

    // Authenticate within 5 s.
    let deadline = Instant::now() + AUTH_TIMEOUT;
    let first = loop {
        match ws.read() {
            Ok(Message::Text(text)) => break text.to_string(),
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(e) if is_timeout(&e) && Instant::now() < deadline => {}
            Err(_) => return,
        }
    };
    let frame: Value = serde_json::from_str(&first).unwrap_or(Value::Null);
    let authentic = frame.get("type").and_then(Value::as_str) == Some("authenticate")
        && frame.get("ticket").and_then(Value::as_str).is_some_and(|t| constant_time_eq(t.as_bytes(), config.token.as_bytes()));
    if !authentic {
        let _ = ws.send(Message::text(json!({"type": "error", "message": "pairing token does not match"}).to_string()));
        let _ = ws.close(None);
        let _ = ws.flush();
        return;
    }
    if let Some(connection) = frame.get("connection").and_then(Value::as_str) {
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
    let ready = json!({"type": "ready", "server": "benchup-mcp", "version": env!("CARGO_PKG_VERSION")});
    let mut alive = ws.send(Message::text(ready.to_string())).is_ok();
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
