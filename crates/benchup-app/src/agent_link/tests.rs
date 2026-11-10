//! Frame parsing, connection lifecycle, queueing next to MIRA, shared-project
//! rules, persistence and secrecy of the pairing token. No test uses the
//! network except the loopback one (127.0.0.1, port 0), and every test that
//! commits an edit (which autosaves) runs with a temp data directory.
use super::*;
#[cfg(feature = "assistant")]
use crate::app::assistant::{EntryRole, Prompt, client::fake::FakeTransport};
use benchup_tools::pairing;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Run `test` in a child process whose data directory is a temp dir.
fn in_temp_data_dir(name: &str, test: impl FnOnce()) {
    const CHILD: &str = "BENCHUP_AGENT_LINK_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        test();
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("app::agent_link::tests::{name}"), "--nocapture"])
        .env(CHILD, "1")
        .env("BENCHUP_DATA_DIR", directory.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

// ------------------------------------------------------------ fake bridge

#[derive(Default)]
struct Bridge {
    incoming: VecDeque<Result<Value, String>>,
    sent: Vec<Value>,
    connects: Vec<(String, String)>,
}
#[derive(Clone, Default)]
struct FakeBridge(Arc<Mutex<Bridge>>);
struct FakeLink(FakeBridge);
impl Link for FakeLink {
    fn events(&mut self) -> Vec<Result<Value, String>> {
        self.0.0.lock().unwrap().incoming.drain(..).collect()
    }
    fn send(&mut self, message: Value) -> Result<(), String> {
        self.0.0.lock().unwrap().sent.push(message);
        Ok(())
    }
}
impl Connector for FakeBridge {
    fn connect(&mut self, _: &egui::Context, url: &str, token: &str, _: &str) -> Box<dyn Link> {
        self.0.lock().unwrap().connects.push((url.into(), token.into()));
        Box::new(FakeLink(self.clone()))
    }
}
impl FakeBridge {
    fn push(&self, frame: Value) {
        self.0.lock().unwrap().incoming.push_back(Ok(frame));
    }
    fn fail(&self, message: &str) {
        self.0.lock().unwrap().incoming.push_back(Err(message.into()));
    }
    fn call(&self, id: &str, name: &str, arguments: Value) {
        self.push(json!({"type": "call", "id": id, "name": name, "arguments": arguments, "client": "Claude Code"}));
    }
    /// The whole bridge side of the handshake, for a token the bridge knows.
    fn handshake_with(&self, token: &str) {
        self.push(json!({"type": "challenge", "nonce": BRIDGE_NONCE, "server": "benchup-mcp", "version": "1", "protocol": 2}));
        self.push(json!({
            "type": "ready", "server": "benchup-mcp", "version": "1", "protocol": 2,
            "proof": pairing::bridge_proof(token, BRIDGE_NONCE, APP_NONCE),
        }));
    }
    fn ready(&self) {
        self.handshake_with(TOKEN);
    }
    fn sent(&self) -> Vec<Value> {
        self.0.lock().unwrap().sent.clone()
    }
    fn results(&self) -> Vec<Value> {
        self.0.lock().unwrap().sent.iter().filter(|m| m["type"] == "result").cloned().collect()
    }
    fn result(&self, id: &str) -> Value {
        self.results().into_iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("no result for {id}"))
    }
    fn connects(&self) -> usize {
        self.0.lock().unwrap().connects.len()
    }
}

const TOKEN: &str = "11111111-2222-4333-8444-555555555555";
const BRIDGE_NONCE: &str = "abababababababababababababababababababababababababababababababab";
const APP_NONCE: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

fn app_with(bridge: &FakeBridge) -> BenchUpApp {
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.agent_link = AgentLink::with_connector(Box::new(bridge.clone()));
    // Skip the credentials file: tests must not write to a data directory.
    app.agent_link.token = Some(TOKEN.into());
    app.agent_link.token_checked = true;
    app.agent_link.nonce_source = || Ok(APP_NONCE.into());
    app.general_settings.agent_link = Preferences { enabled: true, port: 50123 };
    app
}
fn connected(bridge: &FakeBridge, ctx: &egui::Context) -> BenchUpApp {
    let mut app = app_with(bridge);
    bridge.ready();
    app.poll_agent_link(ctx);
    assert!(app.agent_link.connected());
    app
}
fn create_board(id: &str, name: &str, x: f64) -> Value {
    json!({"type": "call", "id": id, "name": "create_board", "client": "Claude Code",
        "arguments": {"name": name, "origin_mm": [x, 0, 0], "size_mm": [100, 100, 100]}})
}
fn labels(app: &BenchUpApp) -> Vec<String> {
    app.doc.history.labels().map(str::to_owned).collect()
}

// ------------------------------------------------------------ frame parsing

#[test]
fn frames_are_parsed_strictly() {
    assert_eq!(
        parse_frame(&json!({"type": "ready", "server": "benchup-mcp", "proof": "p", "protocol": 2})),
        Frame::Ready { proof: Some("p".into()), protocol: Some(2) }
    );
    assert_eq!(parse_frame(&json!({"type": "ready"})), Frame::Ready { proof: None, protocol: None });
    assert_eq!(
        parse_frame(&json!({"type": "challenge", "nonce": "n", "protocol": 2})),
        Frame::Challenge { nonce: "n".into(), protocol: Some(2) }
    );
    assert_eq!(parse_frame(&json!({"type": "cancel", "id": "7"})), Frame::Cancel { id: "7".into() });
    assert_eq!(parse_frame(&json!({"type": "cancel"})), Frame::Ignored);
    let Frame::Call { deadline_ms, .. } = parse_frame(&json!({"type": "call", "id": "1", "name": "n", "deadline_ms": 1700000000123u64})) else { panic!() };
    assert_eq!(deadline_ms, Some(1_700_000_000_123));
    assert_eq!(parse_frame(&json!({"type": "error", "message": "bad token"})), Frame::Error("bad token".into()));
    // Unknown types, non-objects and missing fields are ignored.
    for ignored in [
        json!({"type": "launch"}),
        json!({"nothing": 1}),
        json!([1, 2]),
        json!("call"),
        json!(null),
        json!({"type": 7}),
        json!({"type": "call", "name": "get_scene"}),
        json!({"type": "call", "id": 5, "name": "get_scene"}),
        json!({"type": "call", "id": "", "name": "get_scene"}),
    ] {
        assert_eq!(parse_frame(&ignored), Frame::Ignored, "{ignored}");
    }
    let Frame::Call { id, client, call, .. } =
        parse_frame(&json!({"type": "call", "id": "7", "name": "get_scene", "client": "  Cl\u{7}aude\n Code  "}))
    else {
        panic!("a call")
    };
    // Missing arguments mean none; control characters never reach the UI.
    assert_eq!((id.as_str(), client.as_str()), ("7", "Claude Code"));
    let call = call.unwrap();
    assert_eq!((call.name.as_str(), &call.arguments), ("get_scene", &json!({})));
    let Frame::Call { call, .. } = parse_frame(&json!({"type": "call", "id": "8", "name": "x", "arguments": null})) else { panic!() };
    assert_eq!(call.unwrap().arguments, json!({}));
    // Arguments that are not an object, or a call with no name, are answered with an error.
    for broken in [
        json!({"type": "call", "id": "9", "name": "get_scene", "arguments": [1]}),
        json!({"type": "call", "id": "9", "name": "get_scene", "arguments": "{}"}),
        json!({"type": "call", "id": "9", "name": "get_scene", "arguments": 3}),
        json!({"type": "call", "id": "9", "arguments": {}}),
        json!({"type": "call", "id": "9", "name": "", "arguments": {}}),
        json!({"type": "call", "id": "9", "name": 4, "arguments": {}}),
    ] {
        let Frame::Call { id, call, .. } = parse_frame(&broken) else { panic!("{broken}") };
        assert_eq!(id, "9");
        assert!(call.is_err(), "{broken}");
    }
    let long = "x".repeat(500);
    let Frame::Call { client, .. } = parse_frame(&json!({"type": "call", "id": "1", "name": "n", "client": long})) else { panic!() };
    assert_eq!(client.chars().count(), MAX_CLIENT_CHARS);
}

#[test]
fn malformed_calls_are_answered_not_dropped_and_calls_before_ready_are_ignored() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = app_with(&bridge);
    bridge.call("early", "get_scene", json!({}));
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.status, Status::Connecting);
    assert!(bridge.results().is_empty() && app.agent_link.queued() == 0, "not trusted before ready");
    bridge.ready();
    bridge.call("bad", "get_scene", json!([1]));
    bridge.call("unknown", "launch_missiles", json!({}));
    bridge.push(json!({"type": "weird"}));
    app.poll_agent_link(&ctx);
    let bad = bridge.result("bad");
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["result"]["error"], "arguments must be a JSON object");
    let unknown = bridge.result("unknown");
    assert_eq!(unknown["ok"], false);
    assert!(unknown["result"]["error"].as_str().unwrap().contains("unknown tool"));
    assert_eq!(bridge.results().len(), 2);
}

// ------------------------------------------------------------ lifecycle

#[test]
fn off_by_default_and_connects_with_the_token_on_localhost_only() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = app_with(&bridge);
    app.general_settings.agent_link.enabled = false;
    app.poll_agent_link(&ctx);
    assert_eq!((app.agent_link.status.clone(), bridge.connects()), (Status::Off, 0));
    app.general_settings.agent_link.enabled = true;
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.status, Status::Connecting);
    assert_eq!(bridge.0.lock().unwrap().connects, vec![("ws://127.0.0.1:50123".to_owned(), TOKEN.to_owned())]);
    bridge.ready();
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.status, Status::Connected { client: None });
    bridge.call("1", "get_scene", json!({}));
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.status, Status::Connected { client: Some("Claude Code".into()) });
    assert_eq!(app.agent_status_text(), Some("Agent connected"));
    assert_eq!(bridge.result("1")["ok"], true);
    // Turning it off closes the link and answers nothing further.
    app.general_settings.agent_link.enabled = false;
    app.poll_agent_link(&ctx);
    assert_eq!((app.agent_link.status.clone(), app.agent_status_text()), (Status::Off, None));
    assert!(app.agent_link.active.is_none());
}

#[test]
fn reconnect_backs_off_from_one_to_ten_seconds_and_resets_after_ready() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = app_with(&bridge);
    let mut now = 0.;
    let mut delays = Vec::new();
    for _ in 0..6 {
        app.agent_link.pump(&ctx, now, &app.general_settings.agent_link.clone());
        assert_eq!(app.agent_link.status, Status::Connecting);
        bridge.fail("connection refused");
        app.agent_link.pump(&ctx, now, &app.general_settings.agent_link.clone());
        assert!(matches!(&app.agent_link.status, Status::Error(m) if m == "connection refused"));
        delays.push(app.agent_link.retry_at - now);
        // Too early: no new attempt.
        let before = bridge.connects();
        app.agent_link.pump(&ctx, now + 0.5, &app.general_settings.agent_link.clone());
        assert_eq!(bridge.connects(), before);
        now = app.agent_link.retry_at;
    }
    assert_eq!(delays, vec![1., 2., 4., 8., 10., 10.]);
    // A successful handshake starts over.
    let prefs = app.general_settings.agent_link.clone();
    app.agent_link.pump(&ctx, now, &prefs);
    bridge.ready();
    app.agent_link.pump(&ctx, now, &prefs);
    assert!(app.agent_link.connected());
    bridge.fail("closed");
    app.agent_link.pump(&ctx, now, &prefs);
    assert_eq!(app.agent_link.retry_at - now, 1.);
}

#[test]
fn bridge_errors_timeouts_and_setting_changes_reconnect() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = app_with(&bridge);
    let prefs = app.general_settings.agent_link.clone();
    app.agent_link.pump(&ctx, 0., &prefs);
    bridge.push(json!({"type": "error", "message": "bad token"}));
    app.agent_link.pump(&ctx, 0., &prefs);
    assert_eq!(app.agent_link.status, Status::Error("The bridge refused the connection: bad token".into()));
    // No `ready` in time.
    app.agent_link.pump(&ctx, 1., &prefs);
    assert_eq!(app.agent_link.status, Status::Connecting);
    app.agent_link.pump(&ctx, 1. + READY_TIMEOUT + 1., &prefs);
    assert!(matches!(app.agent_link.status, Status::Error(_)));
    // A new port or a new token reconnects at once.
    let mut now = 100.;
    app.agent_link.pump(&ctx, now, &prefs);
    bridge.ready();
    app.agent_link.pump(&ctx, now, &prefs);
    assert!(app.agent_link.connected());
    let count = bridge.connects();
    app.general_settings.agent_link.port = 50999;
    let prefs = app.general_settings.agent_link.clone();
    app.agent_link.pump(&ctx, now, &prefs);
    assert_eq!(bridge.connects(), count + 1);
    assert_eq!(bridge.0.lock().unwrap().connects.last().unwrap().0, "ws://127.0.0.1:50999");
    now += 1.;
    bridge.ready();
    app.agent_link.pump(&ctx, now, &prefs);
    app.agent_link.token = Some("22222222-2222-4333-8444-555555555555".into());
    app.agent_link.pump(&ctx, now, &prefs);
    assert_eq!(bridge.connects(), count + 2);
    assert_eq!(bridge.0.lock().unwrap().connects.last().unwrap().1, "22222222-2222-4333-8444-555555555555");
}

#[cfg(feature = "assistant")]
#[test]
fn disabling_answers_waiting_calls_and_a_lost_link_drops_them() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = connected(&bridge, &ctx);
    let fake = FakeTransport::default();
    app.assistant = crate::app::assistant::Assistant::with_transport(Box::new(fake.clone()));
    app.assistant.key = Some("k".into());
    app.assistant.settings.model = "m".into();
    fake.push_text("thinking");
    app.assistant_send(&ctx, Prompt { prompt: "hi".into(), references: Default::default() }).unwrap();
    assert!(app.assistant.busy());
    bridge.push(create_board("a", "A", 0.));
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.queued(), 1);
    // The bridge is told the call is waiting, once.
    app.poll_agent_link(&ctx);
    let statuses = bridge.0.lock().unwrap().sent.iter().filter(|m| m["type"] == "status").count();
    assert_eq!(statuses, 1);
    app.general_settings.agent_link.enabled = false;
    app.poll_agent_link(&ctx);
    assert_eq!(bridge.result("a")["result"]["error"], "Agent control was turned off");
    assert_eq!(app.agent_link.queued(), 0);
    // A lost connection forgets calls nobody can be told about.
    app.general_settings.agent_link.enabled = true;
    app.poll_agent_link(&ctx);
    bridge.ready();
    bridge.push(create_board("b", "B", 0.));
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.queued(), 1);
    bridge.fail("gone");
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.queued(), 0);
}

#[test]
fn calls_that_wait_longer_than_the_bridge_does_are_refused() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = connected(&bridge, &ctx);
    app.agent_link.queue.push_back(Queued {
        id: "slow".into(),
        client: String::new(),
        call: benchup_tools::Call::new("slow", "get_scene", json!({})),
        queued_at: 0.,
        notified: false,
        deadline_ms: u64::MAX,
        epoch: 0,
    });
    app.agent_link.expire(QUEUE_TIMEOUT - 1.);
    assert_eq!(app.agent_link.queued(), 1);
    app.agent_link.expire(QUEUE_TIMEOUT + 1.);
    assert_eq!(app.agent_link.queued(), 0);
    assert_eq!(bridge.result("slow")["result"]["error"], "BenchUp is busy with a MIRA turn, retry");
}

#[cfg(feature = "assistant")]
#[test]
fn a_full_queue_refuses_more_calls() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = connected(&bridge, &ctx);
    app.assistant = crate::app::assistant::Assistant::with_transport(Box::new(FakeTransport::default()));
    app.assistant.key = Some("k".into());
    app.assistant.settings.model = "m".into();
    app.assistant_send(&ctx, Prompt { prompt: "hi".into(), references: Default::default() }).unwrap();
    for n in 0..QUEUE_LIMIT + 2 {
        bridge.call(&format!("c{n}"), "get_scene", json!({}));
    }
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.queued(), QUEUE_LIMIT);
    assert_eq!(bridge.results().len(), 2);
    assert_eq!(bridge.result("c64")["result"]["error"], "Too many agent calls are waiting, retry");
}

// ------------------------------------------------------------ editing

#[test]
fn an_external_call_is_one_undo_step_with_an_agent_activity_card() {
    in_temp_data_dir("an_external_call_is_one_undo_step_with_an_agent_activity_card", || {
        let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
        let mut app = connected(&bridge, &ctx);
        assert!(labels(&app).is_empty());
        bridge.push(create_board("1", "Leg", 0.));
        app.poll_agent_link(&ctx);
        let result = bridge.result("1");
        assert_eq!(result["ok"], true, "{result}");
        let id = result["result"]["created"][0].as_u64().unwrap();
        assert_eq!(result["result"]["document"], json!({"name": app.doc.name, "epoch": app.doc_epoch}));
        assert_eq!(labels(&app), vec!["Agent: create_board (1 new, 0 changed)".to_owned()]);
        assert_eq!(app.selected, BTreeSet::from([id]), "what the agent made is selected");
        // A read adds no step and no card.
        bridge.call("2", "get_scene", json!({}));
        app.poll_agent_link(&ctx);
        assert_eq!(bridge.result("2")["ok"], true);
        assert_eq!(labels(&app).len(), 1);
        #[cfg(feature = "assistant")]
        {
            assert_eq!(app.assistant.transcript.len(), 1);
            // The change card names the client and can be undone while newest.
            let entry = &app.assistant.transcript[0];
            assert_eq!(entry.role, EntryRole::Agent);
            assert_eq!(entry.text, "Claude Code: create_board");
            let changes = entry.changes.as_ref().unwrap();
            assert_eq!(changes.created, vec![id]);
            assert_eq!(changes.position, Some(app.doc.history.position()));
            assert!(crate::app::assistant::panel::turn_undoable(changes, app.doc.history.position(), labels(&app).len()));
        }
        // A failed call changes nothing.
        bridge.call("3", "move", json!({"ids": [9999], "delta_mm": [1, 0, 0]}));
        app.poll_agent_link(&ctx);
        assert_eq!(bridge.result("3")["ok"], false);
        assert_eq!(labels(&app).len(), 1);
        app.undo(&ctx, false);
        assert!(app.doc.model.objects.is_empty(), "undo removes the board in one step");
        assert_eq!(app.doc.history.position(), 0);
    });
}

#[cfg(feature = "assistant")]
#[test]
fn agent_calls_wait_for_a_running_mira_turn_and_then_run_in_order() {
    in_temp_data_dir("agent_calls_wait_for_a_running_mira_turn_and_then_run_in_order", || {
        let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
        let mut app = connected(&bridge, &ctx);
        let fake = FakeTransport::default();
        app.assistant = crate::app::assistant::Assistant::with_transport(Box::new(fake.clone()));
        app.assistant.key = Some("k".into());
        app.assistant.settings.model = "m".into();
        let mira = crate::app::assistant::client::ToolCall::new(
            "m1",
            "create_board",
            &json!({"name": "FromMira", "origin_mm": [0, 0, 0], "size_mm": [50, 50, 50]}),
        );
        fake.push_tool_calls(vec![mira]);
        fake.push_text("Added a board.");
        app.assistant_send(&ctx, Prompt { prompt: "add a board".into(), references: Default::default() }).unwrap();
        bridge.push(create_board("a", "First", 300.));
        bridge.push(create_board("b", "Second", 600.));
        app.poll_agent_link(&ctx);
        assert!(app.assistant.busy());
        assert_eq!(app.agent_link.queued(), 2, "queued, not interleaved");
        assert!(bridge.results().is_empty());
        for _ in 0..20 {
            if !app.assistant.busy() {
                break;
            }
            app.poll_assistant(&ctx);
            app.poll_agent_link(&ctx);
        }
        assert!(!app.assistant.busy());
        app.poll_agent_link(&ctx);
        assert_eq!(app.agent_link.queued(), 0);
        let results = bridge.results();
        assert_eq!(results.iter().map(|r| r["id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert!(results.iter().all(|r| r["ok"] == true));
        let history = labels(&app);
        assert_eq!(history.len(), 3, "{history:?}");
        assert!(history[0].starts_with("MIRA: "), "{history:?}");
        assert!(history[1].starts_with("Agent: create_board") && history[2].starts_with("Agent: create_board"));
        let names: Vec<_> = app.doc.model.objects.values().map(|o| o.name.clone()).collect();
        assert!(names.contains(&"FromMira".to_owned()) && names.contains(&"First".to_owned()) && names.contains(&"Second".to_owned()));
        let first = results[0]["result"]["created"][0].as_u64().unwrap();
        let second = results[1]["result"]["created"][0].as_u64().unwrap();
        assert!(first < second);
        // MIRA's card and both agent cards are in the transcript, in order.
        let roles: Vec<_> = app.assistant.transcript.iter().map(|e| e.role).collect();
        assert_eq!(roles, vec![EntryRole::User, EntryRole::Assistant, EntryRole::Agent, EntryRole::Agent]);
    });
}

fn shared_app(bridge: &FakeBridge, ctx: &egui::Context) -> (BenchUpApp, u64, u64) {
    let mut app = connected(bridge, ctx);
    let mut env = benchup_tools::ToolEnv::default();
    let mut board = |name: &str, x: f64| {
        let call = benchup_tools::Call::new("t", "create_board", json!({"name": name, "origin_mm": [x, 0, 0], "size_mm": [100, 100, 100]}));
        benchup_tools::execute(&mut app.doc.model, &call, &mut env).created[0]
    };
    let (mine, theirs) = (board("Mine", 0.), board("Theirs", 500.));
    app.test_share_with_lease(ctx, &BTreeSet::from([mine]));
    (app, mine, theirs)
}

#[test]
fn the_edit_lease_is_enforced_for_agent_calls() {
    in_temp_data_dir("the_edit_lease_is_enforced_for_agent_calls", || {
        let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
        let (mut app, mine, theirs) = shared_app(&bridge, &ctx);
        bridge.call("t", "move", json!({"ids": [theirs], "delta_mm": [50, 0, 0]}));
        bridge.call("m", "move", json!({"ids": [mine], "delta_mm": [50, 0, 0]}));
        app.poll_agent_link(&ctx);
        let refused = bridge.result("t");
        assert_eq!((refused["ok"].clone(), refused["result"]["error"].clone()), (json!(false), json!("not-leased")));
        assert_eq!(bridge.result("m")["ok"], true);
        assert_eq!(labels(&app).len(), 1, "only the leased move was recorded");
        let x = |id| app.doc.model.selection_bounds(&BTreeSet::from([id])).unwrap().0.x;
        assert!((x(theirs) - 0.5).abs() < 1e-9 && (x(mine) - 0.05).abs() < 1e-9);
    });
}

#[test]
fn a_shared_project_without_edit_access_refuses_agent_edits_but_allows_reads() {
    in_temp_data_dir("a_shared_project_without_edit_access_refuses_agent_edits_but_allows_reads", || {
        let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
        let (mut app, mine, _) = shared_app(&bridge, &ctx);
        app.test_expire_lease();
        bridge.call("w", "move", json!({"ids": [mine], "delta_mm": [50, 0, 0]}));
        bridge.call("r", "get_scene", json!({}));
        app.poll_agent_link(&ctx);
        let refused = bridge.result("w");
        assert_eq!(refused["ok"], false);
        assert_eq!(refused["result"]["error"], "read-only");
        assert_eq!(bridge.result("r")["ok"], true);
        assert!(labels(&app).is_empty());
        #[cfg(feature = "assistant")]
        assert!(app.assistant.transcript.is_empty());
    });
}

#[test]
fn an_edit_refused_at_commit_time_is_rolled_back_with_the_reason() {
    in_temp_data_dir("an_edit_refused_at_commit_time_is_rolled_back_with_the_reason", || {
        let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
        let mut app = connected(&bridge, &ctx);
        // Edit access lapses between the check and the commit.
        let before = app.doc.model.clone();
        app.test_share_with_lease(&ctx, &BTreeSet::new());
        app.test_expire_lease();
        let call = benchup_tools::Call::new("x", "create_board", json!({"name": "A", "origin_mm": [0, 0, 0], "size_mm": [1, 1, 1]}));
        // The tool environment itself is read-only now, so the call is refused.
        let (ok, result) = app.run_agent_call(call, "Claude Code", &ctx);
        assert!(!ok);
        assert_eq!(result["error"], "read-only");
        assert_eq!(app.doc.model, before);
        assert!(labels(&app).is_empty());
    });
}

// ------------------------------------------------------------ settings and secrecy

#[derive(Default)]
struct MemoryStorage(std::collections::BTreeMap<String, String>);
impl eframe::Storage for MemoryStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.into(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
    fn flush(&mut self) {}
}

#[test]
fn settings_default_off_round_trip_and_load_from_older_data() {
    let defaults = Preferences::default();
    assert_eq!((defaults.enabled, defaults.port), (false, 47615));
    // Saved data from before this add-on has neither the section nor its fields.
    let old: crate::app::settings::GeneralSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(old.agent_link, defaults);
    let partial: crate::app::settings::GeneralSettings = serde_json::from_str(r#"{"agent_link":{"port":5000}}"#).unwrap();
    assert_eq!(partial.agent_link, Preferences { enabled: false, port: 5000 });
    let mut zero: crate::app::settings::GeneralSettings = serde_json::from_str(r#"{"agent_link":{"enabled":true,"port":0}}"#).unwrap();
    zero.normalize();
    assert_eq!(zero.agent_link, Preferences { enabled: true, port: DEFAULT_PORT });

    // Saved immediately, without a model edit, and restored.
    let ctx = egui::Context::default();
    let mut storage = MemoryStorage::default();
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.general_settings.agent_link = Preferences { enabled: true, port: 51234 };
    app.general_settings.section = crate::app::settings::Section::AddOns;
    app.persist_inspector_changes(&ctx, Some(&mut storage), false);
    assert!(!app.project.modified);
    let mut restored = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    restored.restore_inspector_preferences(&egui::Context::default(), Some(&storage));
    assert_eq!(restored.general_settings.agent_link, Preferences { enabled: true, port: 51234 });
    assert_eq!(restored.general_settings.section, crate::app::settings::Section::AddOns);
}

#[test]
fn the_pairing_token_never_reaches_preferences_storage_or_the_project() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = connected(&bridge, &ctx);
    app.general_settings.agent_link.enabled = true;
    let mut storage = MemoryStorage::default();
    app.persist_inspector_changes(&ctx, Some(&mut storage), true);
    assert!(!storage.0.is_empty());
    for (key, value) in &storage.0 {
        assert!(!value.contains(TOKEN), "{key} holds the token");
    }
    assert!(!serde_json::to_string(&app.doc).unwrap().contains(TOKEN));
    assert!(!format!("{:?}", app.agent_link.status).contains(TOKEN));
}

#[test]
fn tokens_are_version_4_uuids_and_snippets_carry_the_token_and_port() {
    let (a, b) = (new_token().unwrap(), new_token().unwrap());
    assert_ne!(a, b);
    for token in [&a, &b] {
        let parts: Vec<_> = token.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert!(token.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
        assert!(parts[2].starts_with('4') && "89ab".contains(&parts[3][..1]));
    }
    // The token travels in the environment, never in argv.
    assert_eq!(claude_command("T", 47615), "claude mcp add benchup -e BENCHUP_AGENT_TOKEN=T -- benchup-mcp live");
    assert_eq!(claude_command("T", 5000), "claude mcp add benchup -e BENCHUP_AGENT_TOKEN=T -- benchup-mcp live --port 5000");
    assert!(!claude_command("T", 5000).contains("--token"));
    let value: Value = serde_json::from_str(&mcp_json("T", 47615)).unwrap();
    assert_eq!(
        value,
        json!({"mcpServers": {"benchup": {"command": "benchup-mcp", "args": ["live"], "env": {"BENCHUP_AGENT_TOKEN": "T"}}}})
    );
    let value: Value = serde_json::from_str(&mcp_json("T", 5000)).unwrap();
    assert_eq!(value["mcpServers"]["benchup"]["args"], json!(["live", "--port", "5000"]));
}

#[test]
fn the_token_is_saved_privately_once_and_regenerated_on_request() {
    in_temp_data_dir("the_token_is_saved_privately_once_and_regenerated_on_request", || {
        let mut link = AgentLink::default();
        link.load_token();
        assert!(link.token.is_none(), "viewing never creates a token");
        let first = link.ensure_token().unwrap();
        let dir = crate::storage::data_dir().unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(TOKEN_FILE)).unwrap().trim(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join(TOKEN_FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut again = AgentLink::default();
        assert_eq!(again.ensure_token().unwrap(), first);
        again.regenerate();
        assert_ne!(again.token.as_deref(), Some(first.as_str()));
        assert_eq!(AgentLink::default().ensure_token(), again.token);
    });
}

// ------------------------------------------------------------ real socket

/// `realtime::Socket` speaks plain `ws://` to 127.0.0.1 and the app completes
/// the protocol 2 handshake: an empty ticket, a challenge, a proof in each
/// direction, then a call and its result. The raw token never travels.
#[test]
fn the_real_socket_completes_the_v2_handshake_with_a_loopback_bridge() {
    use tungstenite::Message;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        let mut received = Vec::new();
        let mut read = |socket: &mut tungstenite::WebSocket<std::net::TcpStream>| {
            let Message::Text(text) = socket.read().unwrap() else { panic!("text") };
            received.push(text.to_string());
            serde_json::from_str::<Value>(&text).unwrap()
        };
        let first = read(&mut socket);
        socket
            .send(Message::Text(
                json!({"type": "challenge", "nonce": BRIDGE_NONCE, "server": "benchup-mcp", "version": "t", "protocol": 2}).to_string().into(),
            ))
            .unwrap();
        let prove = read(&mut socket);
        let app_nonce = prove["nonce"].as_str().unwrap().to_owned();
        assert!(pairing::valid_nonce(&app_nonce));
        assert!(pairing::proofs_match(prove["proof"].as_str().unwrap(), &pairing::app_proof(TOKEN, BRIDGE_NONCE, &app_nonce)));
        socket
            .send(Message::Text(
                json!({"type": "ready", "server": "benchup-mcp", "version": "t", "protocol": 2,
                    "proof": pairing::bridge_proof(TOKEN, BRIDGE_NONCE, &app_nonce)})
                .to_string()
                .into(),
            ))
            .unwrap();
        socket
            .send(Message::Text(json!({"type": "call", "id": "x1", "name": "get_scene", "arguments": {}, "client": "Test"}).to_string().into()))
            .unwrap();
        let reply = read(&mut socket);
        (first, reply, received)
    });
    let ctx = egui::Context::default();
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.agent_link.token = Some(TOKEN.into());
    app.agent_link.token_checked = true;
    app.general_settings.agent_link = Preferences { enabled: true, port };
    let mut saw_client = false;
    for _ in 0..250 {
        app.poll_agent_link(&ctx);
        saw_client |= app.agent_link.status == Status::Connected { client: Some("Test".into()) };
        if server.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (first, reply, received) = server.join().unwrap();
    assert_eq!((first["type"].clone(), first["ticket"].clone()), (json!("authenticate"), json!("")));
    assert!(first["connection"].as_str().unwrap().starts_with("benchup-app"));
    assert_eq!((reply["type"].clone(), reply["id"].clone(), reply["ok"].clone()), (json!("result"), json!("x1"), json!(true)));
    assert!(reply["result"].is_object() && reply["result"]["document"]["name"].is_string());
    assert!(received.iter().all(|frame| !frame.contains(TOKEN)), "the raw token was sent: {received:?}");
    assert!(saw_client);
}

/// With MIRA turned off the agent link keeps working, and its activity is not
/// recorded anywhere (there is no transcript to show it in).
#[cfg(feature = "assistant")]
#[test]
fn the_agent_link_works_with_mira_off_and_records_no_transcript() {
    in_temp_data_dir("the_agent_link_works_with_mira_off_and_records_no_transcript", || {
        let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
        let mut app = connected(&bridge, &ctx);
        app.general_settings.assistant_enabled = false;
        bridge.push(create_board("1", "Leg", 0.));
        app.poll_agent_link(&ctx);
        assert_eq!(bridge.result("1")["ok"], true);
        assert_eq!(labels(&app).len(), 1);
        assert!(app.assistant.transcript.is_empty());
        app.undo(&ctx, false);
        assert!(app.doc.model.objects.is_empty());
    });
}

/// Without a token nothing connects, and the reason is shown.
#[test]
fn no_secure_randomness_means_no_token_no_connection_and_a_clear_error() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = app_with(&bridge);
    app.agent_link.token = None;
    app.agent_link.generator = || Err("no randomness".into());
    app.poll_agent_link(&ctx);
    assert_eq!(bridge.connects(), 0);
    assert_eq!(app.agent_link.status, Status::Error("no randomness".into()));
    assert!(app.agent_link.token.is_none());
    // Regenerating cannot replace a good token with nothing.
    app.agent_link.token = Some(TOKEN.into());
    app.agent_link.regenerate();
    assert_eq!(app.agent_link.token.as_deref(), Some(TOKEN));
    assert_eq!(app.agent_link.token_error.as_deref(), Some("no randomness"));
}

// ------------------------------------------------------------ protocol 2

fn waiting_app(bridge: &FakeBridge) -> (BenchUpApp, egui::Context) {
    let ctx = egui::Context::default();
    let mut app = app_with(bridge);
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.status, Status::Connecting);
    (app, ctx)
}

#[test]
fn the_handshake_proves_both_sides_and_never_sends_the_raw_token() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = app_with(&bridge);
    bridge.ready();
    bridge.call("1", "get_scene", json!({}));
    app.poll_agent_link(&ctx);
    assert!(app.agent_link.connected());
    let sent = bridge.sent();
    let prove = sent.iter().find(|m| m["type"] == "prove").expect("a prove frame");
    assert_eq!(prove["nonce"], APP_NONCE);
    assert_eq!(prove["proof"], pairing::app_proof(TOKEN, BRIDGE_NONCE, APP_NONCE));
    assert_eq!(bridge.result("1")["ok"], true);
    for frame in &sent {
        assert!(!frame.to_string().contains(TOKEN), "token in {frame}");
    }
}

#[test]
fn a_bad_bridge_proof_is_an_error_and_no_call_runs() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = app_with(&bridge);
    bridge.handshake_with("a-different-token");
    bridge.push(create_board("1", "A", 0.));
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.status, Status::Error("The program on this port did not prove the pairing token".into()));
    assert!(bridge.results().is_empty() && labels(&app).is_empty() && app.doc.model.objects.is_empty());
    // A ready with no proof at all is no better.
    let bridge = FakeBridge::default();
    let mut app = app_with(&bridge);
    bridge.push(json!({"type": "challenge", "nonce": BRIDGE_NONCE, "protocol": 2}));
    bridge.push(json!({"type": "ready", "protocol": 2}));
    app.poll_agent_link(&ctx);
    assert!(matches!(&app.agent_link.status, Status::Error(m) if m.contains("did not prove")));
    assert!(app.agent_link.active.is_none());
}

#[test]
fn calls_before_a_verified_ready_are_ignored() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let (mut app, ctx2) = waiting_app(&bridge);
    let _ = ctx;
    bridge.push(create_board("a", "A", 0.));
    app.poll_agent_link(&ctx2);
    bridge.push(json!({"type": "challenge", "nonce": BRIDGE_NONCE, "protocol": 2}));
    bridge.push(create_board("b", "B", 0.));
    app.poll_agent_link(&ctx2);
    assert!(!app.agent_link.connected());
    assert!(bridge.results().is_empty() && app.agent_link.queued() == 0 && app.doc.model.objects.is_empty());
}

#[test]
fn a_repeated_or_late_challenge_or_an_unchallenged_ready_disconnects() {
    let ctx = egui::Context::default();
    // Twice.
    let bridge = FakeBridge::default();
    let mut app = app_with(&bridge);
    for _ in 0..2 {
        bridge.push(json!({"type": "challenge", "nonce": BRIDGE_NONCE, "protocol": 2}));
    }
    app.poll_agent_link(&ctx);
    assert!(matches!(app.agent_link.status, Status::Error(_)) && app.agent_link.active.is_none());
    // After ready.
    let bridge = FakeBridge::default();
    let mut app = connected(&bridge, &ctx);
    bridge.push(json!({"type": "challenge", "nonce": BRIDGE_NONCE, "protocol": 2}));
    app.poll_agent_link(&ctx);
    assert!(matches!(app.agent_link.status, Status::Error(_)) && app.agent_link.active.is_none());
    // A bridge that never challenges speaks the old protocol.
    let bridge = FakeBridge::default();
    let mut app = app_with(&bridge);
    bridge.push(json!({"type": "ready", "server": "benchup-mcp", "version": "1"}));
    app.poll_agent_link(&ctx);
    assert!(matches!(&app.agent_link.status, Status::Error(m) if m.contains("Update benchup-mcp")));
    // A malformed nonce.
    let bridge = FakeBridge::default();
    let mut app = app_with(&bridge);
    bridge.push(json!({"type": "challenge", "nonce": "short", "protocol": 2}));
    app.poll_agent_link(&ctx);
    assert!(matches!(app.agent_link.status, Status::Error(_)));
    assert!(bridge.sent().iter().all(|m| m["type"] != "prove"));
}

#[test]
fn a_protocol_mismatch_tells_the_user_to_update_the_bridge() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = app_with(&bridge);
    bridge.push(json!({"type": "challenge", "nonce": BRIDGE_NONCE, "protocol": 3}));
    app.poll_agent_link(&ctx);
    let Status::Error(message) = &app.agent_link.status else { panic!("{:?}", app.agent_link.status) };
    assert!(message.contains("incompatible protocol") && message.contains("Update benchup-mcp"), "{message}");
    assert!(bridge.sent().iter().all(|m| m["type"] != "prove"));
}

#[test]
fn no_randomness_for_the_app_nonce_is_an_error() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = app_with(&bridge);
    app.agent_link.nonce_source = || Err("no randomness".into());
    bridge.ready();
    app.poll_agent_link(&ctx);
    assert_eq!(app.agent_link.status, Status::Error("no randomness".into()));
    assert!(app.agent_link.active.is_none());
    assert!(new_nonce().is_ok_and(|n| pairing::valid_nonce(&n)));
}

#[test]
fn a_call_past_its_deadline_is_dropped_silently_and_a_missing_deadline_still_runs() {
    in_temp_data_dir("a_call_past_its_deadline_is_dropped_silently_and_a_missing_deadline_still_runs", || {
        let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
        let mut app = connected(&bridge, &ctx);
        let now = now_ms();
        let with_deadline = |id: &str, deadline: u64| {
            let mut call = create_board(id, id, 0.);
            call["deadline_ms"] = json!(deadline);
            call
        };
        bridge.push(with_deadline("expired", now.saturating_sub(5)));
        bridge.push(with_deadline("almost", now + 500));
        bridge.push(with_deadline("fine", now + 60_000));
        bridge.call("none", "get_scene", json!({}));
        app.poll_agent_link(&ctx);
        let ids: Vec<_> = bridge.results().iter().map(|r| r["id"].as_str().unwrap().to_owned()).collect();
        assert_eq!(ids, vec!["fine", "none"], "expired calls get no answer");
        assert_eq!(app.doc.model.objects.len(), 1);
    });
}

#[test]
fn cancel_removes_a_queued_call_before_it_runs() {
    let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
    let mut app = connected(&bridge, &ctx);
    bridge.push(create_board("a", "A", 0.));
    bridge.push(json!({"type": "cancel", "id": "a"}));
    bridge.push(json!({"type": "cancel", "id": "unknown"}));
    app.poll_agent_link(&ctx);
    assert!(bridge.results().is_empty());
    assert!(app.doc.model.objects.is_empty());
}

#[test]
fn a_call_queued_for_another_project_is_refused_and_results_name_the_document() {
    in_temp_data_dir("a_call_queued_for_another_project_is_refused_and_results_name_the_document", || {
        let (bridge, ctx) = (FakeBridge::default(), egui::Context::default());
        let mut app = connected(&bridge, &ctx);
        app.agent_link.queue.push_back(Queued {
            id: "old".into(),
            client: String::new(),
            call: benchup_tools::Call::new("old", "create_board", json!({"name": "A", "origin_mm": [0, 0, 0], "size_mm": [1, 1, 1]})),
            queued_at: 0.,
            notified: false,
            deadline_ms: u64::MAX,
            epoch: app.doc_epoch,
        });
        let epoch = app.doc_epoch;
        app.replace_project(benchup_core::Document::default(), None, &ctx);
        assert!(app.doc_epoch > epoch, "replacing the project bumps the epoch");
        app.poll_agent_link(&ctx);
        let refused = bridge.result("old");
        assert_eq!(refused["ok"], false);
        assert_eq!(refused["result"]["error"], "The open project changed before this call ran");
        assert_eq!(refused["result"]["hint"], "Call get_scene to see the project that is open now.");
        assert_eq!(refused["result"]["document"]["epoch"], app.doc_epoch);
        assert!(app.doc.model.objects.is_empty());
        // Refusals made before queueing carry it too.
        bridge.call("bad", "get_scene", json!([1]));
        app.poll_agent_link(&ctx);
        assert_eq!(bridge.result("bad")["result"]["document"]["name"], json!(app.doc.name));
    });
}

#[test]
fn client_names_drop_invisible_and_bidi_characters() {
    assert_eq!(clean_client("  Cl\u{202E}au\u{200B}de\u{2066} Co\u{FEFF}de\u{7}  "), "Claude Code");
    assert_eq!(clean_client("\u{200F}\u{2069}"), "");
    assert_eq!(clean_client("é日本 ok"), "é日本 ok");
    assert_eq!(clean_client(&"y".repeat(100)).chars().count(), MAX_CLIENT_CHARS);
}

// ------------------------------------------------------------ end to end
// The real app side (real socket, real handshake) against the real
// `benchup_mcp::live::Bridge`, driven through the MCP protocol. Loopback only.

const E2E_TOKEN: &str = "e2e-0123456789-abcdef-token";

fn start_bridge(token: &str) -> benchup_mcp::live::Bridge {
    let mut config = benchup_mcp::live::LiveConfig::new(token);
    config.port = 0;
    config.call_timeout = std::time::Duration::from_secs(8);
    benchup_mcp::live::Bridge::start(config).unwrap()
}
fn real_app(port: u16, token: &str) -> BenchUpApp {
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.agent_link.set_token_for_test(token);
    app.general_settings.agent_link = Preferences { enabled: true, port };
    app
}
/// Pump the app until `done`, for at most 10 s.
fn pump_until(app: &mut BenchUpApp, ctx: &egui::Context, mut done: impl FnMut(&mut BenchUpApp) -> bool) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(10) {
        app.poll_agent_link(ctx);
        if done(app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out; status {:?}", app.agent_link.status);
}
/// An MCP session on the bridge that makes one `create_board` call.
fn mcp_create_board(mut bridge: benchup_mcp::live::Bridge) -> std::thread::JoinHandle<Vec<Value>> {
    std::thread::spawn(move || {
        let input = format!(
            "{}\n{}\n",
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": "e2e-agent"}}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "create_board",
                "arguments": {"name": "E2E", "origin_mm": [0, 0, 0], "size_mm": [100, 100, 100]}}}),
        );
        let mut out = Vec::new();
        benchup_mcp::serve(std::io::Cursor::new(input.into_bytes()), &mut out, &mut bridge).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    })
}
fn assert_board_created(replies: &[Value], app: &mut BenchUpApp, ctx: &egui::Context) {
    let reply = replies.iter().find(|r| r["id"] == 2).expect("a reply to the call");
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert!(app.doc.model.objects.values().any(|o| o.name == "E2E"));
    let agent_steps: Vec<_> = labels(app).into_iter().filter(|l| l.starts_with("Agent:")).collect();
    assert_eq!(agent_steps.len(), 1, "{:?}", labels(app));
    assert_eq!(labels(app).len(), 1);
    app.undo(ctx, false);
    assert!(app.doc.model.objects.is_empty(), "undo restores the project");
}

#[test]
fn end_to_end_an_mcp_call_edits_the_open_project_through_the_real_bridge() {
    in_temp_data_dir("end_to_end_an_mcp_call_edits_the_open_project_through_the_real_bridge", || {
        let ctx = egui::Context::default();
        let bridge = start_bridge(E2E_TOKEN);
        let mut app = real_app(bridge.port(), E2E_TOKEN);
        pump_until(&mut app, &ctx, |app| app.agent_link.connected());
        let session = mcp_create_board(bridge);
        pump_until(&mut app, &ctx, |_| session.is_finished());
        assert_board_created(&session.join().unwrap(), &mut app, &ctx);
    });
}

#[cfg(feature = "assistant")]
#[test]
fn end_to_end_a_call_waits_for_a_running_mira_turn() {
    in_temp_data_dir("end_to_end_a_call_waits_for_a_running_mira_turn", || {
        let ctx = egui::Context::default();
        let bridge = start_bridge(E2E_TOKEN);
        let mut app = real_app(bridge.port(), E2E_TOKEN);
        pump_until(&mut app, &ctx, |app| app.agent_link.connected());
        let fake = FakeTransport::default();
        app.assistant = crate::app::assistant::Assistant::with_transport(Box::new(fake.clone()));
        app.assistant.key = Some("k".into());
        app.assistant.settings.model = "m".into();
        fake.push_text("Done.");
        app.assistant_send(&ctx, Prompt { prompt: "hi".into(), references: Default::default() }).unwrap();
        assert!(app.assistant.busy());
        let session = mcp_create_board(bridge);
        pump_until(&mut app, &ctx, |app| app.agent_link.queued() == 1);
        assert!(!session.is_finished() && app.doc.model.objects.is_empty(), "waits for the turn");
        pump_until(&mut app, &ctx, |app| {
            app.poll_assistant(&ctx);
            session.is_finished()
        });
        assert!(!app.assistant.busy());
        let replies = session.join().unwrap();
        let reply = replies.iter().find(|r| r["id"] == 2).unwrap();
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        assert!(app.doc.model.objects.values().any(|o| o.name == "E2E"));
        assert_eq!(labels(&app).iter().filter(|l| l.starts_with("Agent:")).count(), 1);
    });
}

#[test]
fn end_to_end_a_wrong_token_is_refused_and_nothing_is_edited() {
    in_temp_data_dir("end_to_end_a_wrong_token_is_refused_and_nothing_is_edited", || {
        let ctx = egui::Context::default();
        let bridge = start_bridge(E2E_TOKEN);
        let mut app = real_app(bridge.port(), "a-different-token-0123456789");
        pump_until(&mut app, &ctx, |app| matches!(app.agent_link.status, Status::Error(_)));
        assert!(!app.agent_link.connected());
        assert!(matches!(&app.agent_link.status, Status::Error(m) if m.contains("refused") && m.contains("token")), "{:?}", app.agent_link.status);
        let session = mcp_create_board(bridge);
        let start = std::time::Instant::now();
        while !session.is_finished() && start.elapsed() < std::time::Duration::from_secs(10) {
            app.poll_agent_link(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(session.is_finished());
        let replies = session.join().unwrap();
        let reply = replies.iter().find(|r| r["id"] == 2).unwrap();
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        assert!(app.doc.model.objects.is_empty() && labels(&app).is_empty());
        assert!(!app.agent_link.connected());
    });
}
