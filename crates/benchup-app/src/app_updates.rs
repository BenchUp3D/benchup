use benchup_core::tr;
use super::*;

impl BenchUpApp {
    pub(super) fn update_window(&mut self, ctx: &egui::Context) {
        if !self.updates.open {
            return;
        }
        let mut restart = false;
        let response = Modal::new("software_updates", 480.).show(ctx, |modal| {
            modal.header(|ui| {
                ui.text_heading(tr!("app_updates.software_updates"));
                ui.text_weak(tr!("app_updates.benchup", env!("CARGO_PKG_VERSION")));
            });
            modal.body(|ui| {
                ui.text_label(self.updates.status());
                if self.updates.busy() {
                    ui.spinner();
                }
                ui.add_space(12.);
                ui.add_enabled_ui(self.updates.available(), |ui| {
                    let mut enabled = self.updates.automatic_updates();
                    if ui.checkbox(&mut enabled, tr!("app_updates.automatically_check_and_download_updates")).changed() {
                        self.updates.set_automatic_updates(enabled, ctx);
                    }
                });
                ui.text_label(RichText::new(tr!("app_updates.automatic_checks_run_at_most_once_a_day")).small());
                if let Some(error) = &self.updates.error {
                    ui.add_space(8.);
                    ui.text_colored_label(ACCENT, error);
                }
                if self.updates.ready() {
                    ui.add_space(8.);
                    ui.text_label(tr!("app_updates.restarting_saves_your_model_locally_and_restores_it"));
                }
            });
            modal.footer(|ui| {
                if ui.button(tr!("common.close")).clicked() { ui.close(); }
                if self.updates.ready() {
                    restart = primary_button(ui, tr!("app_updates.restart_now"), self.can_restart_for_update()).clicked();
                } else if primary_button(ui, tr!("app_updates.check_now"), self.updates.available() && !self.updates.busy()).clicked() {
                    self.updates.check(ctx, false);
                }
                ui.hyperlink_to(tr!("app_updates.release_notes"), "https://github.com/BenchUp3D/editor/releases");
            });
        });
        if response.should_close() {
            self.updates.open = false;
        }
        if restart {
            self.request_project(projects::Action::RestartForUpdate, ctx);
        }
    }

    fn can_restart_for_update(&self) -> bool {
        !self.project.blocked()
            && self.gesture.is_none()
            && self.drawing.is_none()
            && self.line_start.is_none()
            && self.modeling.is_none()
            && self.paste_preview.is_none()
    }

    fn prepare_update_restart(&mut self) -> Result<(), String> {
        if !self.can_restart_for_update() {
            return Err(
                tr!("app_updates.finish_the_current_file_operation_or_modeling_preview").into(),
            );
        }
        if self.storage_recovery {
            return Err(tr!("app_updates.your_previous_workspace_could_not_be_restored_make").into());
        }
        self.persist();
        if let Some(error) = &self.save_error {
            return Err(tr!(
                "app_updates.your_model_could_not_be_saved_benchup_will",
                error = error,
            ));
        }
        crate::updates::Resume::save(&self.doc, self.project.path.clone(), self.project.modified)
            .map_err(|e| tr!("app_updates.could_not_save_your_workspace_for_restart", e = e))
    }

    pub(super) fn restart_for_update(&mut self, ctx: &egui::Context) {
        if !self.updates.ready() || self.updates.busy() {
            return;
        }
        let result = self
            .prepare_update_restart()
            .and_then(|()| self.updates.restart());
        match result {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(error) => {
                // An aborted restart must not associate later work with this
                // snapshot. Resume::take also checks the exact saved document.
                let _ = crate::updates::Resume::take(&self.doc);
                self.updates.error = Some(error);
                self.updates.open = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_is_blocked_during_file_operations_and_unresolved_recovery() {
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        let (_sender, receiver) = std::sync::mpsc::channel();
        app.project.pending = Some(receiver);
        assert!(app.prepare_update_restart().is_err());
        app.project.pending = None;
        app.line_start = Some(DVec3::ZERO);
        assert!(app.prepare_update_restart().is_err());
        app.line_start = None;
        app.storage_recovery = true;
        assert!(app.prepare_update_restart().is_err());
        assert!(!app.updates.restarting);
    }

    #[test]
    fn failed_autosave_keeps_the_editor_open_and_preserves_the_model() {
        const CHILD: &str = "BENCHUP_UPDATE_SAVE_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let directory = tempfile::tempdir().unwrap();
            let blocked = directory.path().join("blocked");
            std::fs::write(&blocked, b"not a directory").unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "app::app_updates::tests::failed_autosave_keeps_the_editor_open_and_preserves_the_model", "--nocapture"])
                .env(CHILD, "1")
                .env("BENCHUP_DATA_DIR", blocked)
                .output().unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        assert!(std::env::var_os("BENCHUP_DATA_DIR").is_some());
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        app.doc.model.add_edge(DVec3::ZERO, DVec3::X).unwrap();
        let model = app.doc.model.clone();
        assert!(app.prepare_update_restart().is_err());
        assert!(app.save_error.is_some());
        assert_eq!(app.doc.model, model);
        assert!(!app.updates.restarting);
    }
}
