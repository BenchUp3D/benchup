use benchup_core::tr;
use super::shortcut_input::{shortcut_input, valid_tool_key};
use super::*;
use crate::inspector_table::{Column, InspectorTable, inspector_field};
#[cfg(test)]
use crate::test_ui::ContextExt as _;
use benchup_core::LengthUnit;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum Section {
    #[default]
    Colors,
    Navigation,
    Shortcuts,
    Units,
    Grid,
    StockPricing,
    SurfaceTreatment,
    Diagnostics,
    Rendering,
    Tools,
    ExportScales,
}

impl Section {
    fn groups() -> [(&'static str, &'static [Self]); 4] {
        [
            (
                tr!("settings.workspace"),
                &[Self::Units, Self::Grid, Self::Colors],
            ),
            (
                tr!("settings.controls"),
                &[Self::Navigation, Self::Shortcuts, Self::Tools],
            ),
            (
                tr!("settings.materials_estimates"),
                &[Self::StockPricing, Self::SurfaceTreatment],
            ),
            (
                tr!("settings.system"),
                &[Self::Diagnostics, Self::Rendering, Self::ExportScales],
            ),
        ]
    }
    #[cfg(test)]
    const ALL: [Self; 11] = [
        Self::Units,
        Self::Navigation,
        Self::Shortcuts,
        Self::Grid,
        Self::Colors,
        Self::StockPricing,
        Self::SurfaceTreatment,
        Self::Diagnostics,
        Self::Rendering,
        Self::Tools,
        Self::ExportScales,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Colors => tr!("settings.axis_and_inference_colors"),
            Self::Navigation => tr!("settings.navigation"),
            Self::Shortcuts => tr!("settings.shortcuts_management"),
            Self::Units => tr!("settings.default_units"),
            Self::Grid => tr!("settings.grid"),
            Self::StockPricing => tr!("settings.material"),
            Self::SurfaceTreatment => tr!("settings.sanding_finishing"),
            Self::Diagnostics => tr!("settings.diagnostics"),
            Self::Rendering => tr!("settings.rendering"),
            Self::Tools => tr!("settings.tools"),
            Self::ExportScales => tr!("settings.export_scales"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Colors => "settings_colors",
            Self::Navigation => "settings_navigation",
            Self::Shortcuts => "settings_shortcuts",
            Self::Units => "units",
            Self::Grid => "settings_grid",
            Self::StockPricing => "settings_stock",
            Self::SurfaceTreatment => "time_estimates_sanding",
            Self::Diagnostics => "diagnostics",
            Self::Rendering => "settings_colors",
            Self::Tools => "settings_shortcuts",
            Self::ExportScales => "units",
        }
    }

    fn button(self, ui: &mut egui::Ui, selected: bool) -> egui::Response {
        let colors = Palette::for_ui(ui);
        let color = if selected { colors.accent } else { colors.ink };
        let width = ui.available_width();
        let label = ui.painter().layout(
            self.label().to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            color,
            (width - 44.).max(1.),
        );
        let height = (label.size().y + 16.).max(40.);
        let response = ui.add_sized([width, height], egui::Button::new("").selected(selected));
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::RadioButton,
                ui.is_enabled(),
                selected,
                self.label(),
            )
        });
        if ui.is_rect_visible(response.rect) {
            icons::line_icon(
                ui.painter(),
                egui::pos2(response.rect.left() + 17., response.rect.center().y),
                self.icon(),
                color,
            );
            ui.painter().galley(
                egui::pos2(
                    response.rect.left() + 34.,
                    response.rect.center().y - label.size().y * 0.5,
                ),
                label,
                color,
            );
        }
        response
    }
}

pub(super) const TOOLS: [(Tool, &str); 18] = [
    (Tool::Select, "Space"),
    (Tool::Line, "L"),
    (Tool::Rectangle, "R"),
    (Tool::Circle, "C"),
    (Tool::Polygon, "G"),
    (Tool::Freehand, "D"),
    (Tool::Erase, "E"),
    (Tool::Orbit, "O"),
    (Tool::PushPull, "P"),
    (Tool::Scale, "S"),
    (Tool::Offset, "F"),
    (Tool::Move, "M"),
    (Tool::Paint, "B"),
    (Tool::TapeMeasure, "T"),
    (Tool::Protractor, "A"),
    (Tool::Dimensions, "I"),
    // Append to preserve the indices of saved custom shortcuts.
    (Tool::Rotate, "Q"),
    (Tool::Image, "J"),
];

const DEFAULT_LANGUAGE: &str = "en_US";
#[path = "language.rs"]
mod language;

pub(super) fn detected_language() -> &'static str {
    language::detected_language()
}

fn languages() -> [(&'static str, &'static str); 6] {
    [
        (DEFAULT_LANGUAGE, tr!("settings.language.english")),
        ("zh_CN", tr!("settings.language.simplified_chinese")),
        ("pt_BR", tr!("settings.language.brazilian_portuguese")),
        ("vi_VN", tr!("settings.language.vietnamese")),
        ("es_419", tr!("settings.language.spanish")),
        ("it_IT", tr!("settings.language.italian")),
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) enum NavigationType {
    #[default]
    MouseTrackpad,
    SpaceMouse,
}

impl NavigationType {
    fn label(self) -> &'static str {
        match self {
            Self::MouseTrackpad => tr!("settings.mouse_trackpad"),
            Self::SpaceMouse => tr!("settings.3d_mouse"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct GeneralSettings {
    pub section: Section,
    pub language: String,
    pub navigation_type: NavigationType,
    pub middle_mouse_pan: bool,
    pub orbit_speed: f64,
    pub pan_speed: f64,
    pub zoom_speed: f64,
    pub reverse_zoom: bool,
    pub invert_orbit: bool,
    pub invert_pan: bool,
    pub walk: walk::Settings,
    pub space_mouse: space_mouse::Settings,
    pub reset_axis_locks_on_action: bool,
    pub auto_group_new_shapes: bool,
    pub show_fps: bool,
    pub default_units: benchup_core::UnitSettings,
    pub tool_keys: Vec<String>,
    pub stock: stock_settings::Settings,
    pub export_scale: export_scale::Preferences,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            section: Section::default(),
            language: DEFAULT_LANGUAGE.into(),
            navigation_type: NavigationType::default(),
            middle_mouse_pan: false,
            orbit_speed: 1.,
            pan_speed: 1.,
            zoom_speed: 1.,
            reverse_zoom: false,
            invert_orbit: false,
            invert_pan: false,
            walk: walk::Settings::default(),
            space_mouse: space_mouse::Settings::default(),
            reset_axis_locks_on_action: true,
            auto_group_new_shapes: false,
            show_fps: false,
            default_units: Default::default(),
            stock: Default::default(),
            export_scale: Default::default(),
            tool_keys: TOOLS.iter().map(|(_, key)| key.to_string()).collect(),
        }
    }
}

impl GeneralSettings {
    pub(super) fn navigation_pans(&self, button: Option<egui::PointerButton>, shift: bool) -> bool {
        // Shift switches modes during a drag; only middle-button navigation
        // uses the alternate mapping. Orbit-tool and drawing controls stay fixed.
        shift ^ (button == Some(egui::PointerButton::Middle) && self.middle_mouse_pan)
    }

    // Keep the existing serialized fields and input-device bindings independent.
    // The selector only chooses which device's settings are being edited.
    fn navigation_speeds_mut(&mut self) -> [&mut f64; 3] {
        match self.navigation_type {
            NavigationType::MouseTrackpad => [
                &mut self.orbit_speed,
                &mut self.pan_speed,
                &mut self.zoom_speed,
            ],
            NavigationType::SpaceMouse => [
                &mut self.space_mouse.orbit_speed,
                &mut self.space_mouse.pan_speed,
                &mut self.space_mouse.zoom_speed,
            ],
        }
    }

    fn navigation_inversions_mut(&mut self) -> [&mut bool; 3] {
        match self.navigation_type {
            NavigationType::MouseTrackpad => [
                &mut self.invert_orbit,
                &mut self.invert_pan,
                // Preserve the existing saved reverse-scroll preference.
                &mut self.reverse_zoom,
            ],
            NavigationType::SpaceMouse => [
                &mut self.space_mouse.invert_orbit,
                &mut self.space_mouse.invert_pan,
                &mut self.space_mouse.invert_zoom,
            ],
        }
    }

    pub(super) fn mouse_orbit_speed(&self) -> f64 {
        self.orbit_speed * if self.invert_orbit { -1. } else { 1. }
    }

    pub(super) fn mouse_pan_speed(&self) -> f64 {
        self.pan_speed * if self.invert_pan { -1. } else { 1. }
    }

    pub(super) fn mouse_zoom_speed(&self) -> f64 {
        self.zoom_speed * if self.reverse_zoom { -1. } else { 1. }
    }

    pub(super) fn normalize(&mut self) {
        self.walk.normalize();
        self.space_mouse.normalize();
        self.stock.normalize();
        self.export_scale.normalize();
        // Earlier builds saved bare codes; every catalog now uses language_REGION.
        match self.language.as_str() {
            "en" => self.language = "en_US".into(),
            "vi" => self.language = "vi_VN".into(),
            "es" => self.language = "es_419".into(),
            "it" => self.language = "it_IT".into(),
            _ => {}
        }
        if !languages().iter().any(|&(code, _)| code == self.language.as_str()) {
            self.language = DEFAULT_LANGUAGE.into();
        }
        for speed in [
            &mut self.orbit_speed,
            &mut self.pan_speed,
            &mut self.zoom_speed,
        ] {
            *speed = if speed.is_finite() {
                speed.clamp(0.1, 4.)
            } else {
                1.
            };
        }
        // Migrate older shortcut lists without discarding custom assignments.
        if (16..TOOLS.len()).contains(&self.tool_keys.len()) {
            while self.tool_keys.len() < TOOLS.len() {
                let preferred = TOOLS[self.tool_keys.len()].1;
                let key = std::iter::once(preferred)
                    .chain(["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z"])
                    .find(|key| !self.tool_keys.iter().any(|saved| saved.as_str() == *key))
                    .unwrap();
                self.tool_keys.push(key.into());
            }
        }
        if self.tool_keys.len() != TOOLS.len()
            || self
                .tool_keys
                .iter()
                .enumerate()
                .any(|(i, key)| !valid_tool_key(key) || self.tool_keys[..i].contains(key))
        {
            self.tool_keys = Self::default().tool_keys;
        }
    }

    pub(super) fn tool_for_key(&self, key: Key) -> Option<Tool> {
        self.tool_keys
            .iter()
            .position(|name| Key::from_name(name) == Some(key))
            .map(|index| TOOLS[index].0)
    }

    pub(super) fn tool_shortcut(&self, tool: Tool) -> &str {
        TOOLS
            .iter()
            .position(|(candidate, _)| *candidate == tool)
            .map(|index| self.tool_keys[index].as_str())
            .unwrap_or(tool.shortcut())
    }

    fn assign_key(&mut self, index: usize, key: String) {
        if let Some(other) = self.tool_keys.iter().position(|value| *value == key) {
            self.tool_keys.swap(index, other);
        } else {
            self.tool_keys[index] = key;
        }
    }
}

impl BenchUpApp {
    pub(super) fn axis_color(&self, axis: Axis) -> Color32 {
        let index = match axis {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        };
        let [r, g, b] = self.doc.display.axis_colors[index];
        Color32::from_rgb(r, g, b)
    }

    pub(super) fn inference_color(&self) -> Color32 {
        let [r, g, b] = self.doc.display.inference_color;
        Color32::from_rgb(r, g, b)
    }

    fn navigation_settings_contents(&mut self, ui: &mut egui::Ui) {
        let navigation_type = &mut self.general_settings.navigation_type;
        inspector_field(ui, "settings/navigation/type", tr!("settings.navigation_type"), |ui| {
            egui::ComboBox::from_id_salt("settings_navigation_type")
                .width(ui.available_width())
                .selected_text(navigation_type.label())
                .show_ui(ui, |ui| {
                    for kind in [NavigationType::MouseTrackpad, NavigationType::SpaceMouse] {
                        ui.selectable_value(navigation_type, kind, kind.label());
                    }
                })
                .response
        });
        ui.text_weak(tr!("settings.adjust_navigation_sensitivity_for_the_selected_device"));
        let navigation_type = self.general_settings.navigation_type;
        if navigation_type == NavigationType::MouseTrackpad {
            inspector_field(
                ui,
                "settings/navigation/middle_mouse",
                tr!("settings.middle_mouse_drag"),
                |ui| {
                    egui::ComboBox::from_id_salt("settings_middle_mouse_drag")
                        .width(ui.available_width())
                        .selected_text(if self.general_settings.middle_mouse_pan {
                            tr!("settings.pan_shift_to_orbit")
                        } else {
                            tr!("settings.orbit_shift_to_pan")
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.general_settings.middle_mouse_pan,
                                false,
                                tr!("settings.orbit_shift_to_pan"),
                            );
                            ui.selectable_value(
                                &mut self.general_settings.middle_mouse_pan,
                                true,
                                tr!("settings.pan_shift_to_orbit"),
                            );
                        })
                        .response
                },
            );
        }
        if navigation_type == NavigationType::SpaceMouse {
            let space_mouse = &mut self.general_settings.space_mouse;
            ui.checkbox(&mut space_mouse.enabled, tr!("settings.enable_3d_mouse"));
            #[cfg(target_arch = "wasm32")]
            ui.add_enabled_ui(space_mouse.enabled, |ui| {
                if ui
                    .add_enabled(
                        self.space_mouse.browser_can_connect(),
                        egui::Button::new(tr!("settings.connect_3d_mouse")),
                    )
                    .clicked()
                {
                    self.space_mouse.connect_browser();
                }
            });
            let status = if !space_mouse.enabled {
                tr!("settings.disabled").to_owned()
            } else {
                #[cfg(target_arch = "wasm32")]
                {
                    self.space_mouse.browser_status()
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    match self.space_mouse.device() {
                        Some(device) => tr!("settings.connected", device = device),
                        None => tr!("settings.no_device_detected").to_owned(),
                    }
                }
            };
            ui.text_weak(status);
            ui.text_weak(tr!("settings.tilt_or_twist_the_cap_to_orbit_slide_it_to_pan_and_push_or_pull_it_to_zoom_the_fit_button_frames_the_model"));
        }
        let enabled = navigation_type == NavigationType::MouseTrackpad
            || self.general_settings.space_mouse.enabled;
        ui.add_enabled_ui(enabled, |ui| {
            ui.push_id(("settings/navigation/inversions", navigation_type), |ui| {
                for (label, value) in [tr!("settings.invert_tilt_and_twist"), tr!("settings.invert_pan"), tr!("settings.invert_zoom")]
                    .into_iter()
                    .zip(self.general_settings.navigation_inversions_mut())
                {
                    ui.checkbox(value, label);
                }
            });
            // Separate widget state prevents an in-progress slider edit from
            // carrying over when the selected device changes.
            ui.push_id(("settings/navigation/speeds", navigation_type), |ui| {
                for ((key, label), value) in [
                    ("orbit", tr!("settings.orbit_speed")),
                    ("pan", tr!("settings.pan_speed")),
                    ("zoom", tr!("settings.zoom_speed")),
                ]
                .into_iter()
                .zip(self.general_settings.navigation_speeds_mut())
                {
                    inspector_field(ui, key, label, |ui| {
                        ui.add(egui::Slider::new(value, 0.1..=4.).suffix(tr!("settings.format")))
                    });
                }
            });
        });
    }

    pub(super) fn settings_window(&mut self, ctx: &egui::Context) {
        benchup_core::i18n::set_language(&self.general_settings.language);
        let response = Modal::new("general_settings", 950.)
            .resizable(true)
            .dismissible(self.material_removal.is_none())
            .show(ctx, |modal| {
                modal.header(|ui| {
                    ui.horizontal(|ui| {
                        ui.text_heading(tr!("settings.settings"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let language = &mut self.general_settings.language;
                            let languages = languages();
                            let selected = languages
                                .iter()
                                .find(|&&(code, _)| code == language.as_str())
                                .unwrap_or(&languages[0]);
                            let picker = egui::ComboBox::from_id_salt("settings_language")
                                .selected_text(selected.1)
                                .show_ui(ui, |ui| {
                                    for &(code, name) in &languages {
                                        if ui
                                            .selectable_label(code == language.as_str(), name)
                                            .clicked()
                                        {
                                            *language = code.into();
                                            benchup_core::i18n::set_language(language);
                                            ui.ctx().request_repaint();
                                        }
                                    }
                                });
                            let label = ui.text_label(tr!("settings.language"));
                            picker.response.labelled_by(label.id);
                        });
                    });
                    ui.text_weak(tr!("settings.changes_are_saved_automatically_on_this_device"));
                });
                modal.body_unpadded(650., |ui| {
                    let size = ui.available_size();
                    let sidebar_width = (size.x * 0.29).clamp(100., 220.);
                    ui.horizontal_top(|ui| {
                        // Frames inherit their parent's layout. Give both columns an
                        // explicit vertical layout instead of inheriting this row.
                        ui.allocate_ui_with_layout(
                            egui::vec2(sidebar_width, size.y),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::Frame::new()
                                    .fill(Palette::for_ui(ui).canvas)
                                    .inner_margin(14)
                                    .show(ui, |ui| {
                                        ui.set_width((sidebar_width - 28.).max(1.));
                                        ui.set_min_height((size.y - 28.).max(24.));
                                        ui.spacing_mut().item_spacing.y = 8.;
                                        for (index, (title, sections)) in
                                            Section::groups().iter().enumerate()
                                        {
                                            if index > 0 {
                                                ui.add_space(12.);
                                            }
                                            ui.text_label(
                                                RichText::new(*title)
                                                    .small()
                                                    .strong()
                                                    .color(Palette::for_ui(ui).muted),
                                            );
                                            for &section in *sections {
                                                if section
                                                    .button(
                                                        ui,
                                                        self.general_settings.section == section,
                                                    )
                                                    .clicked()
                                                {
                                                    self.general_settings.section = section;
                                                }
                                            }
                                        }
                                    });
                            },
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), size.y),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let key = match self.general_settings.section {
                                    Section::Colors => "settings/colors",
                                    Section::Navigation => "settings/navigation",
                                    Section::Shortcuts => "settings/shortcuts",
                                    Section::Units => "settings/units",
                                    Section::Grid => "settings/grid",
                                    Section::StockPricing => "settings/stock",
                                    Section::SurfaceTreatment => "settings/treatment",
                                    Section::Diagnostics => "settings/diagnostics",
                                    Section::Rendering => "settings/rendering",
                                    Section::Tools => "settings/tools",
                                    Section::ExportScales => "settings/export_scales",
                                };
                                let scroll = sidebar::restored_scroll_area(
                                    ui,
                                    key,
                                    egui::ScrollArea::vertical()
                                        .id_salt(key)
                                        .auto_shrink([false, false]),
                                );
                                let output = scroll.show(ui, |ui| {
                                    // Keep padding inside the viewport so the scrollbar
                                    // stays against the settings panel edge.
                                    egui::Frame::new().inner_margin(20).show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.spacing_mut().item_spacing = egui::vec2(8., 12.);
                                        ui.horizontal(|ui| {
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(22., 22.),
                                                Sense::hover(),
                                            );
                                            icons::line_icon(
                                                ui.painter(),
                                                rect.center(),
                                                self.general_settings.section.icon(),
                                                Palette::for_ui(ui).ink,
                                            );
                                            ui.text_heading(self.general_settings.section.label());
                                        });
                                        ui.add_space(8.);
                                        self.settings_contents(ui);
                                    });
                                });
                                sidebar::remember_scroll(
                                    ui.ctx(),
                                    key,
                                    output.state.clamped_offset(),
                                );
                            },
                        );
                    });
                });
                modal.footer(|ui| {
                    if primary_button(ui, tr!("common.close"), true).clicked() {
                        ui.close();
                    }
                });
            });
        if response.should_close() {
            self.settings_open = false;
        }
    }

    fn settings_contents(&mut self, ui: &mut egui::Ui) {
        match self.general_settings.section {
            Section::ExportScales => self.general_settings.export_scale.settings_ui(ui),
            Section::Tools => {
                ui.checkbox(
                    &mut self.general_settings.reset_axis_locks_on_action,
                    tr!("settings.reset_axis_locks_on_each_action"),
                )
                .on_hover_text(tr!("settings.clear_both_axis_locks_after_completing_an_edit"));
                ui.checkbox(
                    &mut self.general_settings.auto_group_new_shapes,
                    tr!("settings.automatically_group_new_shapes"),
                )
                .on_hover_text(tr!("settings.group_new_shapes_outside_existing_objects"));
            }
            Section::Rendering => {
                ui.checkbox(&mut self.doc.display.shadow_antialiasing, tr!("settings.shadow_antialiasing"))
                    .on_hover_text(tr!("settings.smooth_shadow_boundaries_with_4_multisampling_only_shadows"));
            }
            Section::Diagnostics => {
                ui.text_weak(tr!("settings.tools_for_checking_performance_and_troubleshooting"));
                ui.checkbox(&mut self.general_settings.show_fps, tr!("settings.show_frames_per_second"))
                    .on_hover_text(tr!("settings.display_the_rendering_frame_rate_in_the_lower"));
            }
            Section::SurfaceTreatment => {
                ui.text_weak(tr!("settings.defaults_for_objects_in_all_projects_objects_use"));
                time_estimates::treatment_controls(
                    ui,
                    &mut self.time_estimates,
                    true,
                    &self.general_settings.stock.finishes,
                    &mut self.finish_product_filters,
                );
            }
            Section::StockPricing => {
                if let Some(id) = self
                    .general_settings
                    .stock
                    .show(ui, &mut self.wood_type_filters)
                {
                    self.material_removal = Some(id);
                }
            }
            Section::Colors => {
                let d = &mut self.doc.display;
                color_row(ui, tr!("settings.x_axis"), &mut d.axis_colors[0]);
                color_row(ui, tr!("settings.y_axis"), &mut d.axis_colors[1]);
                color_row(ui, tr!("settings.z_axis"), &mut d.axis_colors[2]);
                color_row(ui, tr!("settings.inference_points"), &mut d.inference_color);
                color_row(ui, tr!("settings.midpoints"), &mut d.midpoint_color);
                ui.text_weak(tr!("settings.axis_inferences_use_the_corresponding_axis_color"));
                if ui.button(tr!("settings.restore_default_colors")).clicked() {
                    let defaults = benchup_core::DisplaySettings::default();
                    d.axis_colors = defaults.axis_colors;
                    d.inference_color = defaults.inference_color;
                    d.midpoint_color = defaults.midpoint_color;
                }
            }
            Section::Navigation => {
                self.navigation_settings_contents(ui);
                ui.separator();
                ui.text_label(RichText::new(tr!("settings.walking_person")).strong());
                inspector_field(ui, "walk_speed", tr!("settings.walking_speed"), |ui| {
                    ui.add(egui::DragValue::new(&mut self.general_settings.walk.speed_kmh)
                        .speed(0.1).range(0.5..=12.).suffix(" km/h"))
                });
                inspector_field(ui, "walk_look_sensitivity", tr!("settings.look_sensitivity"), |ui| {
                    let mut percent = self.general_settings.walk.look_sensitivity * 100.;
                    let response = ui.add(egui::DragValue::new(&mut percent)
                        .speed(1.).range(10. ..=200.).suffix(" %"))
                        .on_hover_text(tr!("settings.scales_mouse_look_while_walking_100_matches_orbit_sensitivity"));
                    if response.changed() {
                        self.general_settings.walk.look_sensitivity = percent / 100.;
                    }
                    response
                });
                ui.text_weak(tr!("settings.eye_height_is_measured_above_the_floor_body_width_and_depth_determine_obstacle_clearance_allow_12_cm_above_the_eyes_for_the_head"));
                let unit = self.doc.display.units.length;
                for (key, label, value, min, max) in [
                    ("walk_eye_height", tr!("settings.eye_height"), &mut self.general_settings.walk.eye_height, 0.5, 2.3),
                    ("walk_width", tr!("settings.body_width"), &mut self.general_settings.walk.width, 0.2, 1.2),
                    ("walk_depth", tr!("settings.body_depth_length"), &mut self.general_settings.walk.depth, 0.15, 1.2),
                ] {
                    inspector_field(ui, key, label, |ui| {
                        let mut displayed = *value / unit.factor();
                        let response = ui.add(egui::DragValue::new(&mut displayed)
                            .speed(0.01 / unit.factor())
                            .range(min / unit.factor()..=max / unit.factor())
                            .suffix(format!(" {}", unit.symbol())));
                        if response.changed() {
                            *value = displayed * unit.factor();
                        }
                        response
                    });
                }
                ui.text_weak(tr!("settings.wasd_arrows_walk_shift_crouch_space_jump_alt_sit_stand_move_mouse_look_escape_exit"));
            }
            Section::Shortcuts => {
                ui.text_weak(tr!("settings.click_a_shortcut_then_press_a_letter_or"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.shortcut_filter)
                        .desired_width(ui.available_width())
                        .hint_text(tr!("settings.search_tools")),
                );
                let query = self.shortcut_filter.to_lowercase();
                let mut table = InspectorTable::new(
                    ui.available_width(),
                    [Column::flex(0.55), Column::flex(0.45)],
                )
                .centered()
                .striped();
                table.header(ui, [tr!("settings.tool"), tr!("settings.shortcut")]);
                let mut matches = 0;
                for (index, (tool, _)) in TOOLS.iter().enumerate() {
                    if !tool.label().to_lowercase().contains(&query) {
                        continue;
                    }
                    matches += 1;
                    table.row(ui, ("settings_tool", index), |column, ui| {
                        if column == 0 {
                            ui.text_label(tool.label());
                        } else if let Some(key) = shortcut_input(
                            ui,
                            ("settings_tool_key", index),
                            &self.general_settings.tool_keys[index],
                            ui.available_width(),
                        ) {
                            self.general_settings.assign_key(index, key);
                        }
                    });
                }
                if matches == 0 {
                    ui.text_weak(tr!("settings.no_tools_match_your_search"));
                }
                if ui.button(tr!("settings.restore_default_shortcuts")).clicked() {
                    self.general_settings.tool_keys = GeneralSettings::default().tool_keys;
                }
            }
            Section::Units => {
                ui.text_weak(tr!("settings.used_for_new_projects_change_the_current_project"));
                Self::unit_settings_contents(ui, &mut self.general_settings.default_units);
            }
            Section::Grid => {
                inspector_field(ui, "settings/grid/show", tr!("settings.show_grid"), |ui| {
                    ui.checkbox(&mut self.doc.display.grid, "")
                });
                inspector_field(ui, "settings/grid/snap", tr!("settings.snap_to_grid"), |ui| {
                    ui.checkbox(&mut self.snap_to_grid, "")
                });
                inspector_field(ui, "settings/grid/infinite", tr!("settings.infinite_grid"), |ui| {
                    ui.checkbox(&mut self.doc.display.grid_infinite, "")
                });
                let unit = self.doc.display.units.length;
                if !self.doc.display.grid_infinite {
                    grid_length_control(
                        ui,
                        "settings/grid/max_length",
                        tr!("settings.maximum_length"),
                        &mut self.doc.display.grid_max_length,
                        unit,
                    );
                    ui.text_weak(tr!("settings.length_of_each_side_of_the_square_centered"));
                }
                inspector_field(
                    ui,
                    "settings/grid/auto_size",
                    tr!("settings.automatically_size_grid_when_zooming"),
                    |ui| ui.checkbox(&mut self.doc.display.grid_auto_size, ""),
                );
                ui.add_enabled_ui(!self.doc.display.grid_auto_size, |ui| {
                    grid_length_control(
                        ui,
                        "settings/grid/spacing",
                        tr!("settings.grid_spacing"),
                        &mut self.doc.display.grid_spacing,
                        unit,
                    );
                });
                ui.text_weak(tr!("settings.spacing_controls_both_grid_lines_and_grid_snapping"));
            }
        }
    }
}

fn grid_length_control(
    ui: &mut egui::Ui,
    key: &'static str,
    label: &str,
    meters: &mut f64,
    unit: LengthUnit,
) -> egui::Response {
    let mut value = *meters / unit.factor();
    let response = inspector_field(ui, key, label, |ui| {
        ui.add_sized(
            [ui.available_width(), 0.],
            egui::DragValue::new(&mut value)
                .range((0.000001 / unit.factor())..=(1e6 / unit.factor()))
                .speed(0.1)
                .suffix(format!(" {}", unit.symbol())),
        )
    });
    if response.changed() && value.is_finite() {
        *meters = (value * unit.factor()).clamp(0.000001, 1e6);
    }
    response
}

fn color_row(ui: &mut egui::Ui, label: &str, color: &mut [u8; 3]) {
    ui.horizontal(|ui| {
        ui.color_edit_button_srgb(color);
        ui.text_label(label);
    });
}

#[cfg(test)]
mod tests {
    fn rendered_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(value) => text.push(value.galley.job.text.clone()),
                egui::Shape::Vec(values) => {
                    for value in values {
                        collect(value, text);
                    }
                }
                _ => {}
            }
        }
        let mut text = Vec::new();
        for shape in shapes {
            collect(&shape.shape, &mut text);
        }
        text
    }

    #[test]
    fn grid_length_field_converts_project_units_on_display_and_input() {
        fn frame(
            ctx: &egui::Context,
            time: &mut f64,
            length: &mut f64,
            unit: LengthUnit,
            events: Vec<egui::Event>,
        ) -> (Rect, Vec<String>) {
            *time += 0.02;
            let mut rect = Rect::NOTHING;
            let mut output = ctx.run_test_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 300.))),
                    time: Some(*time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    rect =
                        grid_length_control(ui, "test_grid_length", "Maximum length", length, unit)
                            .rect;
                },
            );
            let text = rendered_text(&output.shapes);
            output.textures_delta.clear();
            (rect, text)
        }
        for unit in [
            LengthUnit::Meters,
            LengthUnit::Feet,
            LengthUnit::Inches,
            LengthUnit::Millimeters,
        ] {
            let ctx = egui::Context::default();
            let mut time = 0.;
            let mut length = 3.048;
            let (rect, text) = frame(&ctx, &mut time, &mut length, unit, vec![]);
            assert!(
                text.iter()
                    .any(|s| s.ends_with(&format!(" {}", unit.symbol())))
            );
            assert_eq!(
                length, 3.048,
                "Changing display units changed the physical length"
            );
            let pos = rect.center();
            for pressed in [true, false] {
                frame(
                    &ctx,
                    &mut time,
                    &mut length,
                    unit,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            let command = egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            };
            frame(
                &ctx,
                &mut time,
                &mut length,
                unit,
                vec![
                    egui::Event::ModifiersChanged(command),
                    egui::Event::Key {
                        key: Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: command,
                    },
                    egui::Event::Text("12".into()),
                ],
            );
            frame(
                &ctx,
                &mut time,
                &mut length,
                unit,
                vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(
                (length - 12. * unit.factor()).abs() < 1e-9,
                "Input did not convert {:?}: {length}",
                unit
            );
            let physical_length = length;
            for changed_unit in [
                LengthUnit::Meters,
                LengthUnit::Feet,
                LengthUnit::Millimeters,
            ] {
                frame(&ctx, &mut time, &mut length, changed_unit, vec![]);
                assert_eq!(length, physical_length);
            }
        }
    }

    #[test]
    fn finite_grid_length_control_only_appears_when_infinite_is_disabled() {
        for infinite in [true, false] {
            let ctx = egui::Context::default();
            let mut app = BenchUpApp::with_document(Document::default(), None, true);
            app.general_settings.section = Section::Grid;
            app.doc.display.grid_infinite = infinite;
            let mut output =
                ctx.run_test_ui(egui::RawInput::default(), |ui| app.settings_contents(ui));
            let text = rendered_text(&output.shapes);
            output.textures_delta.clear();
            assert!(text.iter().any(|s| s == "Infinite grid"));
            assert_eq!(text.iter().any(|s| s == "Maximum length"), !infinite);
        }
    }

    #[test]
    fn settings_groups_include_every_section_once() {
        let grouped: Vec<_> = super::Section::groups()
            .iter()
            .flat_map(|(_, sections)| sections.iter().copied())
            .collect();
        assert_eq!(grouped.len(), super::Section::ALL.len());
        for section in super::Section::ALL {
            assert_eq!(grouped.iter().filter(|&&value| value == section).count(), 1);
        }
    }
    use super::*;

    #[test]
    fn adding_rotate_migrates_existing_custom_keys_and_resolves_conflicts() {
        for custom in ["H", "Q"] {
            let mut settings = GeneralSettings::default();
            settings.tool_keys.truncate(16);
            settings.tool_keys[1] = custom.into();
            let old = settings.tool_keys.clone();
            settings.normalize();
            assert_eq!(&settings.tool_keys[..old.len()], old.as_slice());
            assert_eq!(settings.tool_keys.len(), TOOLS.len());
            assert_eq!(settings.tool_for_key(Key::from_name(custom).unwrap()), Some(Tool::Line));
            let rotation = settings.tool_shortcut(Tool::Rotate);
            assert_ne!(rotation, custom);
            assert_eq!(settings.tool_for_key(Key::from_name(rotation).unwrap()), Some(Tool::Rotate));
        }
    }

    #[test]
    fn adding_image_keeps_custom_shortcuts_and_picks_an_unused_key() {
        let mut settings = GeneralSettings::default();
        settings.tool_keys.truncate(17);
        settings.tool_keys[1] = "J".into();
        let original = settings.tool_keys.clone();
        settings.normalize();
        assert_eq!(&settings.tool_keys[..17], &original);
        assert_eq!(settings.tool_for_key(Key::J), Some(Tool::Line));
        assert_ne!(settings.tool_shortcut(Tool::Image), "J");
        assert_eq!(settings.tool_shortcut(Tool::Rotate), "Q");
    }

    #[test]
    fn reassigning_shortcuts_swaps_conflicts_and_repairs_invalid_saved_values() {
        let mut settings = GeneralSettings::default();
        settings.assign_key(1, "R".into());
        assert_eq!(settings.tool_for_key(Key::R), Some(Tool::Line));
        assert_eq!(settings.tool_for_key(Key::L), Some(Tool::Rectangle));
        settings.normalize();
        assert_eq!(settings.tool_for_key(Key::R), Some(Tool::Line));
        settings.tool_keys[0] = "Escape".into();
        settings.orbit_speed = f64::NAN;
        settings.normalize();
        assert_eq!(settings.tool_keys, GeneralSettings::default().tool_keys);
        assert_eq!(settings.orbit_speed, 1.);
        let old: GeneralSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old, GeneralSettings::default());
    }

    #[test]
    fn navigation_controls_edit_separate_device_values_when_switching() {
        let mut settings = GeneralSettings::default();
        for (kind, values, inversions) in [
            (
                NavigationType::MouseTrackpad,
                [2., 0.5, 3.],
                [true, false, true],
            ),
            (
                NavigationType::SpaceMouse,
                [0.4, 2.5, 1.5],
                [false, true, false],
            ),
        ] {
            settings.navigation_type = kind;
            for (speed, value) in settings.navigation_speeds_mut().into_iter().zip(values) {
                *speed = value;
            }
            for (invert, value) in settings
                .navigation_inversions_mut()
                .into_iter()
                .zip(inversions)
            {
                *invert = value;
            }
        }
        for (kind, expected, inversions) in [
            (
                NavigationType::MouseTrackpad,
                [2., 0.5, 3.],
                [true, false, true],
            ),
            (
                NavigationType::SpaceMouse,
                [0.4, 2.5, 1.5],
                [false, true, false],
            ),
        ] {
            settings.navigation_type = kind;
            assert_eq!(
                settings.navigation_speeds_mut().map(|value| *value),
                expected
            );
            assert_eq!(
                settings.navigation_inversions_mut().map(|value| *value),
                inversions
            );
        }
        assert!(settings.space_mouse.enabled);
    }

    #[test]
    fn navigation_renders_one_speed_group_and_device_options_above_it() {
        for kind in [NavigationType::MouseTrackpad, NavigationType::SpaceMouse] {
            let ctx = egui::Context::default();
            let mut app = BenchUpApp::with_document(Document::default(), None, true);
            app.general_settings.navigation_type = kind;
            let mut output = ctx.run_test_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 900.))),
                    ..Default::default()
                },
                |ui| app.navigation_settings_contents(ui),
            );
            let text = rendered_text(&output.shapes);
            output.textures_delta.clear();
            assert!(text.iter().any(|label| label == "Navigation type"));
            assert!(text.iter().any(|label| label == kind.label()));
            assert_eq!(
                text.iter().any(|label| label == "Middle mouse drag"),
                kind == NavigationType::MouseTrackpad
            );
            assert_eq!(
                text.iter().any(|label| label == "Orbit (Shift to pan)"),
                kind == NavigationType::MouseTrackpad
            );
            for label in ["Orbit speed", "Pan speed", "Zoom speed"] {
                assert_eq!(
                    text.iter().filter(|value| value.as_str() == label).count(),
                    1
                );
            }
            assert!(!text.iter().any(|label| label == "Reverse scroll zoom"));
            assert!(!text.iter().any(|label| label == "Navigate with a 3D mouse"));
            assert_eq!(
                text.iter().any(|label| label == "Enable 3D Mouse"),
                kind == NavigationType::SpaceMouse
            );
            for label in ["Invert tilt and twist", "Invert pan", "Invert zoom"] {
                assert_eq!(
                    text.iter().filter(|value| value.as_str() == label).count(),
                    1
                );
            }
            let option = "Invert zoom";
            let option_index = text.iter().position(|label| label == option).unwrap();
            let orbit_index = text
                .iter()
                .position(|label| label == "Orbit speed")
                .unwrap();
            assert!(option_index < orbit_index);
        }
    }

    #[test]
    fn every_settings_section_renders_in_the_shared_modal() {
        for section in Section::ALL {
            let ctx = egui::Context::default();
            let mut app = BenchUpApp::with_document(Document::default(), None, true);
            app.settings_open = true;
            app.general_settings.section = section;
            let mut output = ctx.run_test_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1100., 800.))),
                    ..Default::default()
                },
                |_| app.settings_window(&ctx),
            );
            output.textures_delta.clear();
            assert!(app.settings_open);
        }
    }

    #[test]
    fn supported_languages_include_simplified_chinese() {
        assert!(languages().contains(&("zh_CN", "Simplified Chinese")));
        let mut settings = GeneralSettings::default();
        settings.language = "zh_CN".into();
        settings.normalize();
        assert_eq!(settings.language, "zh_CN");
    }

    #[test]
    fn supported_languages_include_brazilian_portuguese() {
        assert!(languages().contains(&("pt_BR", "Brazilian Portuguese")));
        let mut settings = GeneralSettings::default();
        settings.language = "pt_BR".into();
        settings.normalize();
        assert_eq!(settings.language, "pt_BR");
    }

    #[test]
    fn supported_languages_include_vietnamese() {
        assert!(languages().contains(&("vi_VN", "Vietnamese")));
        let mut settings = GeneralSettings::default();
        settings.language = "vi_VN".into();
        settings.normalize();
        assert_eq!(settings.language, "vi_VN");
    }

    #[test]
    fn supported_languages_include_spanish() {
        assert!(languages().contains(&("es_419", "Spanish")));
        let mut settings = GeneralSettings::default();
        settings.language = "es_419".into();
        settings.normalize();
        assert_eq!(settings.language, "es_419");
    }

    #[test]
    fn every_language_code_uses_language_and_region() {
        for (code, _) in languages() {
            let (language, region) = code.split_once('_').expect(code);
            assert!(!language.is_empty() && !region.is_empty(), "{code}");
        }
    }

    #[test]
    fn saved_bare_language_codes_migrate_to_regional_codes() {
        for (saved, expected) in [("en", "en_US"), ("vi", "vi_VN"), ("es", "es_419"), ("unsupported", "en_US")] {
            let mut settings = GeneralSettings::default();
            settings.language = saved.into();
            settings.normalize();
            assert_eq!(settings.language, expected, "{saved}");
        }
        assert_eq!(GeneralSettings::default().language, "en_US");
    }

    #[test]
    fn language_picker_shows_native_names_in_translated_catalogs() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) { benchup_core::i18n::set_language("en"); }
        }
        let _reset = Reset;
        let native = [
            ("en_US", "English"),
            ("zh_CN", "简体中文"),
            ("pt_BR", "Português (Brasil)"),
            ("vi_VN", "Tiếng Việt"),
            ("es_419", "Español"),
        ];
        for (language, _) in &native[1..] {
            benchup_core::i18n::set_language(language);
            assert_eq!(languages(), native, "{language}");
        }
    }

    #[test]
    fn settings_labels_switch_languages_in_the_same_window() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) { benchup_core::i18n::set_language("en"); }
        }
        let _reset = Reset;
        let ctx = egui::Context::default();
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        app.settings_open = true;
        for (language, title, group, export_scales) in [
            ("en_US", "Settings", "Workspace", "Export scales"),
            ("zh_CN", "设置", "工作区", "导出比例"),
            ("pt_BR", "Configurações", "Área de trabalho", "Escalas de exportação"),
            ("vi_VN", "Cài đặt", "Không gian làm việc", "Tỷ lệ xuất"),
            ("es_419", "Configuración", "Espacio de trabajo", "Escalas de exportación"),
            ("en_US", "Settings", "Workspace", "Export scales"),
        ] {
            app.general_settings.language = language.into();
            let input = || egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1100., 800.))),
                ..Default::default()
            };
            ctx.run_test_ui(input(), |_| app.settings_window(&ctx));
            let output = ctx.run_test_ui(input(), |_| app.settings_window(&ctx));
            let text = rendered_text(&output.shapes);
            assert!(text.iter().any(|label| label == title));
            assert!(text.iter().any(|label| label == group));
            assert_eq!(Section::ExportScales.label(), export_scales);
        }
    }

    #[test]
    fn settings_header_shows_the_language_picker() {
        let ctx = egui::Context::default();
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        app.settings_open = true;
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1100., 800.))),
            ..Default::default()
        };
        ctx.run_test_ui(input(), |_| app.settings_window(&ctx));
        let output = ctx.run_test_ui(input(), |_| app.settings_window(&ctx));
        let text = rendered_text(&output.shapes);
        assert!(text.iter().any(|label| label == "Language"));
        assert!(text.iter().any(|label| label == "English"));
    }
}
