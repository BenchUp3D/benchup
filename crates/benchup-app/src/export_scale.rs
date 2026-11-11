//! Export preferences live in GeneralSettings, captured by InspectorPreferences.
use super::*;
use crate::inspector_table::{Column, InspectorTable};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Preset {
    pub id: u64,
    pub name: String,
    pub output: f64,
    pub model: f64,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) enum Choice {
    #[default]
    ActualSize,
    Preset(u64),
    Custom,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub presets: Vec<Preset>,
    pub choice: Choice,
    pub custom_output: String,
    pub custom_model: String,
}

impl Default for Preferences {
    fn default() -> Self {
        // NMRA S-1.2 / beginner scale guide. Woodworking mockups are separate
        // conveniences, not NMRA standards: https://www.nmra.org/beginner/scales
        let scales = [
            ("Z (NMRA)", 220.),
            ("N (NMRA)", 160.),
            ("TT (NMRA)", 120.),
            ("HO (NMRA)", 87.1),
            ("S (NMRA)", 64.),
            ("O (NMRA, US)", 48.),
            ("Woodworking mockup 1:2", 2.),
            ("Woodworking mockup 1:5", 5.),
            ("Woodworking mockup 1:10", 10.),
            ("Miniature 1:12", 12.),
        ];
        Self {
            presets: scales
                .into_iter()
                .enumerate()
                .map(|(i, (name, model))| Preset {
                    id: i as u64 + 1,
                    name: name.into(),
                    output: 1.,
                    model,
                })
                .collect(),
            choice: Choice::ActualSize,
            custom_output: "1".into(),
            custom_model: "1".into(),
        }
    }
}

fn factor(output: f64, model: f64) -> Option<f64> {
    let scale = output / model;
    (output.is_finite()
        && model.is_finite()
        && output > 0.
        && model > 0.
        && scale.is_finite()
        && scale > 0.)
        .then_some(scale)
}

impl Preferences {
    pub(super) fn normalize(&mut self) {
        let mut ids = std::collections::BTreeSet::new();
        self.presets
            .retain(|p| ids.insert(p.id) && factor(p.output, p.model).is_some());
        if let Choice::Preset(id) = self.choice {
            if !self.presets.iter().any(|p| p.id == id) {
                self.choice = Choice::ActualSize;
            }
        }
    }

    pub(super) fn factor(&self) -> Option<f64> {
        match self.choice {
            Choice::ActualSize => Some(1.),
            Choice::Preset(id) => self
                .presets
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| factor(p.output, p.model)),
            Choice::Custom => factor(
                self.custom_output.trim().parse().ok()?,
                self.custom_model.trim().parse().ok()?,
            ),
        }
    }

    pub(super) fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.text_label("Define ratios for STL export. Ratio = exported size : model size.");
        ui.text_weak("Actual size (1:1) is always available. Scaling applies to all dimensions, including joints and holes.");
        ui.hyperlink_to(
            "NMRA scale reference (North America)",
            "https://www.nmra.org/beginner/scales",
        );
        ui.text_weak("Woodworking mockup presets are conveniences, not NMRA standards. Railway names can use different ratios in other regions.");
        let mut table = InspectorTable::new(
            ui.available_width(),
            [
                Column::flex(3.),
                Column::flex(1.),
                Column::flex(1.),
                Column::fixed(64.),
            ],
        );
        table.header(ui, ["Name", "Exported", "Model", ""]);
        let mut remove = None;
        for preset in &mut self.presets {
            table.row(
                ui,
                ("export_scale_preset", preset.id),
                |column, cell| match column {
                    0 => {
                        cell.text_edit_singleline(&mut preset.name);
                    }
                    1 => {
                        cell.add(
                            egui::DragValue::new(&mut preset.output)
                                .range(0.000001..=1e6)
                                .speed(0.1),
                        );
                    }
                    2 => {
                        cell.add(
                            egui::DragValue::new(&mut preset.model)
                                .range(0.000001..=1e6)
                                .speed(0.1),
                        );
                    }
                    _ => {
                        if cell.button("Remove").clicked() {
                            remove = Some(preset.id);
                        }
                    }
                },
            );
        }
        if let Some(id) = remove {
            self.presets.retain(|p| p.id != id);
            if self.choice == Choice::Preset(id) {
                self.choice = Choice::ActualSize;
            }
        }
        if ui.button("Add preset").clicked() {
            let id = self
                .presets
                .iter()
                .map(|p| p.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            self.presets.push(Preset {
                id,
                name: "New scale".into(),
                output: 1.,
                model: 1.,
            });
        }
    }

    fn picker(&mut self, ui: &mut egui::Ui) {
        let label = match self.choice {
            Choice::ActualSize => "Actual size (1:1)".into(),
            Choice::Custom => "Custom ratio".into(),
            Choice::Preset(id) => self
                .presets
                .iter()
                .find(|p| p.id == id)
                .map(|p| format!("{} ({}:{})", p.name, p.output, p.model))
                .unwrap_or_default(),
        };
        egui::ComboBox::from_id_salt("stl_export_scale_choice")
            .width(ui.available_width())
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.choice, Choice::ActualSize, "Actual size (1:1)");
                for preset in &self.presets {
                    ui.selectable_value(
                        &mut self.choice,
                        Choice::Preset(preset.id),
                        format!("{} ({}:{})", preset.name, preset.output, preset.model),
                    );
                }
                ui.selectable_value(&mut self.choice, Choice::Custom, "Custom ratio");
            });
        if self.choice == Choice::Custom {
            ui.text_label("Exported size : model size (using the same units)");
            let mut table = InspectorTable::new(
                ui.available_width(),
                [Column::flex(1.), Column::fixed(12.), Column::flex(1.)],
            );
            table.header(ui, ["Exported", "", "Model"]);
            table.row(ui, "custom_export_ratio", |column, cell| match column {
                0 => {
                    cell.text_edit_singleline(&mut self.custom_output);
                }
                1 => {
                    cell.text_label(":");
                }
                _ => {
                    cell.text_edit_singleline(&mut self.custom_model);
                }
            });
        }
        if let Some(scale) = self.factor() {
            ui.text_label(format!(
                "Scale: {:.6}% — 1000 mm in the model exports as {:.4} mm.",
                scale * 100.,
                scale * 1000.
            ));
        } else {
            ui.colored_label(
                egui::Color32::RED,
                "Enter two finite numbers greater than zero.",
            );
        }
        ui.text_weak(
            "STL coordinates are exported in millimetres. The project stays at its original size.",
        );
        ui.text_weak("Manage presets in Settings → Export scales.");
    }
}

impl BenchUpApp {
    pub(super) fn export_scale_window(&mut self, ctx: &egui::Context) {
        if !self.project.export_scale_open {
            return;
        }
        let mut export = false;
        let response = Modal::new("stl_export_scale", 520.).show(ctx, |modal| {
            modal.header(|ui| {
                ui.text_heading("Export STL");
            });
            modal.body(|ui| self.general_settings.export_scale.picker(ui));
            modal.footer(|ui| {
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
                if primary_button(
                    ui,
                    "Export",
                    self.general_settings.export_scale.factor().is_some(),
                )
                .clicked()
                {
                    export = true;
                    ui.close();
                }
            });
        });
        if response.should_close() {
            self.project.export_scale_open = false;
        }
        if export {
            self.project.export_scale_open = false;
            if let Some(scale) = self.general_settings.export_scale.factor() {
                self.export_model_at_scale(ctx, scale);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stl_export_waits_for_scale_confirmation_and_escape_cancels() {
        use crate::test_ui::ContextExt as _;
        let ctx = egui::Context::default();
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        app.project.export_stl = true;
        app.export_model(&ctx);
        assert!(app.project.export_scale_open);
        assert!(app.project.pending.is_none());
        assert!(app.project.error.is_none());
        let _ = ctx.run_test_ui(egui::RawInput::default(), |_| app.export_scale_window(&ctx));
        let _ = ctx.run_test_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |_| app.export_scale_window(&ctx),
        );
        assert!(!app.project.export_scale_open);
        assert!(app.project.pending.is_none());
        assert!(!app.project.modified);
    }

    #[test]
    fn defaults_match_nmra_and_old_preferences_use_actual_size() {
        let prefs: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs.factor(), Some(1.));
        for (name, denominator) in [
            ("Z (NMRA)", 220.),
            ("N (NMRA)", 160.),
            ("HO (NMRA)", 87.1),
            ("O (NMRA, US)", 48.),
        ] {
            let p = prefs.presets.iter().find(|p| p.name == name).unwrap();
            assert_eq!(factor(p.output, p.model), Some(1. / denominator));
        }
    }

    #[test]
    fn custom_ratios_allow_reduction_and_enlargement_and_reject_invalid_values() {
        let mut prefs = Preferences {
            choice: Choice::Custom,
            custom_output: "1".into(),
            custom_model: "160".into(),
            ..Default::default()
        };
        assert_eq!(prefs.factor(), Some(1. / 160.));
        prefs.custom_output = "320".into();
        assert_eq!(prefs.factor(), Some(2.));
        for value in ["0", "-1", "NaN", "inf", "", "abc"] {
            prefs.custom_model = value.into();
            assert_eq!(prefs.factor(), None);
        }
        prefs.choice = Choice::Preset(u64::MAX);
        prefs.normalize();
        assert_eq!(prefs.choice, Choice::ActualSize);
    }
}
