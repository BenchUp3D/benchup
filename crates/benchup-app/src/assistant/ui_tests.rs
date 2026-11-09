//! Panel rendering, connect dialog, context menu and shortcut tests, driven
//! through the shared UI `Harness`. None of them touches the network or the
//! real data directory.
use super::*;
use crate::app::assistant::{
    panel::{self, Chip},
    Assistant, Changes, Connection, Entry, EntryRole, Prompt,
};

fn object(h: &mut Harness, name: &str, offset: DVec3) -> u64 {
    let edge = h.app.doc.model.add_edge(offset, offset + DVec3::X * 2.).unwrap();
    h.app
        .doc
        .model
        .make_object(&BTreeSet::from([edge]), false, name)
        .unwrap()
}

fn open_panel(h: &mut Harness) {
    h.size.y = 2000.;
    // Keep the long sections collapsed so the panel sits on screen.
    for key in ["outliner", "entity_info", "units", "components", "tags", "display", "view",
        "time_cost_estimates", "materials", "history"] {
        sidebar::set_section_open(&h.ctx, key, false);
    }
    h.app.assistant.key = Some("test-key".into());
    h.app.assistant.ui.key_checked = true;
    h.app.assistant.ui.models_requested = true;
    h.app.assistant.settings.model = "test-model".into();
    sidebar::set_section_open(&h.ctx, "assistant", true);
    h.frame(vec![], egui::Modifiers::NONE);
}

#[test]
fn context_menu_ask_mira_about_this_attaches_the_selection_and_opens_the_panel() {
    let mut h = Harness::new();
    let beam = object(&mut h, "Beam", DVec3::ZERO);
    let plank = object(&mut h, "Plank", DVec3::Y * 2.);
    sidebar::set_section_open(&h.ctx, "assistant", false);
    h.frame(vec![], egui::Modifiers::NONE);
    let output = h.frame(vec![], egui::Modifiers::NONE);
    let row = text_position(&output, "Beam");
    h.app.selected = BTreeSet::from([beam, plank]);
    let menu = open_context_menu(&mut h, row);
    assert_eq!(h.app.selected, BTreeSet::from([beam, plank]), "right-clicking a selected row keeps the selection");
    h.click(text_position(&menu, "Ask MIRA about this"));
    let chips: BTreeSet<u64> = h.app.assistant.ui.chips.iter().map(|c| c.id).collect();
    assert_eq!(chips, BTreeSet::from([beam, plank]), "the selection becomes references");
    assert!(h.app.inspector);
    assert!(sidebar::section_open(&h.ctx, "assistant", false), "the section opens");
    // Asking twice does not duplicate chips; a hidden sidebar is reopened.
    h.app.inspector = false;
    h.app.context_action(&h.ctx, context_menu::ContextAction::AskAssistant);
    assert_eq!(h.app.assistant.ui.chips.len(), 2);
    assert!(h.app.inspector);
    assert!(!h.app.project.modified);
}

#[test]
fn ask_mira_is_disabled_without_a_selection() {
    let mut h = Harness::new();
    object(&mut h, "Beam", DVec3::ZERO);
    h.app.selected.clear();
    h.app.context_action(&h.ctx, context_menu::ContextAction::AskAssistant);
    assert!(h.app.assistant.ui.chips.is_empty());
}

#[test]
fn connect_dialog_declares_cancel_before_connect_so_cancel_renders_on_the_right() {
    let mut h = Harness::new();
    h.frame(vec![], egui::Modifiers::NONE);
    h.app.assistant_open_connect(&h.ctx, false);
    // A dialog is measured on its first frame and drawn on the next.
    h.frame(vec![], egui::Modifiers::NONE);
    let output = h.frame(vec![], egui::Modifiers::NONE);
    let cancel = text_position(&output, "Cancel");
    let connect = text_position(&output, "Connect");
    assert!(cancel.x > connect.x, "Cancel is on the right of Connect");
    assert!((cancel.y - connect.y).abs() < 4., "both sit on the footer row");
    // The key field is masked, and a rejected key is reported inline.
    text_position(&output, "Get a key");
    h.app.assistant.ui.connect.as_mut().unwrap().submitted = true;
    h.app.assistant.connection = Connection::Failed("Invalid API key".into());
    let output = h.frame(vec![], egui::Modifiers::NONE);
    text_position(&output, "Invalid API key");
    // Cancel closes it and clears the failure.
    h.click(text_position(&output, "Cancel"));
    assert!(h.app.assistant.ui.connect.is_none());
    assert_eq!(h.app.assistant.connection, Connection::Disconnected);
}

#[test]
fn disconnected_panel_offers_connect_and_connected_panel_shows_the_composer() {
    let mut h = Harness::new();
    for key in ["outliner", "entity_info", "units", "components", "tags", "display", "view",
        "time_cost_estimates", "materials", "history"] {
        sidebar::set_section_open(&h.ctx, key, false);
    }
    h.app.assistant.ui.key_checked = true;
    sidebar::set_section_open(&h.ctx, "assistant", true);
    h.frame(vec![], egui::Modifiers::NONE);
    let output = h.frame(vec![], egui::Modifiers::NONE);
    let connect = text_position(&output, "Connect");
    h.click(connect);
    assert!(h.app.assistant.ui.connect.is_some());
    h.app.assistant.ui.connect = None;

    open_panel(&mut h);
    let output = h.frame(vec![], egui::Modifiers::NONE);
    text_position(&output, "Send");
    text_position(&output, "Include selection");
    text_position(&output, "New chat");
    // Until dismissed, the privacy notice names the destination host.
    text_position(&output, "A summary of your model (names and dimensions) is sent to api.endlesstokens.ai.");
    let dismiss = text_position(&output, "Don't show again");
    h.click(dismiss);
    assert!(h.app.assistant.settings.privacy_dismissed);
}

#[test]
fn change_cards_list_rows_focus_entities_and_offer_undo_only_for_the_latest_turn() {
    let mut h = Harness::new();
    let beam = object(&mut h, "Beam", DVec3::ZERO);
    open_panel(&mut h);
    h.app.assistant.transcript.push(Entry {
        role: EntryRole::Assistant,
        text: "Added a beam.".into(),
        changes: Some(Changes {
            created: vec![beam],
            position: Some(h.app.doc.history.position()),
            ..Changes::default()
        }),
    });
    h.frame(vec![], egui::Modifiers::NONE);
    let output = h.frame(vec![], egui::Modifiers::NONE);
    text_position(&output, "Created 1 · Modified 0 · Erased 0 · Joints 0");
    text_position(&output, "Undo turn");
    // Clicking the row selects the entity (and frames it).
    h.app.selected.clear();
    h.click(text_position(&output, "Created · Beam"));
    assert_eq!(h.app.selected, BTreeSet::from([beam]));
}

#[test]
fn send_enters_submit_and_a_blocked_send_reports_why() {
    let mut h = Harness::new();
    open_panel(&mut h);
    // Not connected to a working server: the error is shown, nothing is sent.
    h.app.assistant.key = None;
    h.app.assistant.ui.text = "make it taller".into();
    h.app.assistant_submit(&h.ctx);
    assert_eq!(
        h.app.assistant.ui.notice.as_deref(),
        Some("Connect to the assistant server with an API key first")
    );
    assert_eq!(h.app.assistant.ui.text, "make it taller", "the draft is kept");
}

#[test]
fn shortcut_is_cmd_shift_m_by_default_and_toggles_the_panel() {
    let mut h = Harness::new();
    h.frame(vec![], egui::Modifiers::NONE);
    assert_eq!(h.app.general_settings.assistant_key, "M");
    // No other binding already uses this chord.
    let chord = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    assert_eq!(h.app.assistant_shortcut(), egui::KeyboardShortcut::new(chord, Key::M));
    h.app.inspector = true;
    sidebar::set_section_open(&h.ctx, "assistant", false);
    assert!(!h.app.assistant_visible(&h.ctx));
    let press = |h: &mut Harness| {
        h.frame(
            vec![egui::Event::Key {
                key: Key::M,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: chord,
            }],
            chord,
        );
    };
    press(&mut h);
    assert!(h.app.assistant_visible(&h.ctx), "the first press shows the panel");
    press(&mut h);
    assert!(!h.app.assistant_visible(&h.ctx), "the second press hides it");
    // Settings written before this key existed still load.
    let older: crate::app::settings::GeneralSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(older.assistant_key, "M");
}

#[test]
fn drop_helpers_and_labels_are_stable() {
    let mut h = Harness::new();
    let beam = object(&mut h, "Beam", DVec3::ZERO);
    h.app.assistant_add_chips([beam, beam, 999_999]);
    assert_eq!(h.app.assistant.ui.chips, vec![Chip { label: "Beam".into(), id: beam }]);
    assert_eq!(panel::entity_label(&h.app.doc.model, beam).0, "Beam");
    assert_eq!(panel::host_of("https://api.endlesstokens.ai/v1"), "api.endlesstokens.ai");
}

#[test]
fn drawing_tools_are_paused_in_the_viewport_while_mira_works_but_selecting_is_not() {
    use crate::app::assistant::client::{ChatError, ChatRequest, ChatResponse, ChatTransport};
    use std::sync::{mpsc, Mutex};
    /// Never answers, so the turn stays pending across frames.
    #[derive(Default)]
    struct Stall(Mutex<Vec<mpsc::Sender<Result<ChatResponse, ChatError>>>>);
    impl ChatTransport for Stall {
        fn models(&self, _: &egui::Context, _: &str, _: &str) -> mpsc::Receiver<Result<Vec<String>, ChatError>> {
            mpsc::channel().1
        }
        fn chat(&self, _: &egui::Context, _: &str, _: &str, _: &ChatRequest) -> mpsc::Receiver<Result<ChatResponse, ChatError>> {
            let (sender, receiver) = mpsc::channel();
            self.0.lock().unwrap().push(sender);
            receiver
        }
    }
    let start = |busy: bool| {
        let mut h = Harness::new();
        h.app.assistant = Assistant::with_transport(Box::new(Stall::default()));
        h.app.assistant.key = Some("k".into());
        h.app.assistant.settings.model = "m".into();
        if busy {
            h.app
                .assistant_send(&h.ctx, Prompt { prompt: "look".into(), references: BTreeSet::new() })
                .unwrap();
            assert!(h.app.assistant.busy());
        }
        h.app.set_tool(Tool::Line);
        let point = h.app.viewport.center();
        h.click(point);
        h
    };
    assert!(start(false).app.line_start.is_some(), "control: the Line tool starts a line");
    let h = start(true);
    assert!(h.app.line_start.is_none() && h.app.drawing.is_none(), "no drawing while MIRA edits");
    // Escape stops the turn and hands the model back.
    let mut h = h;
    h.key(Key::Escape);
    assert!(!h.app.assistant.busy());
}

#[test]
fn assistant_shortcut_keys_cannot_collide_with_ctrl_shift_commands() {
    use crate::app::settings::GeneralSettings;
    // Save As (S), Redo (Z), Paste in place (V) and the other Ctrl/Cmd letters.
    for key in ["S", "Z", "V", "A", "C", "X", "Y", "F", "N", "O", "Space"] {
        assert!(!GeneralSettings::assistant_key_allowed(key), "{key}");
        let mut settings = GeneralSettings { assistant_key: key.into(), ..Default::default() };
        settings.normalize();
        assert_eq!(settings.assistant_key, "M", "a saved {key} falls back to the default");
    }
    for key in ["M", "K", "B", "J"] {
        assert!(GeneralSettings::assistant_key_allowed(key), "{key}");
    }
    assert!(!GeneralSettings::assistant_key_allowed("m") && !GeneralSettings::assistant_key_allowed(""));
}

fn enter_event(pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers }
}

#[test]
fn enter_sends_the_prompt_and_shift_enter_inserts_a_newline() {
    let mut h = Harness::new();
    let fake = crate::app::assistant::client::fake::FakeTransport::default();
    h.app.assistant = Assistant::with_transport(Box::new(fake.clone()));
    open_panel(&mut h);
    h.app.assistant.ui.text = "make it taller".into();
    h.app.assistant.ui.focus_prompt = true;
    h.frame(vec![], egui::Modifiers::NONE);
    h.frame(vec![], egui::Modifiers::NONE);
    // Shift+Enter: a newline, nothing sent.
    let shift = egui::Modifiers { shift: true, ..Default::default() };
    h.frame(vec![enter_event(true, shift)], shift);
    h.frame(vec![enter_event(false, shift)], egui::Modifiers::NONE);
    assert!(!h.app.assistant.busy() && fake.requests.lock().unwrap().is_empty(), "Shift+Enter does not send");
    assert!(h.app.assistant.ui.text.contains('\n'), "{:?}", h.app.assistant.ui.text);
    // Enter alone sends.
    h.frame(vec![enter_event(true, egui::Modifiers::NONE)], egui::Modifiers::NONE);
    h.frame(vec![enter_event(false, egui::Modifiers::NONE)], egui::Modifiers::NONE);
    assert_eq!(fake.requests.lock().unwrap().len(), 1, "Enter sends the prompt");
    assert!(h.app.assistant.ui.text.is_empty(), "and clears the composer");
}

#[test]
fn change_cards_count_every_row_they_do_not_show() {
    let mut h = Harness::new();
    open_panel(&mut h);
    // 10 created + 5 erased = 15 rows against a 12-row limit.
    h.app.assistant.transcript.push(Entry {
        role: EntryRole::Assistant,
        text: "Big change.".into(),
        changes: Some(Changes {
            created: (1000..1010).collect(),
            erased: (2000..2005).collect(),
            ..Changes::default()
        }),
    });
    h.frame(vec![], egui::Modifiers::NONE);
    let output = h.frame(vec![], egui::Modifiers::NONE);
    text_position(&output, "+3 more");
}
