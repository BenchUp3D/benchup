//! Settings → Add-ons → Agent connection.
use super::*;

fn masked(token: &str) -> String {
    let tail: String = token.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("••••••••-••••-••••-••••-••••••••{tail}")
}

impl BenchUpApp {
    pub(crate) fn agent_link_settings(&mut self, ui: &mut egui::Ui) {
        let colors = Palette::for_ui(ui);
        ui.text_strong(tr!("agent_link.title"));
        ui.text_weak(tr!("agent_link.help"));
        let prefs = &mut self.general_settings.agent_link;
        ui.checkbox(&mut prefs.enabled, tr!("agent_link.allow")).on_hover_text(tr!("agent_link.allow_help"));
        ui.horizontal(|ui| {
            ui.text_label(tr!("agent_link.port"));
            ui.add(egui::DragValue::new(&mut prefs.port).range(1024..=65535).speed(1))
                .on_hover_text(tr!("agent_link.port_help"));
        });
        prefs.normalize();
        let port = prefs.port;
        let status = match &self.agent_link.status {
            Status::Off => tr!("agent_link.status.off").to_owned(),
            Status::Connecting => tr!("agent_link.status.connecting").to_owned(),
            Status::Connected { client: None } => tr!("agent_link.status.waiting").to_owned(),
            Status::Connected { client: Some(client) } => tr!("agent_link.status.connected", client = client),
            Status::Error(message) => tr!("agent_link.status.error", message = message),
        };
        let status_color = if self.agent_link.connected() { colors.ink } else { colors.muted };
        ui.text_label(RichText::new(status).color(status_color));

        ui.add_space(4.);
        ui.text_strong(tr!("agent_link.token"));
        self.agent_link.load_token();
        match self.agent_link.token.clone() {
            None => {
                ui.text_weak(tr!("agent_link.token_none"));
            }
            Some(token) => {
                let shown = if self.agent_link.show_token { token.clone() } else { masked(&token) };
                ui.text_label(RichText::new(shown).monospace());
            }
        }
        if let Some(error) = &self.agent_link.token_error {
            ui.text_label(RichText::new(tr!("agent_link.token_save_failed", error = error)).color(colors.accent));
        }
        ui.text_weak(tr!("agent_link.token_help"));
        let has_token = self.agent_link.token.is_some();
        ui.horizontal_wrapped(|ui| {
            let label = if self.agent_link.show_token { tr!("agent_link.hide") } else { tr!("agent_link.show") };
            if ui.add_enabled(has_token, egui::Button::new(label)).clicked() {
                self.agent_link.show_token = !self.agent_link.show_token;
            }
            if ui.add_enabled(has_token, egui::Button::new(tr!("agent_link.copy_token"))).clicked()
                && let Some(token) = &self.agent_link.token
            {
                ui.ctx().copy_text(token.clone());
            }
            let label = if has_token { tr!("agent_link.regenerate") } else { tr!("agent_link.generate") };
            if ui.button(label).on_hover_text(tr!("agent_link.regenerate_help")).clicked() {
                self.agent_link.regenerate();
            }
        });

        ui.add_space(4.);
        ui.text_strong(tr!("agent_link.setup"));
        if let Some(token) = self.agent_link.token.clone() {
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr!("agent_link.copy_command")).clicked() {
                    ui.ctx().copy_text(claude_command(&token, port));
                }
                if ui.button(tr!("agent_link.copy_json")).clicked() {
                    ui.ctx().copy_text(mcp_json(&token, port));
                }
            });
        }
    }
}
