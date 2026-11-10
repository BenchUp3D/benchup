use benchup_core::Document;
use benchup_mcp::{Backend, file_mode::FileBackend, live::{Bridge, LiveConfig}, serve};
use benchup_tools::{files, pairing};
use serde_json::{Value, json};
use std::io::Cursor;
use std::net::TcpStream;
use std::time::Duration;
use tungstenite::{Message, WebSocket, client::IntoClientRequest, http::HeaderValue};

const TOKEN: &str = "0123456789abcdef-test-token";

struct Nop;
impl Backend for Nop {
    fn call(&mut self, _: &str, name: &str, _: Value) -> (Value, bool) {
        (json!({"ok": true, "echo": name}), true)
    }
}

fn run(backend: &mut dyn Backend, lines: &[Value]) -> Vec<Value> {
    run_raw(backend, &lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n"))
}
fn run_raw(backend: &mut dyn Backend, input: &str) -> Vec<Value> {
    let mut out = Vec::new();
    serve(Cursor::new(input.as_bytes().to_vec()), &mut out, backend).unwrap();
    String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}
fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}
fn call(id: u64, name: &str, arguments: Value) -> Value {
    request(id, "tools/call", json!({"name": name, "arguments": arguments}))
}

#[test]
fn initialize_negotiates_version() {
    let out = run(&mut Nop, &[
        request(1, "initialize", json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "x"}})),
        request(2, "initialize", json!({"protocolVersion": "1999-01-01"})),
    ]);
    assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(out[1]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "benchup-mcp");
    assert_eq!(out[0]["result"]["capabilities"]["tools"]["listChanged"], false);
    assert!(out[0]["result"]["instructions"].as_str().unwrap().contains("get_scene"));
}

#[test]
fn notifications_get_no_reply_and_errors_are_coded() {
    let out = run_raw(
        &mut Nop,
        &format!(
            "{}\n{}\n{}\n{}\n{}\n",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "method": "whatever/unknown"}),
            request(7, "nope", json!({})),
            "{not json",
            "[]",
        ),
    );
    assert_eq!(out.len(), 3);
    assert_eq!(out[0]["id"], 7);
    assert_eq!(out[0]["error"]["code"], -32601);
    assert_eq!(out[1]["error"]["code"], -32700);
    assert_eq!(out[1]["id"], Value::Null);
    assert_eq!(out[2]["error"]["code"], -32600);
    assert_eq!(run(&mut Nop, &[request(3, "ping", json!({}))])[0]["result"], json!({}));
}

#[test]
fn tools_list_has_every_tool_with_schema() {
    let out = run(&mut Nop, &[request(1, "tools/list", json!({}))]);
    let tools = out[0]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), benchup_tools::definitions().len());
    for d in benchup_tools::definitions() {
        let tool = tools.iter().find(|t| t["name"] == d.name).unwrap_or_else(|| panic!("missing {}", d.name));
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

#[test]
fn tools_call_wraps_result() {
    let out = run(&mut Nop, &[call(1, "get_scene", json!({}))]);
    let r = &out[0]["result"];
    assert_eq!(r["isError"], false);
    assert_eq!(r["structuredContent"]["echo"], "get_scene");
    let text: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, r["structuredContent"]);
}

// ---------------------------------------------------------------- file mode

fn write_project(dir: &std::path::Path, doc: &Document) -> std::path::PathBuf {
    let path = dir.join("p.bup");
    std::fs::write(&path, files::encode_document(doc).unwrap()).unwrap();
    path
}
fn board_args() -> Value {
    json!({"name": "Top", "origin_mm": [0, 0, 0], "size_mm": [600, 300, 20]})
}

#[test]
fn file_mode_persists_one_undo_step_per_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_project(dir.path(), &Document::default());
    let mut backend = FileBackend::open(&path, false).unwrap();
    let out = run(&mut backend, &[call(1, "create_board", board_args()), call(2, "get_scene", json!({}))]);
    assert_eq!(out[0]["result"]["isError"], false, "{out:?}");
    assert_eq!(out[1]["result"]["isError"], false);
    let doc = files::decode(&std::fs::read(&path).unwrap()).unwrap();
    assert!(!doc.model.objects.is_empty());
    let labels: Vec<_> = doc.history.labels().collect();
    assert_eq!(labels.len(), 1);
    assert!(labels[0].starts_with("Agent:"), "{labels:?}");
}

#[test]
fn file_mode_failed_call_leaves_file_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_project(dir.path(), &Document::default());
    let before = std::fs::read(&path).unwrap();
    let mut backend = FileBackend::open(&path, false).unwrap();
    let out = run(&mut backend, &[
        call(1, "create_board", json!({"name": "Bad", "origin_mm": [0, 0, 0], "size_mm": [-1, 5, 5]})),
        call(2, "no_such_tool", json!({})),
    ]);
    assert_eq!(out[0]["result"]["isError"], true);
    assert_eq!(out[1]["result"]["isError"], true);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn file_mode_save_failure_rolls_back_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    let path = write_project(&sub, &Document::default());
    let mut backend = FileBackend::open(&path, false).unwrap();
    std::fs::remove_dir_all(&sub).unwrap(); // saving now fails
    let out = run(&mut backend, &[call(1, "create_board", board_args())]);
    assert_eq!(out[0]["result"]["isError"], true);
    assert!(backend.document().model.objects.is_empty());
    assert_eq!(backend.document().history.labels().count(), 0);
}

#[test]
fn file_mode_refuses_to_overwrite_a_file_changed_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_project(dir.path(), &Document::default());
    let mut backend = FileBackend::open(&path, false).unwrap();
    let out = run(&mut backend, &[call(1, "create_board", board_args())]);
    assert_eq!(out[0]["result"]["isError"], false);
    // Someone else (BenchUp) saves a different project over the file.
    let mut other = Document::default();
    other.name = "Edited elsewhere".into();
    let theirs = files::encode_document(&other).unwrap();
    std::fs::write(&path, &theirs).unwrap();
    let out = run(&mut backend, &[call(2, "create_board", board_args())]);
    assert_eq!(out[0]["result"]["isError"], true);
    assert!(out[0]["result"]["structuredContent"]["error"].as_str().unwrap().contains("changed on disk"));
    assert_eq!(std::fs::read(&path).unwrap(), theirs, "their file must not be overwritten");
    assert_eq!(backend.document().model.objects.len(), 1, "second board rolled back in memory");
}

#[test]
fn file_mode_refuses_shared_projects_without_flag() {
    let dir = tempfile::tempdir().unwrap();
    let mut value = serde_json::to_value(Document::default()).unwrap();
    value["shared_project"] = json!({"server": "https://example.invalid", "id": "abc", "revision": 1, "server_updated_at": 1});
    let doc: Document = serde_json::from_value(value).unwrap();
    assert!(doc.shared_project.is_some());
    let path = write_project(dir.path(), &doc);
    let error = FileBackend::open(&path, false).err().unwrap();
    assert!(error.contains("--allow-shared"), "{error}");
    assert!(FileBackend::open(&path, true).is_ok());
}

// ---------------------------------------------------------------- live mode

type Ws = WebSocket<TcpStream>;

fn connect(port: u16, origin: Option<&str>) -> Result<Ws, tungstenite::Error> {
    let mut request = format!("ws://127.0.0.1:{port}").into_client_request().unwrap();
    if let Some(origin) = origin {
        request.headers_mut().insert("Origin", HeaderValue::from_str(origin).unwrap());
    }
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    tungstenite::client(request, stream).map(|(ws, _)| ws).map_err(|e| match e {
        tungstenite::HandshakeError::Failure(e) => e,
        tungstenite::HandshakeError::Interrupted(_) => unreachable!(),
    })
}
fn recv(ws: &mut Ws) -> Value {
    loop {
        if let Message::Text(t) = ws.read().unwrap() {
            return serde_json::from_str(&t).unwrap();
        }
    }
}
const APP_NONCE: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
/// Run the v2 handshake; returns the bridge's `ready` (verified) or its error frame.
fn auth(ws: &mut Ws, token: &str) -> Value {
    ws.send(Message::text(json!({"type": "authenticate", "ticket": "", "connection": "test"}).to_string())).unwrap();
    let challenge = recv(ws);
    assert_eq!(challenge["type"], "challenge");
    assert_eq!(challenge["protocol"], 2);
    let nonce = challenge["nonce"].as_str().unwrap().to_owned();
    assert!(pairing::valid_nonce(&nonce));
    let proof = pairing::app_proof(token, &nonce, APP_NONCE);
    ws.send(Message::text(json!({"type": "prove", "nonce": APP_NONCE, "proof": proof}).to_string())).unwrap();
    let reply = recv(ws);
    if reply["type"] == "ready" {
        assert_eq!(reply["proof"], pairing::bridge_proof(token, &nonce, APP_NONCE));
        assert!(!reply.to_string().contains(token), "the raw token is never sent");
    }
    reply
}
fn bridge(timeout_ms: u64) -> Bridge {
    let mut config = LiveConfig::new(TOKEN);
    config.port = 0;
    config.call_timeout = Duration::from_millis(timeout_ms);
    Bridge::start(config).unwrap()
}
fn call_line(id: u64, name: &str) -> String {
    format!("{}\n", call(id, name, json!({"query": "x"})))
}

#[test]
fn live_round_trip() {
    let mut b = bridge(10_000);
    let mut app = connect(b.port(), None).unwrap();
    let ready = auth(&mut app, TOKEN);
    assert_eq!(ready["type"], "ready");
    let init = format!("{}\n{}", request(1, "initialize", json!({"clientInfo": {"name": "claude-code"}})), call_line(2, "find_objects"));
    let handle = std::thread::spawn(move || {
        let mut out = Vec::new();
        serve(Cursor::new(init.into_bytes()), &mut out, &mut b).unwrap();
        String::from_utf8(out).unwrap()
    });
    let frame = recv(&mut app);
    assert_eq!(frame["type"], "call");
    assert_eq!(frame["name"], "find_objects");
    assert_eq!(frame["arguments"], json!({"query": "x"}));
    assert_eq!(frame["client"], "claude-code");
    // An unmatched id is ignored.
    app.send(Message::text(json!({"type": "result", "id": "zzz", "ok": true, "result": {}}).to_string())).unwrap();
    app.send(Message::text(json!({"type": "status", "message": "hi"}).to_string())).unwrap();
    app.send(Message::text(json!({"type": "result", "id": frame["id"], "ok": true, "result": {"ok": true, "found": 3}}).to_string())).unwrap();
    let out = handle.join().unwrap();
    let lines: Vec<Value> = out.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines[1]["result"]["isError"], false);
    assert_eq!(lines[1]["result"]["structuredContent"]["found"], 3);
}

#[test]
fn live_wrong_token_origin_and_accepted_origins() {
    let b = bridge(1000);
    let mut app = connect(b.port(), None).unwrap();
    let reply = auth(&mut app, "wrong-wrong-wrong-wrong");
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["message"], "pairing token does not match");

    for bad in ["https://evil.example", "http://localhost.evil.com", "http://127.0.0.1:80@evil.com", "null"] {
        match connect(b.port(), Some(bad)) {
            Err(tungstenite::Error::Http(r)) => assert_eq!(r.status().as_u16(), 403, "{bad}"),
            other => panic!("{bad} was not rejected: {:?}", other.map(|_| ())),
        }
    }
    for good in ["https://benchup3d.github.io", "http://localhost:8080", "http://127.0.0.1:3000"] {
        let mut ws = connect(b.port(), Some(good)).unwrap_or_else(|e| panic!("{good}: {e}"));
        assert_eq!(auth(&mut ws, TOKEN)["type"], "ready", "{good}");
    }
}

#[test]
fn live_extra_origin_and_short_token() {
    let mut config = LiveConfig::new(TOKEN);
    config.port = 0;
    config.extra_origins = vec!["https://my.example".into()];
    let b = Bridge::start(config).unwrap();
    assert!(connect(b.port(), Some("https://my.example")).is_ok());
    assert!(Bridge::start(LiveConfig::new("short")).is_err());
}

#[test]
fn live_not_connected_and_timeout() {
    let mut b = bridge(300);
    let out = run(&mut b, &[call(1, "get_scene", json!({}))]);
    assert_eq!(out[0]["result"]["isError"], true);
    assert_eq!(out[0]["result"]["structuredContent"]["error"], "BenchUp is not connected");

    let mut app = connect(b.port(), None).unwrap();
    assert_eq!(auth(&mut app, TOKEN)["type"], "ready");
    let out = run(&mut b, &[call(2, "get_scene", json!({}))]);
    assert_eq!(out[0]["result"]["isError"], true);
    assert_eq!(out[0]["result"]["structuredContent"]["error"], "BenchUp did not answer in time");
}

#[test]
fn live_newer_connection_replaces_older_and_disconnect_fails_call() {
    let mut b = bridge(10_000);
    let mut old = connect(b.port(), None).unwrap();
    assert_eq!(auth(&mut old, TOKEN)["type"], "ready");
    let mut new = connect(b.port(), None).unwrap();
    assert_eq!(auth(&mut new, TOKEN)["type"], "ready");
    // The old connection gets closed.
    loop {
        match old.read() {
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    let handle = std::thread::spawn(move || run(&mut b, &[call(1, "get_scene", json!({}))]));
    let frame = recv(&mut new);
    assert_eq!(frame["type"], "call");
    drop(new); // app goes away mid-call
    let out = handle.join().unwrap();
    assert_eq!(out[0]["result"]["isError"], true);
    assert!(out[0]["result"]["structuredContent"]["error"].as_str().unwrap().contains("disconnected"));
}

#[test]
fn live_timeout_sends_cancel_and_ignores_the_late_result() {
    let mut b = bridge(300);
    let mut app = connect(b.port(), None).unwrap();
    assert_eq!(auth(&mut app, TOKEN)["type"], "ready");
    let handle = std::thread::spawn(move || {
        let out = run(&mut b, &[call(1, "get_scene", json!({}))]);
        (out, b)
    });
    let frame = recv(&mut app);
    assert_eq!(frame["type"], "call");
    let deadline = frame["deadline_ms"].as_u64().unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    assert!(deadline > now.saturating_sub(5_000) && deadline < now + 5_000, "{deadline} vs {now}");
    let cancel = recv(&mut app);
    assert_eq!(cancel, json!({"type": "cancel", "id": frame["id"]}));
    let (out, mut b) = handle.join().unwrap();
    let result = &out[0]["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["error"], "BenchUp did not answer in time");
    assert!(result["structuredContent"]["hint"].as_str().unwrap().contains("get_scene"));
    // The late answer is dropped; the next call is not confused by it.
    app.send(Message::text(json!({"type": "result", "id": frame["id"], "ok": true, "result": {"late": true}}).to_string())).unwrap();
    let next = std::thread::spawn(move || run(&mut b, &[call(2, "get_scene", json!({}))]));
    let second = recv(&mut app);
    assert_ne!(second["id"], frame["id"]);
    app.send(Message::text(json!({"type": "result", "id": second["id"], "ok": true, "result": {"ok": true, "fresh": true}}).to_string())).unwrap();
    let out = next.join().unwrap();
    assert_eq!(out[0]["result"]["structuredContent"]["fresh"], true);
}

#[test]
fn live_pairing_rejects_bad_proofs_nonces_and_extra_frames() {
    let b = bridge(1000);
    // Wrong proof.
    let mut ws = connect(b.port(), None).unwrap();
    assert_eq!(auth(&mut ws, "another-token-another-token")["message"], "pairing token does not match");
    // Malformed app nonce.
    let mut ws = connect(b.port(), None).unwrap();
    ws.send(Message::text(json!({"type": "authenticate", "ticket": "", "connection": "t"}).to_string())).unwrap();
    let challenge = recv(&mut ws);
    let proof = pairing::app_proof(TOKEN, challenge["nonce"].as_str().unwrap(), "short");
    ws.send(Message::text(json!({"type": "prove", "nonce": "short", "proof": proof}).to_string())).unwrap();
    assert_eq!(recv(&mut ws)["type"], "error");
    // A ping after the hello is a protocol error; the old v1 flow (token as ticket, no prove) never gets ready.
    let mut ws = connect(b.port(), None).unwrap();
    ws.send(Message::text(json!({"type": "authenticate", "ticket": TOKEN, "connection": "t"}).to_string())).unwrap();
    assert_eq!(recv(&mut ws)["type"], "challenge");
    ws.send(Message::Ping(vec![1].into())).unwrap();
    assert_eq!(recv(&mut ws)["type"], "error");
    // A non-hello first frame is rejected.
    let mut ws = connect(b.port(), None).unwrap();
    ws.send(Message::text(json!({"type": "prove", "nonce": APP_NONCE, "proof": "x"}).to_string())).unwrap();
    assert_eq!(recv(&mut ws)["type"], "error");
}

#[test]
fn live_oversize_pre_auth_frames_are_dropped_but_authenticated_apps_may_send_large_results() {
    let b = bridge(1000);
    for size in [100 * 1024, 2 * 1024 * 1024] {
        let mut ws = connect(b.port(), None).unwrap();
        let _ = ws.send(Message::text("x".repeat(size)));
        let _ = ws.flush();
        loop {
            match ws.read() {
                Ok(Message::Text(t)) => panic!("unexpected reply {t}"),
                Ok(_) => {}
                Err(_) => break, // closed
            }
        }
    }
    // After authentication a multi-megabyte frame is fine and keeps the link alive.
    let mut ws = connect(b.port(), None).unwrap();
    assert_eq!(auth(&mut ws, TOKEN)["type"], "ready");
    ws.send(Message::text(json!({"type": "status", "message": "y".repeat(3 * 1024 * 1024)}).to_string())).unwrap();
    ws.send(Message::text(json!({"type": "status", "message": "after"}).to_string())).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    ws.send(Message::Ping(vec![].into())).unwrap();
    loop {
        if matches!(ws.read().unwrap(), Message::Pong(_)) {
            break;
        }
    }
}

#[test]
fn live_caps_unauthenticated_connections() {
    let b = bridge(1000);
    let idle: Vec<TcpStream> = (0..benchup_mcp::live::MAX_UNAUTHENTICATED)
        .map(|_| TcpStream::connect(("127.0.0.1", b.port())).unwrap())
        .collect();
    std::thread::sleep(Duration::from_millis(300));
    // The ninth is closed straight away: the handshake cannot complete.
    let ninth = connect_result(b.port());
    assert!(ninth.is_err(), "excess connection must be refused");
    drop(idle);
    std::thread::sleep(Duration::from_millis(500));
    let mut ws = connect(b.port(), None).unwrap();
    assert_eq!(auth(&mut ws, TOKEN)["type"], "ready");
}

fn connect_result(port: u16) -> Result<(), String> {
    let request = format!("ws://127.0.0.1:{port}").into_client_request().unwrap();
    let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    tungstenite::client(request, stream).map(|_| ()).map_err(|e| e.to_string())
}

#[test]
fn live_authentication_window_is_not_extended_by_traffic() {
    let b = bridge(1000);
    let mut ws = connect(b.port(), None).unwrap();
    let start = std::time::Instant::now();
    // Keep pinging before any hello; the connection must still end after ~5 s.
    loop {
        if start.elapsed() > Duration::from_secs(9) {
            panic!("pre-auth pings extended the auth window");
        }
        if ws.send(Message::Ping(vec![].into())).is_err() {
            break;
        }
        ws.flush().ok();
        std::thread::sleep(Duration::from_millis(200));
        ws.get_ref().set_read_timeout(Some(Duration::from_millis(1))).ok();
        match ws.read() {
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(start.elapsed() < Duration::from_secs(8));
}
