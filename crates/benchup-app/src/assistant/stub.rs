//! Stand-in used when the `assistant` feature is off ("Build with MIRA" is not
//! in this build). It has the small surface the rest of the app calls, draws
//! nothing and does no work. Saved MIRA preferences pass through untouched, so
//! a build without the add-on does not erase what a full build saved
//! (the preference types live in `addons`, which is always compiled).
use super::*;
use std::collections::BTreeMap;

pub(super) use addons::AssistantPreferences as Preferences;

#[derive(Default)]
pub(super) struct Assistant {
    saved: Preferences,
}
impl Assistant {
    pub(super) fn preferences(&self) -> Preferences {
        self.saved.clone()
    }
    pub(super) fn restore(&mut self, preferences: &Preferences) {
        self.saved = preferences.clone();
    }
    pub(super) fn busy(&self) -> bool {
        false
    }
    pub(super) fn discard_turn(&mut self, _: &mut Model, _: String) -> bool {
        false
    }
    pub(super) fn record_agent(&mut self, _: &str, _: &str, _: Changes) {}
}

pub(super) use agent_host::Changes;

pub(super) mod panel {
    use super::*;
    pub(crate) fn erased_names(_: &Model, _: &[u64]) -> BTreeMap<u64, String> {
        BTreeMap::new()
    }
}

impl BenchUpApp {
    pub(super) fn poll_assistant(&mut self, _: &egui::Context) {}
    pub(super) fn assistant_stop(&mut self, _: &egui::Context) {}
    pub(super) fn assistant_toggle_shortcut(&mut self, _: &egui::Context) -> bool {
        false
    }
    pub(super) fn assistant_busy_banner(&mut self, _: &egui::Context) {}
    pub(super) fn assistant_window(&mut self, _: &egui::Context) {}
    pub(super) fn assistant_section_contents(&mut self, _: &mut egui::Ui) {}
    pub(super) fn assistant_view_menu_item(&mut self, _: &mut egui::Ui) {}
    pub(super) fn assistant_add_chips(&mut self, _: impl IntoIterator<Item = u64>) {}
    pub(super) fn assistant_reveal(&mut self, _: &egui::Context) {}
}
