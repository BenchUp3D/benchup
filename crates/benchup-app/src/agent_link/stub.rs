//! Stand-in used when the `agent-link` feature is off. Saved preferences pass
//! through untouched so a build without the add-on does not erase them.
use super::*;

pub(super) use addons::AgentPreferences as Preferences;
impl Preferences {
    pub(super) fn normalize(&mut self) {
        if self.port == 0 {
            self.port = addons::DEFAULT_PORT;
        }
    }
}

#[derive(Default)]
pub(super) struct AgentLink;

impl BenchUpApp {
    pub(super) fn poll_agent_link(&mut self, _: &egui::Context) {}
    pub(super) fn agent_status_text(&self) -> Option<&'static str> {
        None
    }
    pub(crate) fn agent_link_settings(&mut self, _: &mut egui::Ui) {}
}
