//! Export preferences live in GeneralSettings, captured by InspectorPreferences.
use super::*;
use crate::inspector_table::{Column, InspectorTable};
use benchup_core::tr;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Preset {
    pub id: u64,
    pub name: String,
    pub output: f64,
    pub model: f64,
}

impl Preset {
    fn display_name(&self) -> &str {
        // Localize untouched built-ins at display time. Keep their saved names
        // stable, and leave renamed or user-created presets as entered.
        match (self.id, self.name.as_str()) {
            (7, "Woodworking mockup 1:2") => tr!("export_scale.woodworking_mockup_1_2"),
            (8, "Woodworking mockup 1:5") => tr!("export_scale.woodworking_mockup_1_5"),
            (9, "Woodworking mockup 1:10") => tr!("export_scale.woodworking_mockup_1_10"),
            (10, "Miniature 1:12") => tr!("export_scale.miniature_1_12"),
            _ => &self.name,
        }
    }
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
        ui.text_label(tr!("export_scale.define_ratios"));
        ui.text_weak(tr!("export_scale.actual_size_hint"));
        ui.hyperlink_to(
            tr!("export_scale.nmra_reference"),
            "https://www.nmra.org/beginner/scales",
        );
        ui.text_weak(tr!("export_scale.preset_standards_hint"));
        // Reserve the full single-line button width, including the theme's
        // padding and border, so the cell clip never cuts off its right edge.
        let remove_width = (ui
            .painter()
            .layout_no_wrap(
                tr!("common.remove").into(),
                egui::TextStyle::Button.resolve(ui.style()),
                ui.visuals().text_color(),
            )
            .size()
            .x
            + 2. * ui.spacing().button_padding.x
            + 4.)
            .max(ui.spacing().interact_size.x);
        let mut table = InspectorTable::new(
            ui.available_width(),
            [
                Column::flex(3.),
                Column::flex(1.),
                Column::flex(1.),
                Column::fixed(remove_width),
            ],
        );
        table.header(
            ui,
            [
                tr!("common.name"),
                tr!("export_scale.exported"),
                tr!("app.model"),
                "",
            ],
        );
        let mut remove = None;
        for preset in &mut self.presets {
            table.row(
                ui,
                ("export_scale_preset", preset.id),
                |column, cell| match column {
                    0 => {
                        let mut name = preset.display_name().to_owned();
                        if cell.text_edit_singleline(&mut name).changed() {
                            preset.name = name;
                        }
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
                        if cell
                            .add(
                                egui::Button::new(tr!("common.remove"))
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            )
                            .clicked()
                        {
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
        if ui.button(tr!("export_scale.add_preset")).clicked() {
            let id = self
                .presets
                .iter()
                .map(|p| p.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            self.presets.push(Preset {
                id,
                name: tr!("export_scale.new_scale").into(),
                output: 1.,
                model: 1.,
            });
        }
    }

    fn picker(&mut self, ui: &mut egui::Ui) {
        let label = match self.choice {
            Choice::ActualSize => tr!("export_scale.actual_size").into(),
            Choice::Custom => tr!("export_scale.custom_ratio").into(),
            Choice::Preset(id) => self
                .presets
                .iter()
                .find(|p| p.id == id)
                .map(|p| format!("{} ({}:{})", p.display_name(), p.output, p.model))
                .unwrap_or_default(),
        };
        egui::ComboBox::from_id_salt("stl_export_scale_choice")
            .width(ui.available_width())
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.choice,
                    Choice::ActualSize,
                    tr!("export_scale.actual_size"),
                );
                for preset in &self.presets {
                    ui.selectable_value(
                        &mut self.choice,
                        Choice::Preset(preset.id),
                        format!(
                            "{} ({}:{})",
                            preset.display_name(),
                            preset.output,
                            preset.model
                        ),
                    );
                }
                ui.selectable_value(
                    &mut self.choice,
                    Choice::Custom,
                    tr!("export_scale.custom_ratio"),
                );
            });
        if self.choice == Choice::Custom {
            ui.text_label(tr!("export_scale.ratio_units_hint"));
            let mut table = InspectorTable::new(
                ui.available_width(),
                [Column::flex(1.), Column::fixed(12.), Column::flex(1.)],
            );
            table.header(ui, [tr!("export_scale.exported"), "", tr!("app.model")]);
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
            ui.text_label(tr!(
                "export_scale.scale_preview",
                percent = scale * 100.,
                size = scale * 1000.
            ));
        } else {
            ui.colored_label(egui::Color32::RED, tr!("export_scale.invalid_ratio"));
        }
        ui.text_weak(tr!("export_scale.stl_units_hint"));
        ui.text_weak(tr!("export_scale.manage_presets_hint"));
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
                ui.text_heading(tr!("export_scale.export_stl"));
            });
            modal.body(|ui| self.general_settings.export_scale.picker(ui));
            modal.footer(|ui| {
                if ui.button(tr!("common.cancel")).clicked() {
                    ui.close();
                }
                if primary_button(
                    ui,
                    tr!("app.export"),
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
    use crate::test_ui::ContextExt as _;

    struct ResetLanguage;

    impl Drop for ResetLanguage {
        fn drop(&mut self) {
            benchup_core::i18n::set_language("en");
        }
    }

    fn contains_text(shapes: &[egui::epaint::ClippedShape], expected: &str) -> bool {
        fn contains(shape: &egui::Shape, expected: &str) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.job.text.contains(expected),
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| contains(shape, expected)),
                _ => false,
            }
        }
        shapes.iter().any(|shape| contains(&shape.shape, expected))
    }

    #[test]
    fn localized_preset_names_preserve_saved_and_custom_preferences() {
        let _reset = ResetLanguage;
        let ctx = egui::Context::default();
        let mut prefs = Preferences::default();
        prefs.presets[8].name = "My workshop model".into();
        let mut custom = prefs.presets[6].clone();
        custom.id = 100;
        prefs.presets.push(custom);
        let saved = serde_json::to_string(&prefs).unwrap();
        for (language, mockup) in [
            ("en", "Woodworking mockup 1:2"),
            ("zh_CN", "木工样机 1:2"),
            ("pt_BR", "Maquete de marcenaria 1:2"),
            ("en", "Woodworking mockup 1:2"),
        ] {
            benchup_core::i18n::set_language(language);
            let output = ctx.run_test_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 1200.),
                    )),
                    ..Default::default()
                },
                |ui| prefs.settings_ui(ui),
            );
            assert!(contains_text(&output.shapes, mockup));
            assert!(contains_text(
                &output.shapes,
                tr!("export_scale.add_preset")
            ));
            assert_eq!(prefs.presets[6].display_name(), mockup);
            assert_eq!(prefs.presets[8].display_name(), "My workshop model");
            assert_eq!(
                prefs.presets.last().unwrap().display_name(),
                "Woodworking mockup 1:2"
            );
            assert_eq!(serde_json::to_string(&prefs).unwrap(), saved);
        }
    }

    #[test]
    fn export_dialog_switches_labels_preview_and_validation_without_restart() {
        let _reset = ResetLanguage;
        let ctx = egui::Context::default();
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        app.project.export_scale_open = true;
        for (language, title, actual, custom, preview, error) in [
            (
                "en",
                "Export STL",
                "Actual size (1:1)",
                "Custom ratio",
                "Scale: 50.000000%",
                "Enter two finite numbers greater than zero.",
            ),
            (
                "zh_CN",
                "导出 STL",
                "实际尺寸 (1:1)",
                "自定义比例",
                "比例：50.000000%",
                "请输入两个大于零的有限数值。",
            ),
            (
                "pt_BR",
                "Exportar STL",
                "Tamanho real (1:1)",
                "Proporção personalizada",
                "Escala: 50.000000%",
                "Insira dois números finitos maiores que zero.",
            ),
            (
                "en",
                "Export STL",
                "Actual size (1:1)",
                "Custom ratio",
                "Scale: 50.000000%",
                "Enter two finite numbers greater than zero.",
            ),
        ] {
            benchup_core::i18n::set_language(language);
            let input = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                ..Default::default()
            };
            app.general_settings.export_scale.choice = Choice::ActualSize;
            ctx.run_test_ui(input(), |_| app.export_scale_window(&ctx));
            let output = ctx.run_test_ui(input(), |_| app.export_scale_window(&ctx));
            for text in [title, actual, tr!("common.cancel"), tr!("app.export")] {
                assert!(contains_text(&output.shapes, text), "{language}: {text}");
            }
            app.general_settings.export_scale.choice = Choice::Custom;
            app.general_settings.export_scale.custom_model = "2".into();
            let output = ctx.run_test_ui(input(), |_| app.export_scale_window(&ctx));
            for text in [custom, preview] {
                assert!(contains_text(&output.shapes, text), "{language}: {text}");
            }
            app.general_settings.export_scale.custom_model = "0".into();
            let output = ctx.run_test_ui(input(), |_| app.export_scale_window(&ctx));
            assert!(contains_text(&output.shapes, error), "{language}: {error}");
        }
    }

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
