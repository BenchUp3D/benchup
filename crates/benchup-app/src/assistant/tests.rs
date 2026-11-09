//! Turn loop, app integration, preferences and scene-summary tests. No test
//! touches the network or the real data directory.
use super::*;
use client::{ChatError, ToolCall, fake::FakeTransport};
use serde_json::{Value, json};
use tools::test_support as tools_tests;
use tools_tests::board;

/// Run `test` in a child process whose data directory is a temp dir, because
/// committing an edit autosaves to the data directory.
fn in_temp_data_dir(name: &str, test: impl FnOnce()) {
    const CHILD: &str = "BENCHUP_ASSISTANT_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        test();
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("app::assistant::tests::{name}"), "--nocapture"])
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

fn tool(id: &str, name: &str, args: Value) -> ToolCall {
    ToolCall::new(id, name, &args)
}
fn new_board(id: &str, name: &str, origin: [f64; 3], size: [f64; 3]) -> ToolCall {
    tool(id, "create_board", json!({"name": name, "origin_mm": origin, "size_mm": size}))
}

fn connected_app(fake: &FakeTransport) -> BenchUpApp {
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.assistant = Assistant::with_transport(Box::new(fake.clone()));
    app.assistant.key = Some("test-key".into());
    app.assistant.settings.model = "test-model".into();
    app
}
fn ask(app: &mut BenchUpApp, ctx: &egui::Context, prompt: &str) {
    app.assistant_send(ctx, Prompt { prompt: prompt.into(), references: BTreeSet::new() }).unwrap();
}
fn finish(app: &mut BenchUpApp, ctx: &egui::Context) {
    for _ in 0..100 {
        if !app.assistant.busy() {
            return;
        }
        app.poll_assistant(ctx);
    }
    panic!("the turn never finished");
}

/// get_scene, four boards, a joint, then a closing sentence.
fn script_table(fake: &FakeTransport) {
    fake.push_tool_calls(vec![tool("a", "get_scene", json!({}))]);
    fake.push_tool_calls(vec![
        new_board("b1", "Leg", [0., 0., 0.], [40., 40., 700.]),
        new_board("b2", "Apron", [20., 0., 500.], [300., 40., 80.]),
    ]);
    fake.push_tool_calls(vec![
        new_board("b3", "Top", [-20., -20., 700.], [400., 400., 20.]),
        new_board("b4", "Leg 2", [340., 0., 0.], [40., 40., 700.]),
    ]);
    fake.push_tool_calls(vec![tool("c", "add_joint", json!({"a": 2, "b": 1, "kind": "mortise_tenon"}))]);
    fake.push_text("Built a small table with one joint.");
}

#[test]
fn scripted_turn_applies_every_call_and_reports_changes() {
    let fake = FakeTransport::default();
    script_table(&fake);
    let ctx = egui::Context::default();
    let mut model = Model::default();
    let setup = turn::TurnSetup {
        prompt: "Build a table".into(),
        references: BTreeSet::new(),
        selection: BTreeSet::new(),
        include_selection: true,
        catalog: Default::default(),
        read_only: false,
        lease: None,
        history: Vec::new(),
    };
    let mut running = turn::Turn::start(&ctx, &fake, ("https://x.test/v1", "k", "m"), &model, setup);
    let result = loop {
        let poll = running.poll(&ctx, &fake, &mut model, &turn::Gate::default());
        if let Some(result) = poll.finished {
            break result;
        }
    };
    assert_eq!(result.summary, "Built a small table with one joint.");
    assert!(result.error.is_none() && !result.stopped);
    // Object ids are allocated in order, so the scripted joint (2 -> 1) is real
    // only if the first two boards got those ids; check what actually happened.
    assert_eq!(model.objects.len(), 4);
    assert_eq!(result.created.len(), 4, "{:?}", result.created);
    let before = result.before.expect("turn changed the model");
    assert_eq!(before, Model::default());
    let requests = fake.requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    let first = &requests[0];
    assert_eq!(first["model"], "m");
    assert_eq!(first["tool_choice"], "auto");
    assert_eq!(first["stream"], false);
    assert_eq!(first["tools"].as_array().unwrap().len(), tools::definitions().len());
    assert_eq!(first["messages"][0]["role"], "system");
    assert!(first["messages"][1]["content"].as_str().unwrap().contains("\"units\":\"mm\""));
    assert_eq!(first["messages"].as_array().unwrap().last().unwrap()["content"], "Build a table");
    // The next request carries the assistant tool call and its result.
    let second = requests[1]["messages"].as_array().unwrap();
    let n = second.len();
    assert_eq!(second[n - 2]["role"], "assistant");
    assert_eq!(second[n - 2]["tool_calls"][0]["id"], "a");
    assert_eq!(second[n - 1]["role"], "tool");
    assert_eq!(second[n - 1]["tool_call_id"], "a");
    assert_eq!(serde_json::from_str::<Value>(second[n - 1]["content"].as_str().unwrap()).unwrap()["ok"], true);
}

#[test]
fn a_turn_is_exactly_one_undo_step() {
    in_temp_data_dir("a_turn_is_exactly_one_undo_step", || {
        let fake = FakeTransport::default();
        // Ids are allocated deterministically, so replay the boards on a scratch
        // model to learn which ids the scripted joint should name.
        let (mut scratch, mut env) = (Model::default(), tools::ToolEnv::default());
        let leg = board(&mut scratch, &mut env, "Leg", [0., 0., 0.], [40., 40., 700.]);
        let apron = board(&mut scratch, &mut env, "Apron", [20., 0., 500.], [300., 40., 80.]);
        fake.push_tool_calls(vec![tool("a", "get_scene", json!({}))]);
        fake.push_tool_calls(vec![
            new_board("b1", "Leg", [0., 0., 0.], [40., 40., 700.]),
            new_board("b2", "Apron", [20., 0., 500.], [300., 40., 80.]),
        ]);
        fake.push_tool_calls(vec![
            new_board("b3", "Top", [-20., -20., 700.], [400., 400., 20.]),
            new_board("b4", "Leg 2", [340., 0., 0.], [40., 40., 700.]),
        ]);
        fake.push_tool_calls(vec![tool("c", "add_joint", json!({"a": apron, "b": leg, "kind": "mortise_tenon"}))]);
        fake.push_text("Built a table.");
        let ctx = egui::Context::default();
        let mut app = connected_app(&fake);
        let pre_turn = app.doc.model.clone();
        ask(&mut app, &ctx, "Build a table");
        finish(&mut app, &ctx);

        assert_eq!(app.doc.model.objects.len(), 4);
        assert_eq!(app.doc.model.connections.len(), 1);
        let labels: Vec<_> = app.doc.history.labels().map(str::to_owned).collect();
        assert_eq!(labels, ["MIRA: Built a table."], "one history entry for the whole turn");
        assert_eq!(app.doc.history.position(), 1);
        assert!(!app.selected.is_empty(), "created parts are selected");
        let entry = app.assistant.transcript.last().unwrap();
        assert_eq!(entry.text, "Built a table.");
        assert_eq!(entry.changes.as_ref().unwrap().created.len(), 4);
        assert_eq!(entry.changes.as_ref().unwrap().joints.len(), 1);

        app.undo(&ctx, false);
        assert!(app.doc.model.same_scene(&pre_turn), "undo restores the pre-turn model");
        assert!(app.doc.model.objects.is_empty() && app.doc.model.connections.is_empty());
        app.undo(&ctx, true);
        assert_eq!(app.doc.model.objects.len(), 4);
        assert_eq!(app.doc.model.connections.len(), 1);
    });
}

#[test]
fn stopping_mid_turn_commits_one_step() {
    in_temp_data_dir("stopping_mid_turn_commits_one_step", || {
        let fake = FakeTransport::default();
        fake.push_tool_calls(vec![new_board("b1", "Shelf", [0., 0., 0.], [600., 200., 19.])]);
        fake.push_text("never delivered");
        let ctx = egui::Context::default();
        let mut app = connected_app(&fake);
        ask(&mut app, &ctx, "Add a shelf");
        app.poll_assistant(&ctx);
        assert!(app.assistant.busy(), "waiting for the second reply");
        assert_eq!(app.doc.model.objects.len(), 1);
        app.assistant_stop(&ctx);
        assert!(!app.assistant.busy());
        let labels: Vec<_> = app.doc.history.labels().map(str::to_owned).collect();
        assert_eq!(labels, ["MIRA (stopped)"]);
        assert_eq!(app.doc.model.objects.len(), 1);
        app.poll_assistant(&ctx);
        assert_eq!(app.doc.history.labels().count(), 1, "late replies are ignored");
        // Stopping an idle assistant does nothing.
        app.assistant_stop(&ctx);
        assert_eq!(app.doc.history.labels().count(), 1);
    });
}

#[test]
fn failed_requests_keep_partial_work_and_report_the_error() {
    in_temp_data_dir("failed_requests_keep_partial_work_and_report_the_error", || {
        let fake = FakeTransport::default();
        fake.push_tool_calls(vec![new_board("b1", "Shelf", [0., 0., 0.], [600., 200., 19.])]);
        fake.replies.lock().unwrap().push_back(Err(ChatError::ToolsUnsupported));
        let ctx = egui::Context::default();
        let mut app = connected_app(&fake);
        ask(&mut app, &ctx, "Add a shelf");
        finish(&mut app, &ctx);
        assert_eq!(app.doc.history.labels().count(), 1);
        let last = app.assistant.transcript.last().unwrap();
        assert_eq!(last.role, EntryRole::Error);
        assert!(last.text.contains("doesn't support tool calls"));

        // No edits: no history entry.
        fake.replies.lock().unwrap().push_back(Err(ChatError::InvalidKey));
        ask(&mut app, &ctx, "Again");
        finish(&mut app, &ctx);
        assert_eq!(app.doc.history.labels().count(), 1);
        assert_eq!(app.assistant.transcript.last().unwrap().text, "Invalid API key");
    });
}

#[test]
fn read_only_turns_change_nothing() {
    let fake = FakeTransport::default();
    fake.push_tool_calls(vec![new_board("b1", "Shelf", [0., 0., 0.], [600., 200., 19.])]);
    fake.push_text("I could not edit.");
    let ctx = egui::Context::default();
    let mut model = Model::default();
    let setup = turn::TurnSetup {
        prompt: "Add a shelf".into(),
        references: BTreeSet::new(),
        selection: BTreeSet::new(),
        include_selection: false,
        catalog: Default::default(),
        read_only: true,
        lease: None,
        history: Vec::new(),
    };
    let mut running = turn::Turn::start(&ctx, &fake, ("https://x.test/v1", "k", "m"), &model, setup);
    let result = loop {
        if let Some(result) = running.poll(&ctx, &fake, &mut model, &turn::Gate::default()).finished {
            break result;
        }
    };
    assert!(result.before.is_none() && model.objects.is_empty());
    let requests = fake.requests.lock().unwrap();
    let tool_message = requests[1]["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_owned();
    assert!(tool_message.contains("read-only"), "{tool_message}");
}

#[test]
fn fenced_json_tool_calls_run_and_rounds_are_capped() {
    let fake = FakeTransport::default();
    for _ in 0..turn::MAX_ROUNDS + 3 {
        fake.push_text("```json\n{\"name\": \"get_scene\", \"arguments\": {}}\n```");
    }
    let ctx = egui::Context::default();
    let mut model = Model::default();
    let setup = turn::TurnSetup {
        prompt: "Loop".into(),
        references: BTreeSet::new(),
        selection: BTreeSet::new(),
        include_selection: false,
        catalog: Default::default(),
        read_only: false,
        lease: None,
        history: Vec::new(),
    };
    let mut running = turn::Turn::start(&ctx, &fake, ("https://x.test/v1", "k", "m"), &model, setup);
    let result = loop {
        if let Some(result) = running.poll(&ctx, &fake, &mut model, &turn::Gate::default()).finished {
            break result;
        }
    };
    assert_eq!(fake.requests.lock().unwrap().len(), turn::MAX_ROUNDS);
    assert!(result.error.unwrap().contains("12 steps"));
    assert_eq!(running.activity.len(), turn::MAX_ROUNDS);
}

#[test]
fn send_validates_before_starting() {
    let fake = FakeTransport::default();
    let ctx = egui::Context::default();
    let mut app = connected_app(&fake);
    let prompt = |text: &str| Prompt { prompt: text.into(), references: BTreeSet::new() };
    assert!(app.assistant_send(&ctx, prompt("   ")).is_err());
    app.assistant.settings.base_url = "http://example.com/v1".into();
    assert!(app.assistant_send(&ctx, prompt("hi")).unwrap_err().contains("https"));
    app.assistant.settings.base_url = client::DEFAULT_BASE_URL.into();
    app.assistant.key = None;
    assert!(app.assistant_send(&ctx, prompt("hi")).unwrap_err().contains("Connect"));
    app.assistant.key = Some("k".into());
    app.assistant.settings.model.clear();
    assert!(app.assistant_send(&ctx, prompt("hi")).unwrap_err().contains("model"));
    assert!(fake.requests.lock().unwrap().is_empty(), "nothing is sent when validation fails");
    app.assistant.settings.model = "m".into();
    fake.push_text("ok");
    app.assistant_send(&ctx, prompt("hi")).unwrap();
    assert!(app.assistant_send(&ctx, prompt("again")).unwrap_err().contains("still working"));
    // Earlier turns are replayed as plain text in the next request.
    finish(&mut app, &ctx);
    fake.push_text("fine");
    app.assistant_send(&ctx, prompt("second")).unwrap();
    let requests = fake.requests.lock().unwrap();
    let roles: Vec<_> = requests[1]["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap().to_owned()).collect();
    assert_eq!(roles, ["system", "system", "user", "assistant", "user"]);
}

#[test]
fn connecting_saves_a_valid_key_and_rejects_an_invalid_one() {
    in_temp_data_dir("connecting_saves_a_valid_key_and_rejects_an_invalid_one", || {
        let fake = FakeTransport::default();
        let ctx = egui::Context::default();
        let mut assistant = Assistant::with_transport(Box::new(fake.clone()));
        *fake.models.lock().unwrap() = Some(Err(ChatError::InvalidKey));
        assistant.connect(&ctx, "bad-key");
        assert_eq!(assistant.connection, Connection::Connecting);
        let mut model = Model::default();
        assistant.poll(&ctx, &mut model, &turn::Gate::default());
        assert_eq!(assistant.connection, Connection::Failed("Invalid API key".into()));
        assert!(credentials::load(&assistant.settings.base_url).is_none(), "a rejected key is not saved");

        *fake.models.lock().unwrap() = Some(Ok(vec!["et:auto".into(), "Qwen3.8-27B".into()]));
        assistant.connect(&ctx, "  good-key ");
        assistant.poll(&ctx, &mut model, &turn::Gate::default());
        assert_eq!(assistant.connection, Connection::Connected);
        assert_eq!(assistant.settings.model, "Qwen3.8-27B", "prefers the tool-capable default");
        assert_eq!(credentials::load(&assistant.settings.base_url).as_deref(), Some("good-key"));

        let mut restarted = Assistant::with_transport(Box::new(fake.clone()));
        assert!(restarted.key.is_none());
        restarted.load_key();
        assert_eq!(restarted.key.as_deref(), Some("good-key"));
        restarted.forget_key().unwrap();
        assert!(credentials::load(&restarted.settings.base_url).is_none());
    });
}

// ------------------------------------------------------------ preferences

#[derive(Default)]
struct Storage(std::collections::BTreeMap<String, String>);
impl eframe::Storage for Storage {
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
fn settings_persist_without_a_model_edit_and_the_key_never_does() {
    let ctx = egui::Context::default();
    let mut storage = Storage::default();
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.assistant.settings = Preferences {
        base_url: "https://llm.example/v1".into(),
        model: "my-model".into(),
        include_selection: false,
        ..Preferences::default()
    };
    app.assistant.key = Some("sk-secret-do-not-store".into());
    app.persist_inspector_changes(&ctx, Some(&mut storage), false);
    assert!(!app.project.modified, "no model edit is needed");
    assert!(!storage.0.is_empty());
    for value in storage.0.values() {
        assert!(!value.contains("sk-secret-do-not-store"), "the key must not be stored in preferences");
    }
    let mut restored = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    restored.restore_inspector_preferences(&ctx, Some(&storage));
    assert_eq!(restored.assistant.settings, app.assistant.settings);
    assert!(restored.assistant.key.is_none());
}

#[test]
fn old_preferences_without_assistant_fields_load_with_defaults() {
    use super::super::sidebar::InspectorPreferences;
    let old: InspectorPreferences = serde_json::from_str(r#"{"visible": false, "width": 412.0}"#).unwrap();
    let saved = serde_json::to_value(&old).unwrap();
    assert_eq!(
        saved["assistant"],
        json!({
            "base_url": client::DEFAULT_BASE_URL,
            "model": "",
            "include_selection": true,
            "privacy_dismissed": false,
            "window": {"detached": false, "position": null, "size": null},
        })
    );
    let partial: Preferences = serde_json::from_str(r#"{"model": "m"}"#).unwrap();
    assert_eq!(partial.base_url, client::DEFAULT_BASE_URL);
    assert!(partial.include_selection);
    // A tampered saved URL falls back to the default instead of sending the key there.
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.assistant.restore(&Preferences { base_url: "http://evil.example".into(), model: " m ".into(), include_selection: true, ..Preferences::default() });
    assert_eq!(app.assistant.settings.base_url, client::DEFAULT_BASE_URL);
    assert_eq!(app.assistant.settings.model, "m");
}

// ------------------------------------------------------------ scene summary

#[test]
fn scene_summary_reports_millimetres_and_truncates_with_a_hint() {
    let (mut model, mut env) = (Model::default(), tools::ToolEnv::default());
    let mut ids = Vec::new();
    for i in 0..40 {
        ids.push(board(&mut model, &mut env, &format!("Part {i}"), [i as f64 * 100., 0., 0.], [50., 50., 18.]));
    }
    let full = scene::summary(&model, &BTreeSet::new(), &Default::default(), scene::SCENE_BUDGET);
    assert!(full.get("omitted").is_none());
    assert_eq!(full["objects"].as_array().unwrap().len(), 40);
    assert_eq!(full["objects"][1]["bounds_mm"]["min"], json!([100.0, 0.0, 0.0]));
    assert_eq!(full["objects"][1]["size_mm"], json!([50.0, 50.0, 18.0]));

    let last = *ids.last().unwrap();
    let small = scene::summary(&model, &BTreeSet::from([last]), &Default::default(), 4000);
    let shown = small["objects"].as_array().unwrap().len();
    assert!(shown > 0 && shown < 40, "{shown}");
    assert_eq!(small["omitted"], 40 - shown);
    assert!(small["hint"].as_str().unwrap().contains("find_objects"));
    assert!(small.to_string().len() <= 4000, "{}", small.to_string().len());
    assert_eq!(small["objects"][0]["id"], last, "the selection is listed even when the scene is cut");
    // Omitted objects stay reachable.
    let found = tools_tests::run(&mut model, &mut env, "find_objects", json!({"query": "part 39"}));
    assert_eq!(found.result["total"], 1);
}

#[test]
fn references_describe_entities_by_id_even_after_a_rename() {
    let (mut model, mut env) = (Model::default(), tools::ToolEnv::default());
    let id = board(&mut model, &mut env, "Left leg", [0., 0., 0.], [40., 40., 700.]);
    let first = scene::references(&model, &BTreeSet::from([id, 424242]), 100_000);
    assert_eq!(first[id.to_string()]["name"], "Left leg");
    assert_eq!(first[id.to_string()]["faces"].as_array().unwrap().len(), 6);
    assert_eq!(first["424242"]["kind"], "missing");
    tools_tests::run(&mut model, &mut env, "rename", json!({"id": id, "name": "Front leg"}));
    assert_eq!(scene::references(&model, &BTreeSet::from([id]), 100_000)[id.to_string()]["name"], "Front leg");
}

// ------------------------------------------------------------ panel behaviour

use crate::test_ui::ContextExt as _;
use panel::Chip;

#[test]
fn window_privacy_and_selection_preferences_persist_without_a_model_edit() {
    let ctx = egui::Context::default();
    let mut storage = Storage::default();
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.assistant.settings.include_selection = false;
    app.assistant.settings.privacy_dismissed = true;
    app.assistant.settings.model = "qwen".into();
    app.assistant.settings.window = WindowPreferences {
        detached: true,
        position: Some([1210., 64.]),
        size: Some([480., 700.]),
    };
    app.persist_inspector_changes(&ctx, Some(&mut storage), false);
    assert!(!app.project.modified, "saved without a model edit");
    let mut restored = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    restored.restore_inspector_preferences(&ctx, Some(&storage));
    assert_eq!(restored.assistant.settings, app.assistant.settings);
    assert!(restored.assistant.settings.window.detached);
    assert!(!restored.project.modified);
    // Latching back is saved the same way.
    app.assistant_set_detached(&ctx, false);
    app.persist_inspector_changes(&ctx, Some(&mut storage), false);
    restored.restore_inspector_preferences(&ctx, Some(&storage));
    assert!(!restored.assistant.settings.window.detached);
    assert_eq!(restored.assistant.settings.window.position, Some([1210., 64.]), "geometry is kept for next time");
}

#[test]
fn saved_window_geometry_is_sanitized_and_old_saves_default() {
    let partial: Preferences = serde_json::from_str(r#"{"window": {"detached": true}}"#).unwrap();
    assert!(partial.window.detached && partial.window.position.is_none() && !partial.privacy_dismissed);
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.assistant.restore(&Preferences {
        window: WindowPreferences {
            detached: true,
            position: Some([f32::NAN, 0.]),
            size: Some([10., 1e9]),
        },
        ..Preferences::default()
    });
    let window = &app.assistant.settings.window;
    assert_eq!(window.position, None);
    assert_eq!(window.size, Some([window::MIN_SIZE[0], 8192.]));
}

#[test]
fn offscreen_positions_are_rejected_near_main_window_is_used() {
    let main = egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(1440., 900.));
    let monitor = Some(egui::vec2(1440., 900.));
    let size = [440., 640.];
    assert!(window::position_usable([200., 100.], size, Some(main), monitor));
    assert!(window::position_usable([-1500., 100.], size, Some(main), monitor), "a monitor to the left");
    assert!(!window::position_usable([f32::NAN, 0.], size, Some(main), monitor));
    assert!(!window::position_usable([90_000., 0.], size, Some(main), monitor));
    assert!(!window::position_usable([20_000., 0.], size, Some(main), monitor), "an unplugged far monitor");
    let near = window::default_position(Some(main), size).unwrap();
    assert!(main.contains(near));
}

#[test]
fn detached_panel_state_follows_latch_calls_and_is_debounced() {
    let ctx = egui::Context::default();
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.inspector = false;
    app.assistant_set_detached(&ctx, true);
    assert!(app.assistant.settings.window.detached);
    assert!(app.assistant_visible(&ctx), "a detached panel is always visible");
    app.assistant_set_detached(&ctx, false);
    assert!(app.inspector && app.assistant_visible(&ctx), "latching back reopens the section");

    // Inside the canvas (the web fallback) the window reports its rectangle and
    // closing it latches the panel back. Geometry is saved only once it settles.
    app.assistant_set_detached(&ctx, true);
    let frame = |app: &mut BenchUpApp, time: f64| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440., 900.))),
            time: Some(time),
            ..Default::default()
        };
        let _ = ctx.run_test_ui(input, |ui| app.assistant_window(ui.ctx()));
    };
    frame(&mut app, 1.0);
    assert_eq!(app.assistant.settings.window.position, None, "not saved while it may still be moving");
    frame(&mut app, 1.2);
    assert_eq!(app.assistant.settings.window.position, None);
    frame(&mut app, 2.0);
    assert!(app.assistant.settings.window.position.is_some() && app.assistant.settings.window.size.is_some());
}

#[test]
fn chips_rewrite_the_prompt_with_ids_and_collect_references() {
    let chips = [
        Chip { label: "Left leg".into(), id: 7 },
        Chip { label: "Leg".into(), id: 3 },
        Chip { label: "Top".into(), id: 9 },
    ];
    let (text, refs) = panel::rewrite_prompt("make @Left leg thicker, then @Leg and also @Legs", &chips);
    assert_eq!(
        text,
        "make @Left leg [#7] thicker, then @Leg [#3] and also @Legs\n\nAlso referenced: @Top [#9]"
    );
    assert_eq!(refs, BTreeSet::from([3, 7, 9]));
    // Plain text with no chips passes through untouched, including emails.
    let (plain, none) = panel::rewrite_prompt("mail me@example.com", &[]);
    assert_eq!(plain, "mail me@example.com");
    assert!(none.is_empty());
}

#[test]
fn at_mentions_are_detected_before_the_caret() {
    assert_eq!(panel::mention_query("add @Le", 7), Some((4, "Le".into())));
    assert_eq!(panel::mention_query("@", 1), Some((0, String::new())));
    assert_eq!(panel::mention_query("a@b", 3), None, "an email is not a mention");
    assert_eq!(panel::mention_query("no mention", 10), None);
    assert_eq!(panel::mention_query("@x\nnext line", 12), None, "a mention ends at the line");
}

#[test]
fn a_renamed_part_still_resolves_by_id() {
    let fake = FakeTransport::default();
    let mut app = connected_app(&fake);
    let mut env = tools::ToolEnv::default();
    let id = board(&mut app.doc.model, &mut env, "Old name", [0., 0., 0.], [40., 40., 700.]);
    app.assistant_add_chips([id]);
    app.assistant.ui.text = "thicken @Old name please".into();
    let (first, refs) = app.assistant_compose();
    assert_eq!(first, format!("thicken @Old name [#{id}] please"));
    assert_eq!(refs, BTreeSet::from([id]));
    app.doc.model.objects.get_mut(&id).unwrap().name = "New name".into();
    let (second, refs) = app.assistant_compose();
    assert_eq!(second, first, "the id, not the label, identifies the part");
    assert_eq!(scene::references(&app.doc.model, &refs, 100_000)[id.to_string()]["name"], "New name");
    // A deleted part drops out of the references instead of confusing the model.
    app.doc.model.objects.remove(&id);
    assert!(app.assistant_compose().1.is_empty());
}

#[test]
fn submitting_sends_rewritten_text_and_clears_the_composer() {
    in_temp_data_dir("submitting_sends_rewritten_text_and_clears_the_composer", || {
        let fake = FakeTransport::default();
        fake.push_text("Fine.");
        let ctx = egui::Context::default();
        let mut app = connected_app(&fake);
        let mut env = tools::ToolEnv::default();
        let id = board(&mut app.doc.model, &mut env, "Left leg", [0., 0., 0.], [40., 40., 700.]);
        app.assistant_add_chips([id]);
        app.assistant.ui.text = "move @Left leg".into();
        app.assistant_submit(&ctx);
        assert!(app.assistant.busy());
        assert!(app.assistant.ui.text.is_empty() && app.assistant.ui.chips.is_empty());
        assert_eq!(app.assistant.ui.last_prompt, "move @Left leg", "Up recalls what you typed");
        finish(&mut app, &ctx);
        let sent = fake.requests.lock().unwrap()[0].to_string();
        assert!(sent.contains(&format!("move @Left leg [#{id}]")), "{sent}");
    });
}

#[test]
fn user_edits_are_refused_while_a_turn_is_pending_and_allowed_after() {
    in_temp_data_dir("user_edits_are_refused_while_a_turn_is_pending_and_allowed_after", || {
        let fake = FakeTransport::default();
        fake.push_text("Nothing to change.");
        let ctx = egui::Context::default();
        let mut app = connected_app(&fake);
        let mut env = tools::ToolEnv::default();
        let id = board(&mut app.doc.model, &mut env, "Shelf", [0., 0., 0.], [600., 200., 19.]);
        ask(&mut app, &ctx, "look at it");
        assert!(app.assistant.busy(), "the scripted reply has not been polled yet");

        let rename = |app: &mut BenchUpApp, name: &str| {
            let before = app.doc.model.clone();
            app.doc.model.objects.get_mut(&id).unwrap().name = name.into();
            app.finish_edit("Rename", before, &ctx);
        };
        rename(&mut app, "Sneaky");
        assert_eq!(app.doc.model.objects[&id].name, "Shelf", "the edit was rolled back");
        assert_eq!(app.doc.history.labels().count(), 0);
        assert!(app.notice.is_some(), "the user is told why");
        app.undo(&ctx, false);
        app.navigate_history(&ctx, 0);
        assert_eq!(app.doc.history.labels().count(), 0);

        finish(&mut app, &ctx);
        rename(&mut app, "Allowed");
        assert_eq!(app.doc.model.objects[&id].name, "Allowed");
        assert_eq!(app.doc.history.labels().collect::<Vec<_>>(), ["Rename"]);
    });
}

#[test]
fn stopping_releases_the_gate_and_replacing_the_project_stops_the_turn() {
    in_temp_data_dir("stopping_releases_the_gate_and_replacing_the_project_stops_the_turn", || {
        let fake = FakeTransport::default();
        fake.push_text("one");
        fake.push_text("two");
        let ctx = egui::Context::default();
        let mut app = connected_app(&fake);
        ask(&mut app, &ctx, "first");
        app.assistant_stop(&ctx);
        assert!(!app.assistant.busy());
        ask(&mut app, &ctx, "second");
        assert!(app.assistant.busy());
        app.new_model(&ctx);
        assert!(!app.assistant.busy(), "a new project cannot be edited by a stale turn");
    });
}

#[test]
fn undo_turn_is_offered_only_while_the_turn_is_the_latest_step() {
    in_temp_data_dir("undo_turn_is_offered_only_while_the_turn_is_the_latest_step", || {
        let fake = FakeTransport::default();
        fake.push_tool_calls(vec![new_board("b1", "Shelf", [0., 0., 0.], [600., 200., 19.])]);
        fake.push_text("Added a shelf.");
        let ctx = egui::Context::default();
        let mut app = connected_app(&fake);
        ask(&mut app, &ctx, "add a shelf");
        finish(&mut app, &ctx);
        let card = app
            .assistant
            .transcript
            .iter()
            .find_map(|entry| entry.changes.clone())
            .expect("the turn reports its changes");
        let undoable = |app: &BenchUpApp| {
            panel::turn_undoable(&card, app.doc.history.position(), app.doc.history.labels().count())
        };
        assert!(undoable(&app), "the newest step");
        // A later edit buries it.
        let id = *app.doc.model.objects.keys().next().unwrap();
        let before = app.doc.model.clone();
        app.doc.model.objects.get_mut(&id).unwrap().name = "Renamed".into();
        app.finish_edit("Rename", before, &ctx);
        assert!(!undoable(&app));
        // Undoing the later edit makes the turn the latest step again... but with a
        // redo pending it must not be offered, or Undo would skip the redo entry.
        app.undo(&ctx, false);
        assert!(!undoable(&app) || app.doc.history.labels().count() == app.doc.history.position());
        app.undo(&ctx, false);
        assert!(!undoable(&app), "already undone");
        assert!(card.erased_names.is_empty() && card.created.len() == 1);
    });
}

// ------------------------------------------------------- shared projects

/// A shared project with two boards, of which the user holds the lease on one.
fn shared_app(fake: &FakeTransport, ctx: &egui::Context) -> (BenchUpApp, u64, u64) {
    let mut app = connected_app(fake);
    let mut env = tools::ToolEnv::default();
    let mine = board(&mut app.doc.model, &mut env, "Mine", [0., 0., 0.], [100., 100., 100.]);
    let theirs = board(&mut app.doc.model, &mut env, "Theirs", [500., 0., 0.], [100., 100., 100.]);
    app.test_share_with_lease(ctx, &BTreeSet::from([mine]));
    (app, mine, theirs)
}
fn x_of(app: &BenchUpApp, id: u64) -> f64 {
    app.doc.model.selection_bounds(&BTreeSet::from([id])).unwrap().0.x
}
#[test]
fn mira_cannot_edit_entities_another_collaborator_holds() {
    in_temp_data_dir("mira_cannot_edit_entities_another_collaborator_holds", || {
        let fake = FakeTransport::default();
        let ctx = egui::Context::default();
        let (mut app, mine, theirs) = shared_app(&fake, &ctx);
        fake.push_tool_calls(vec![tool("a", "move", json!({"ids": [theirs], "delta_mm": [50, 0, 0]}))]);
        fake.push_tool_calls(vec![tool("b", "move", json!({"ids": [mine], "delta_mm": [50, 0, 0]}))]);
        fake.push_text("done");
        ask(&mut app, &ctx, "move both");
        app.poll_assistant(&ctx);
        app.poll_assistant(&ctx);
        let refused = {
            let requests = fake.requests.lock().unwrap();
            let content = requests[1]["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_owned();
            serde_json::from_str::<Value>(&content).unwrap()
        };
        assert_eq!(refused["ok"], false);
        assert_eq!(refused["error"], "not-leased");
        assert_eq!(refused["hint"], "select it first so you hold the edit lease");
        finish(&mut app, &ctx);
        assert!((x_of(&app, theirs) - 0.5).abs() < 1e-9, "the other collaborator's board did not move");
        assert!((x_of(&app, mine) - 0.05).abs() < 1e-9, "the leased board moved");
    });
}

#[test]
fn losing_edit_access_between_rounds_stops_the_turn() {
    in_temp_data_dir("losing_edit_access_between_rounds_stops_the_turn", || {
        let fake = FakeTransport::default();
        let ctx = egui::Context::default();
        let (mut app, mine, _) = shared_app(&fake, &ctx);
        fake.push_tool_calls(vec![tool("a", "move", json!({"ids": [mine], "delta_mm": [50, 0, 0]}))]);
        fake.push_tool_calls(vec![tool("b", "move", json!({"ids": [mine], "delta_mm": [50, 0, 0]}))]);
        ask(&mut app, &ctx, "move twice");
        app.poll_assistant(&ctx);
        assert!(app.assistant.busy());
        app.test_expire_lease();
        let reason = app.shared_edit_block_reason(&ctx);
        app.poll_assistant(&ctx);
        assert!(!app.assistant.busy(), "the turn stopped");
        let error = app.assistant.transcript.iter().rev().find(|e| e.role == EntryRole::Error).expect("the reason is reported");
        assert!(error.text.starts_with(reason), "{}", error.text);
        assert!(error.text.contains("rolled back"), "the edits of the first round were undone too");
        assert!(x_of(&app, mine) < 0.06, "the second round never ran");
    });
}

// ----------------------------------------------------------- bounded input

#[test]
fn only_the_first_32_calls_of_a_reply_run_and_results_are_truncated() {
    let fake = FakeTransport::default();
    let ctx = egui::Context::default();
    let mut model = Model::default();
    let calls: Vec<ToolCall> = (0..40).map(|i| tool(&format!("c{i}"), "get_scene", json!({}))).collect();
    fake.push_tool_calls(calls);
    fake.push_text("ok");
    let setup = turn::TurnSetup {
        prompt: "scene".into(),
        references: BTreeSet::new(),
        selection: BTreeSet::new(),
        include_selection: false,
        catalog: Default::default(),
        read_only: false,
        lease: None,
        history: Vec::new(),
    };
    // Enough objects that one get_scene result exceeds the per-result cap.
    let mut env = tools::ToolEnv::default();
    for i in 0..400 {
        board(&mut model, &mut env, &format!("Board number {i}"), [i as f64 * 10., 0., 0.], [5., 5., 5.]);
    }
    let mut running = turn::Turn::start(&ctx, &fake, ("https://x.test/v1", "k", "m"), &model, setup);
    while running.poll(&ctx, &fake, &mut model, &turn::Gate::default()).finished.is_none() {}
    let requests = fake.requests.lock().unwrap();
    let tool_messages: Vec<&Value> = requests[1]["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").collect();
    assert_eq!(tool_messages.len(), 40, "every call is answered");
    let text = |i: usize| tool_messages[i]["content"].as_str().unwrap();
    assert!(text(31).contains("edit_path"), "call 32 ran");
    let cut: Value = serde_json::from_str(text(31)).expect("a truncated result is still valid JSON");
    assert!(text(31).len() <= 8_000);
    assert_eq!((cut["ok"].clone(), cut["truncated"].clone()), (json!(true), json!(true)));
    assert!(cut["preview"].as_str().unwrap().starts_with("{\"edit_path\""), "{}", &text(31)[..60]);
    assert!(cut["preview"].as_str().unwrap().chars().count() <= 7_000);
    let refused: Value = serde_json::from_str(text(32)).unwrap();
    assert_eq!(refused, json!({"ok": false, "error": "too many calls in one reply"}));
}

#[test]
fn selection_tags_and_references_share_the_scene_budget() {
    let mut model = Model::default();
    let mut env = tools::ToolEnv::default();
    let mut ids = BTreeSet::new();
    for i in 0..300 {
        ids.insert(board(&mut model, &mut env, &format!("Board {i}"), [i as f64 * 10., 0., 0.], [5., 5., 5.]));
    }
    let references = scene::references(&model, &ids, 3_000);
    assert!(references.to_string().len() < 3_000 + 1_500, "references stop at their budget");
    assert!(references["omitted"].as_u64().unwrap() > 0);
    let scene = scene::summary(&model, &ids, &Default::default(), 24_000 - references.to_string().len());
    assert_eq!(scene["selection"].as_array().unwrap().len(), 200);
    assert_eq!(scene["selection_total"], 300);
    assert!(scene["omitted"].as_u64().unwrap() > 0);
}

// -------------------------------------------------------- connect / server

#[test]
fn a_key_is_saved_only_under_the_server_it_was_validated_against() {
    in_temp_data_dir("a_key_is_saved_only_under_the_server_it_was_validated_against", || {
        let fake = FakeTransport::default();
        let ctx = egui::Context::default();
        let mut model = Model::default();
        let (first, second) = ("https://one.example/v1", "https://two.example/v1");
        let mut assistant = Assistant::with_transport(Box::new(fake.clone()));
        *fake.models.lock().unwrap() = Some(Ok(vec!["m".into()]));
        assistant.settings.base_url = first.into();
        assistant.connect(&ctx, "key-one");
        // The user points the assistant at another server before the reply lands.
        assistant.settings.base_url = second.into();
        assistant.poll(&ctx, &mut model, &turn::Gate::default());
        assert!(assistant.key.is_none() && assistant.models.is_empty());
        assert_ne!(assistant.connection, Connection::Connected);
        assert!(credentials::load(first).is_none() && credentials::load(second).is_none(), "the stale reply is dropped");
        // An answer for the current server is saved under it.
        assistant.connect(&ctx, "key-two");
        assistant.poll(&ctx, &mut model, &turn::Gate::default());
        assert_eq!(credentials::load(second).as_deref(), Some("key-two"));
        assert!(credentials::load(first).is_none());
    });
}

// ------------------------------------------------- documentation findings

fn plain_setup(prompt: &str) -> turn::TurnSetup {
    turn::TurnSetup {
        prompt: prompt.into(),
        references: BTreeSet::new(),
        selection: BTreeSet::new(),
        include_selection: true,
        catalog: Default::default(),
        read_only: false,
        lease: None,
        history: Vec::new(),
    }
}
fn run_to_end(fake: &FakeTransport, ctx: &egui::Context, model: &mut Model, setup: turn::TurnSetup, gate: &turn::Gate) -> turn::TurnResult {
    let mut running = turn::Turn::start(ctx, fake, ("https://x.test/v1", "k", "m"), model, setup);
    loop {
        if let Some(result) = running.poll(ctx, fake, model, gate).finished {
            return result;
        }
    }
}

#[test]
fn include_selection_off_hides_the_selection_from_the_scene_and_get_selection() {
    let ctx = egui::Context::default();
    let mut model = Model::default();
    let id = board(&mut model, &mut tools::ToolEnv::default(), "Shelf", [0., 0., 0.], [600., 200., 19.]);
    let scene_selection = |include: bool| -> (Value, String) {
        let fake = FakeTransport::default();
        fake.push_tool_calls(vec![tool("g", "get_selection", json!({}))]);
        fake.push_text("ok");
        let mut setup = plain_setup("look");
        setup.selection = BTreeSet::from([id]);
        setup.include_selection = include;
        run_to_end(&fake, &ctx, &mut model.clone(), setup, &turn::Gate::default());
        let requests = fake.requests.lock().unwrap();
        let context = requests[0]["messages"][1]["content"].as_str().unwrap().to_owned();
        let json_start = context.find('{').unwrap();
        let scene: Value = serde_json::from_str(&context[json_start..]).unwrap();
        let tool_text = requests[1]["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_owned();
        (scene, tool_text)
    };
    let (scene, tool_text) = scene_selection(true);
    assert_eq!(scene["scene"]["selection"], json!([id]));
    assert!(serde_json::from_str::<Value>(&tool_text).unwrap()["selection"] == json!([id]));
    let (scene, tool_text) = scene_selection(false);
    assert_eq!(scene["scene"]["selection"], json!([]), "not listed in the scene summary");
    assert_eq!(scene["references"]["objects"].as_array().map_or(0, Vec::len), 0, "not attached as references");
    let reply: Value = serde_json::from_str(&tool_text).unwrap();
    assert_eq!(reply["selection"], json!([]));
    assert_eq!(reply["note"], "The designer has turned off sharing their selection.");
}

#[test]
fn a_rolled_back_turn_does_not_claim_its_edits_on_the_change_card() {
    in_temp_data_dir("a_rolled_back_turn_does_not_claim_its_edits_on_the_change_card", || {
        let fake = FakeTransport::default();
        let ctx = egui::Context::default();
        let (mut app, mine, _) = shared_app(&fake, &ctx);
        fake.push_tool_calls(vec![tool("a", "move", json!({"ids": [mine], "delta_mm": [50, 0, 0]}))]);
        fake.push_text("never delivered");
        ask(&mut app, &ctx, "move it");
        app.poll_assistant(&ctx);
        assert!((x_of(&app, mine) - 0.05).abs() < 1e-9, "the edit was applied while access held");
        app.test_expire_lease();
        app.poll_assistant(&ctx);
        assert!(!app.assistant.busy());
        assert!(x_of(&app, mine).abs() < 1e-9, "the commit was refused and rolled back");
        assert_eq!(app.doc.history.labels().count(), 0);
        assert!(app.assistant.transcript.iter().all(|e| e.changes.is_none()), "no card claims the rolled-back edits");
        let error = app.assistant.transcript.iter().rev().find(|e| e.role == EntryRole::Error).unwrap();
        assert!(error.text.contains("rolled back"), "{}", error.text);
    });
}

fn grouped_pair(model: &mut Model) -> (u64, u64) {
    let mut env = tools::ToolEnv::default();
    let a = board(model, &mut env, "A", [0., 0., 0.], [100., 100., 20.]);
    let b = board(model, &mut env, "B", [200., 0., 0.], [100., 100., 20.]);
    let group = tools_tests::run(model, &mut env, "group", json!({"ids": [a, b], "name": "Pair", "component": false})).created[0];
    (group, a)
}

#[test]
fn a_turn_puts_the_designer_back_in_their_edit_scope_and_undo_keeps_it() {
    in_temp_data_dir("a_turn_puts_the_designer_back_in_their_edit_scope_and_undo_keeps_it", || {
        let ctx = egui::Context::default();
        // Started at the top level; MIRA opens the group and edits inside it.
        let fake = FakeTransport::default();
        let mut app = connected_app(&fake);
        let (group, _) = grouped_pair(&mut app.doc.model);
        let inner = app.doc.model.objects[&group].entities.iter().copied().find(|id| app.doc.model.objects.contains_key(id)).unwrap();
        let original = app.doc.model.objects[&inner].name.clone();
        assert_eq!(app.doc.model.edit_context, None);
        fake.push_tool_calls(vec![tool("a", "enter_object", json!({"id": group})), tool("b", "rename", json!({"id": inner, "name": "Inner"}))]);
        fake.push_text("Renamed.");
        ask(&mut app, &ctx, "rename inside");
        finish(&mut app, &ctx);
        assert_eq!(app.doc.model.objects[&inner].name, "Inner");
        assert_eq!(app.doc.model.edit_context, None, "back at the top level");
        app.undo(&ctx, false);
        assert_eq!(app.doc.model.objects[&inner].name, original);
        assert_eq!(app.doc.model.edit_context, None, "undo leaves the designer where they were");

        // Started inside the group; MIRA leaves it to edit the top level.
        let fake = FakeTransport::default();
        let mut app = connected_app(&fake);
        let (group, _) = grouped_pair(&mut app.doc.model);
        assert!(app.doc.model.enter_object(group));
        fake.push_tool_calls(vec![tool("a", "exit_to_root", json!({})), tool("b", "rename", json!({"id": group, "name": "Top"}))]);
        fake.push_text("Renamed.");
        ask(&mut app, &ctx, "rename the group");
        finish(&mut app, &ctx);
        assert_eq!(app.doc.model.objects[&group].name, "Top");
        assert_eq!(app.doc.model.edit_context, Some(group), "back in the group");
        app.undo(&ctx, false);
        assert_eq!(app.doc.model.objects[&group].name, "Pair");
        assert_eq!(app.doc.model.edit_context, Some(group));
    });
}

#[test]
fn edit_access_is_rechecked_before_each_mutating_call_in_a_batch() {
    let fake = FakeTransport::default();
    let ctx = egui::Context::default();
    let mut model = Model::default();
    fake.push_tool_calls(vec![
        tool("r", "get_scene", json!({})),
        new_board("b1", "One", [0., 0., 0.], [100., 100., 20.]),
        new_board("b2", "Two", [200., 0., 0.], [100., 100., 20.]),
    ]);
    fake.push_text("never delivered");
    let asked = std::cell::Cell::new(0);
    let recheck = || {
        asked.set(asked.get() + 1);
        (asked.get() > 1).then(|| "access lapsed".to_owned())
    };
    let gate = turn::Gate { recheck: Some(&recheck), ..Default::default() };
    let result = run_to_end(&fake, &ctx, &mut model, plain_setup("two boards"), &gate);
    assert_eq!(asked.get(), 2, "read-only calls are not checked; each edit is");
    assert_eq!(model.objects.len(), 1, "the second board never ran");
    assert_eq!(result.error.as_deref(), Some("access lapsed"));
    assert!(result.stopped);
    assert_eq!(result.created.len(), 1);
}

#[test]
fn an_oversized_tool_result_is_still_valid_json() {
    let mut text = String::from("{\"items\":[");
    text.push_str(&"\"quoted \\\"text\\\"\",".repeat(2_000));
    text.push_str("0]}");
    for ok in [true, false] {
        let cut = turn::truncated_result(&text, ok);
        let value: Value = serde_json::from_str(&cut).unwrap();
        assert_eq!(value["ok"], ok);
        assert_eq!(value["truncated"], true);
        assert!(text.starts_with(value["preview"].as_str().unwrap()));
        assert!(cut.len() <= 8_000, "escaped quotes do not push it over the cap: {}", cut.len());
    }
}

#[test]
fn history_keeps_alternating_roles_after_stopped_and_failed_turns() {
    in_temp_data_dir("history_keeps_alternating_roles_after_stopped_and_failed_turns", || {
        let fake = FakeTransport::default();
        let ctx = egui::Context::default();
        let mut app = connected_app(&fake);
        fake.push_text("never delivered");
        ask(&mut app, &ctx, "first");
        app.assistant_stop(&ctx);
        fake.replies.lock().unwrap().push_back(Err(ChatError::InvalidKey));
        ask(&mut app, &ctx, "second");
        finish(&mut app, &ctx);
        fake.push_text("fine");
        ask(&mut app, &ctx, "third");
        finish(&mut app, &ctx);
        let roles: Vec<Role> = app.assistant.history.iter().map(|m| m.role).collect();
        assert_eq!(roles, [Role::User, Role::Assistant, Role::User, Role::Assistant, Role::User, Role::Assistant]);
        assert_eq!(app.assistant.history[1].content, "(stopped)");
        assert!(app.assistant.history[3].content.starts_with("(error: "), "{}", app.assistant.history[3].content);
        let requests = fake.requests.lock().unwrap();
        let roles: Vec<&str> = requests.last().unwrap()["messages"].as_array().unwrap().iter().skip(2).map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "user", "assistant", "user"]);
    });
}

#[test]
fn a_turn_without_changes_leaves_the_earlier_undo_card_intact() {
    in_temp_data_dir("a_turn_without_changes_leaves_the_earlier_undo_card_intact", || {
        let fake = FakeTransport::default();
        let ctx = egui::Context::default();
        let mut app = connected_app(&fake);
        fake.push_tool_calls(vec![new_board("b1", "Shelf", [0., 0., 0.], [600., 200., 19.])]);
        fake.push_text("Added a shelf.");
        ask(&mut app, &ctx, "add a shelf");
        finish(&mut app, &ctx);
        let card = app.assistant.transcript.iter().find_map(|e| e.changes.clone()).unwrap();
        assert_eq!(card.position, Some(1));
        fake.push_text("Looks fine, nothing to change.");
        ask(&mut app, &ctx, "is it fine?");
        finish(&mut app, &ctx);
        let cards: Vec<&Changes> = app.assistant.transcript.iter().filter_map(|e| e.changes.as_ref()).collect();
        assert_eq!(cards, [&card], "the earlier card keeps its position and names");
    });
}

#[test]
fn a_typed_model_survives_a_model_list_that_omits_it() {
    let fake = FakeTransport::default();
    let ctx = egui::Context::default();
    let mut model = Model::default();
    let mut assistant = Assistant::with_transport(Box::new(fake.clone()));
    *fake.models.lock().unwrap() = Some(Ok(vec!["alpha".into(), "qwen-3".into()]));
    assistant.key = Some("k".into());
    assistant.settings.model = "my-own-model".into();
    assistant.refresh_models(&ctx);
    assistant.poll(&ctx, &mut model, &turn::Gate::default());
    assert_eq!(assistant.settings.model, "my-own-model");
    assert_eq!(assistant.models, ["alpha", "qwen-3"]);
    // Only an empty choice gets a default.
    assistant.settings.model.clear();
    assistant.refresh_models(&ctx);
    assistant.poll(&ctx, &mut model, &turn::Gate::default());
    assert_eq!(assistant.settings.model, "qwen-3");
}

#[test]
fn a_new_server_address_is_kept_only_after_a_successful_connect() {
    in_temp_data_dir("a_new_server_address_is_kept_only_after_a_successful_connect", || {
        let fake = FakeTransport::default();
        let ctx = egui::Context::default();
        let mut model = Model::default();
        let (old, new) = ("https://old.example/v1", "https://new.example/v1");
        let mut assistant = Assistant::with_transport(Box::new(fake.clone()));
        assistant.settings.base_url = old.into();
        assistant.settings.model = "old-model".into();
        assistant.key = Some("old-key".into());
        *fake.models.lock().unwrap() = Some(Err(ChatError::InvalidKey));
        assistant.connect_to(&ctx, new, "bad-key");
        assistant.poll(&ctx, &mut model, &turn::Gate::default());
        assert!(matches!(assistant.connection, Connection::Failed(_)));
        assert_eq!(assistant.settings.base_url, old, "a rejected key leaves the address alone");
        assert_eq!(assistant.key.as_deref(), Some("old-key"));
        assert!(credentials::load(new).is_none());
        *fake.models.lock().unwrap() = Some(Ok(vec!["new-model".into()]));
        assistant.connect_to(&ctx, new, "good-key");
        assert_eq!(assistant.settings.base_url, old, "not yet");
        assistant.poll(&ctx, &mut model, &turn::Gate::default());
        assert_eq!(assistant.settings.base_url, new);
        assert_eq!(assistant.key.as_deref(), Some("good-key"));
        assert_eq!(assistant.settings.model, "new-model");
        assert_eq!(credentials::load(new).as_deref(), Some("good-key"));
    });
}

#[test]
fn bring_to_front_raises_the_embedded_window_layer() {
    let ctx = egui::Context::default();
    assert!(ctx.embed_viewports(), "this is the web/embedded path");
    let mut app = BenchUpApp::with_document(benchup_core::Document::default(), None, true);
    app.assistant.settings.window.detached = true;
    let other = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("another window"));
    let top = |ctx: &egui::Context| ctx.memory(|m| m.areas().top_layer_id(egui::Order::Middle));
    ctx.move_to_top(window::embedded_layer());
    ctx.move_to_top(other);
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    assert_eq!(top(&ctx), Some(other));
    app.assistant_reveal(&ctx);
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    assert_eq!(top(&ctx), Some(window::embedded_layer()));
    assert!(app.assistant.ui.focus_prompt, "and the prompt is focused");
}
