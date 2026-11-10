//! Settings section and status bar, driven through the shared UI `Harness`.
//! No test connects to anything or writes to a data directory (the token is
//! preset, and only the read-only token lookup runs).
use super::*;

fn open_add_ons(h: &mut Harness) -> egui::FullOutput {
    h.app.agent_link.set_token_for_test(TEST_TOKEN);
    h.app.settings_open = true;
    h.app.general_settings.section = settings::Section::AddOns;
    h.frame(vec![], egui::Modifiers::NONE);
    h.frame(vec![], egui::Modifiers::NONE);
    h.frame(vec![], egui::Modifiers::NONE)
}

#[test]
fn add_ons_settings_show_the_agent_connection_and_toggle_it_off_by_default() {
    let mut h = Harness::new();
    let output = open_add_ons(&mut h);
    assert!(!h.app.general_settings.agent_link.enabled);
    let toggle = text_position(&output, "Allow agent control");
    text_position(&output, "Agent connection");
    text_position(&output, "Agent control is off.");
    text_position(&output, "Copy Claude Code command");
    // The token is masked until revealed, and never shown in full by default.
    let shown = output_text(&output);
    assert!(!shown.iter().any(|t| t.contains("aaaaaaaa-bbbb")), "masked");
    h.click(toggle);
    assert!(h.app.general_settings.agent_link.enabled);
}

/// A bridge that completes the pairing handshake as soon as it is connected to.
struct ReadyBridge;
const TEST_TOKEN: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const BRIDGE_NONCE: &str = "abababababababababababababababababababababababababababababababab";
#[derive(Default)]
struct ReadyLink {
    challenged: bool,
    app_nonce: Option<String>,
    ready: bool,
}
impl agent_link::Link for ReadyLink {
    fn events(&mut self) -> Vec<Result<serde_json::Value, String>> {
        if !std::mem::replace(&mut self.challenged, true) {
            return vec![Ok(serde_json::json!({"type": "challenge", "nonce": BRIDGE_NONCE, "protocol": 2}))];
        }
        match (&self.app_nonce, self.ready) {
            (Some(nonce), false) => {
                self.ready = true;
                let proof = benchup_tools::pairing::bridge_proof(TEST_TOKEN, BRIDGE_NONCE, nonce);
                vec![Ok(serde_json::json!({"type": "ready", "protocol": 2, "proof": proof}))]
            }
            _ => Vec::new(),
        }
    }
    fn send(&mut self, message: serde_json::Value) -> Result<(), String> {
        if message["type"] == "prove" {
            self.app_nonce = message["nonce"].as_str().map(str::to_owned);
        }
        Ok(())
    }
}
impl agent_link::Connector for ReadyBridge {
    fn connect(&mut self, _: &egui::Context, _: &str, _: &str, _: &str) -> Box<dyn agent_link::Link> {
        Box::new(ReadyLink::default())
    }
}

#[test]
fn status_bar_says_when_an_agent_is_connected() {
    let mut h = Harness::new();
    h.app.agent_link = agent_link::AgentLink::with_connector(Box::new(ReadyBridge));
    h.app.agent_link.set_token_for_test(TEST_TOKEN);
    assert!(!output_text(&h.frame(vec![], egui::Modifiers::NONE)).iter().any(|t| t == "Agent connected"));
    h.app.general_settings.agent_link.enabled = true;
    h.frame(vec![], egui::Modifiers::NONE);
    let output = h.frame(vec![], egui::Modifiers::NONE);
    text_position(&output, "Agent connected");
    h.app.general_settings.agent_link.enabled = false;
    let output = h.frame(vec![], egui::Modifiers::NONE);
    assert!(!output_text(&output).iter().any(|t| t == "Agent connected"));
}

fn output_text(output: &egui::FullOutput) -> Vec<String> {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(value) => text.push(value.galley.job.text.clone()),
            egui::Shape::Vec(values) => values.iter().for_each(|value| collect(value, text)),
            _ => {}
        }
    }
    let mut text = Vec::new();
    output.shapes.iter().for_each(|shape| collect(&shape.shape, &mut text));
    text
}
