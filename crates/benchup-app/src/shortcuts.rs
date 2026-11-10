use super::*;
use benchup_core::tr;

struct Section<'a> {
    id: &'static str,
    title: &'a str,
    rows: Vec<(&'a str, String)>,
}

impl Section<'_> {
    fn matches(&self, action: &str, keys: &str, query: &str) -> bool {
        let text = format!("{} {action} {keys}", self.title).to_lowercase();
        query.split_whitespace().all(|word| text.contains(word))
    }

    fn show(&self, ui: &mut egui::Ui, query: &str) {
        let rows: Vec<_> = self
            .rows
            .iter()
            .filter(|(action, keys)| self.matches(action, keys, query))
            .collect();
        if rows.is_empty() {
            return;
        }
        let colors = Palette::for_ui(ui);
        egui::Frame::new()
            .fill(colors.surface)
            .stroke(Stroke::new(1., colors.border))
            .corner_radius(8)
            .inner_margin(14)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.text_label(
                    RichText::new(self.title)
                        .size(15.)
                        .strong()
                        .color(colors.ink),
                );
                ui.add_space(8.);
                let column_width = (ui.available_width() - 12.) * 0.5;
                egui::Grid::new(self.id)
                    .num_columns(2)
                    .spacing(egui::vec2(12., 9.))
                    .min_col_width(column_width)
                    .max_col_width(column_width)
                    .show(ui, |ui| {
                        for (action, keys) in rows {
                            ui.add(crate::natural_text::Label::new(*action).wrap());
                            // Keep badges compact and align them to the section's right edge.
                            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                                egui::Frame::new()
                                    .fill(colors.canvas)
                                    .stroke(Stroke::new(1., colors.border))
                                    .corner_radius(4)
                                    .inner_margin(egui::Margin::symmetric(7, 3))
                                    .show(ui, |ui| {
                                        ui.add(
                                            crate::natural_text::Label::new(
                                                RichText::new(keys)
                                                    .monospace()
                                                    .size(11.)
                                                    .color(colors.green),
                                            )
                                            .wrap(),
                                        );
                                    });
                            });
                            ui.end_row();
                        }
                    });
            });
        ui.add_space(6.);
    }
}

impl BenchUpApp {
    pub(super) fn help_window(&mut self, ctx: &egui::Context) {
        let mac = ctx.os() == egui::os::OperatingSystem::Mac;
        let command = if mac { "⌘" } else { "Ctrl" };
        let tool_row = |tool, action| {
            (
                action,
                shortcut_input::key_label(self.general_settings.tool_shortcut(tool)),
            )
        };
        let mira = self.assistant_available();
        let section = |id, title: &'static str, rows: &[(&'static str, &str)]| Section {
            id,
            title,
            rows: rows
                .iter()
                // The MIRA shortcut is only listed while the add-on is on.
                .filter(|(action, _)| mira || *action != tr!("assistant.name"))
                .map(|(action, keys)| (*action, (*keys).to_owned()))
                .collect(),
        };
        let assistant_keys = format!(
            "{command} + Shift + {}",
            shortcut_input::key_label(&self.general_settings.assistant_key)
        );
        let sections = [
            section(
                "drawing_measuring",
                tr!("shortcuts.drawing_measuring"),
                &[
                    tool_row(Tool::Select, tr!("shortcuts.select")),
                    tool_row(Tool::Line, tr!("shortcuts.connected_lines")),
                    tool_row(Tool::Rectangle, tr!("shortcuts.rectangle")),
                    tool_row(Tool::Circle, tr!("shortcuts.circle")),
                    tool_row(Tool::Polygon, tr!("shortcuts.polygon")),
                    tool_row(Tool::Freehand, tr!("shortcuts.freehand")),
                    tool_row(Tool::Paint, tr!("shortcuts.paint_faces")),
                    tool_row(Tool::Erase, tr!("shortcuts.erase_edges_or_faces")),
                    tool_row(Tool::TapeMeasure, tr!("shortcuts.tape_measure")),
                    tool_row(Tool::Protractor, tr!("shortcuts.protractor")),
                    tool_row(Tool::Dimensions, tr!("shortcuts.dimensions")),
                ],
            ),
            Section {
                id: "project_clipboard",
                title: tr!("shortcuts.project_clipboard"),
                rows: [
                    (
                        tr!("shortcuts.new_project"),
                        tr!("shortcuts.n", command = command),
                    ),
                    (
                        tr!("shortcuts.open_project"),
                        tr!("shortcuts.o", command = command),
                    ),
                    (tr!("common.save"), tr!("shortcuts.s", command = command)),
                    (
                        tr!("shortcuts.save_as"),
                        tr!("shortcuts.shift_s", command = command),
                    ),
                    (tr!("common.undo"), tr!("shortcuts.z", command = command)),
                    (
                        tr!("common.redo"),
                        tr!("shortcuts.shift_z", command = command),
                    ),
                    (tr!("shortcuts.cut"), tr!("shortcuts.x", command = command)),
                    (tr!("shortcuts.copy"), tr!("shortcuts.c", command = command)),
                    (
                        tr!("shortcuts.paste"),
                        tr!("shortcuts.v", command = command),
                    ),
                    (
                        tr!("shortcuts.paste_in_place"),
                        tr!("shortcuts.shift_v", command = command),
                    ),
                ]
                .into(),
            },
            section(
                "modeling",
                tr!("shortcuts.modeling"),
                &[
                    tool_row(Tool::PushPull, tr!("shortcuts.push_pull_a_face")),
                    (tr!("shortcuts.push_pull_new_faces"), tr!("move_tool.ctrl")),
                    tool_row(Tool::Scale, tr!("shortcuts.scale_geometry")),
                    tool_row(Tool::Offset, tr!("shortcuts.offset_a_face")),
                    tool_row(Tool::Move, tr!("shortcuts.move_geometry")),
                    tool_row(Tool::Rotate, tr!("shortcuts.rotate_geometry")),
                    (tr!("shortcuts.toggle_move_copy"), Self::move_copy_key(ctx)),
                    (
                        tr!("shortcuts.divide_a_copy_span"),
                        tr!("shortcuts.n_enter"),
                    ),
                    (tr!("shortcuts.repeat_copies"), tr!("shortcuts.nx_enter")),
                    (
                        tr!("shortcuts.flip_solid_operations"),
                        tr!("shortcuts.union_flyout"),
                    ),
                ],
            ),
            section(
                "navigation",
                tr!("shortcuts.navigation"),
                &[
                    tool_row(Tool::Orbit, tr!("shortcuts.orbit")),
                    (
                        tr!("shortcuts.pan_while_orbiting"),
                        tr!("shortcuts.shift_drag"),
                    ),
                    (
                        tr!("shortcuts.orbit_while_drawing"),
                        if mac {
                            tr!("shortcuts.move_pointer")
                        } else {
                            tr!("shortcuts.ctrl_move_pointer")
                        },
                    ),
                    (
                        tr!("shortcuts.temporary_orbit"),
                        tr!("shortcuts.middle_drag"),
                    ),
                    (
                        tr!("shortcuts.temporary_pan"),
                        tr!("shortcuts.shift_middle_drag"),
                    ),
                    (tr!("shortcuts.zoom_to_pointer"), tr!("shortcuts.scroll")),
                    (tr!("shortcuts.toggle_sidebar"), tr!("shortcuts.tab")),
                    ("Walk through model", "Walk button → click a floor face"),
                    ("Move while walking", "WASD / Arrow keys"),
                    ("Look while walking", "Move mouse"),
                    ("Crouch / Jump / Sit or stand", "Shift / Space / Alt"),
                    ("Exit Walk", "Escape"),
                ],
            ),
            section(
                "precision_inference",
                tr!("shortcuts.precision_inference"),
                &[
                    (
                        tr!("shortcuts.lock_x_y_z_axis"),
                        tr!("shortcuts.axis_lock_keys"),
                    ),
                    (
                        tr!("shortcuts.lock_a_plane"),
                        tr!("shortcuts.toggle_two_axis_locks"),
                    ),
                    (
                        tr!("shortcuts.lock_shape_plane_normal_x_y_z"),
                        tr!("shortcuts.axis_lock_keys"),
                    ),
                    (tr!("shortcuts.clear_all_axis_locks"), "↓"),
                    (
                        tr!("shortcuts.set_length_or_radius"),
                        tr!("shortcuts.value_enter"),
                    ),
                    (
                        tr!("shortcuts.set_rectangle_size"),
                        tr!("shortcuts.width_height_enter"),
                    ),
                    (
                        tr!("shortcuts.acquire_reference_axes"),
                        tr!("shortcuts.hover_geometry"),
                    ),
                    (
                        tr!("shortcuts.match_reference_coordinate"),
                        tr!("shortcuts.axis_lock_hover"),
                    ),
                    (
                        tr!("shortcuts.cancel_current_operation"),
                        tr!("shortcuts.escape"),
                    ),
                ],
            ),
            section(
                "selection_objects",
                tr!("shortcuts.selection_objects"),
                &[
                    (
                        tr!("shortcuts.toggle_selection"),
                        tr!("shortcuts.shift_click"),
                    ),
                    (
                        tr!("shortcuts.search_outliner"),
                        if mac {
                            "⌘ + F"
                        } else {
                            tr!("shortcuts.ctrl_f")
                        },
                    ),
                    (
                        tr!("shortcuts.select_all"),
                        if mac {
                            "⌘ + A"
                        } else {
                            tr!("shortcuts.ctrl_a")
                        },
                    ),
                    (
                        tr!("shortcuts.object_clipboard_menu"),
                        tr!("shortcuts.right_click"),
                    ),
                    (
                        tr!("shortcuts.edit_group_or_component"),
                        tr!("shortcuts.double_click"),
                    ),
                    (
                        tr!("shortcuts.close_editing_level"),
                        tr!("shortcuts.escape"),
                    ),
                    (
                        tr!("shortcuts.delete_selection"),
                        tr!("shortcuts.delete_backspace"),
                    ),
                    (tr!("assistant.name"), assistant_keys.as_str()),
                ],
            ),
        ];
        let width = (ctx.content_rect().width() - 72.).clamp(80., 900.);
        let response = Modal::new("keyboard_shortcuts", 940.).show(ctx, |modal| {
            let mut filter_changed = false;
            modal.header(|ui| {
                ui.text_heading(tr!("shortcuts.keyboard_shortcuts"));
                ui.text_weak(tr!("shortcuts.find_a_tool_command_or_gesture"));
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    let clear_width = if self.shortcut_filter.is_empty() {
                        0.
                    } else {
                        60.
                    };
                    filter_changed = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.shortcut_filter)
                                .hint_text(tr!("shortcuts.search_shortcuts"))
                                .desired_width(ui.available_width() - clear_width)
                                .margin(egui::vec2(10., 8.)),
                        )
                        .changed();
                    if !self.shortcut_filter.is_empty() && ui.button(tr!("common.clear")).clicked()
                    {
                        self.shortcut_filter.clear();
                        filter_changed = true;
                    }
                });
            });
            let query = self.shortcut_filter.trim().to_lowercase();
            let any_matches = sections.iter().any(|section| {
                section
                    .rows
                    .iter()
                    .any(|(action, keys)| section.matches(action, keys, &query))
            });
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("shortcut_results")
                .auto_shrink([false, true]);
            if filter_changed {
                scroll = scroll.vertical_scroll_offset(0.);
            }
            modal.body_with_scroll(scroll, |ui| {
                let colors = Palette::for_ui(ui);
                if !any_matches {
                    ui.add_space(24.);
                    ui.text_label(RichText::new(tr!("shortcuts.no_shortcuts_found")).strong());
                    ui.text_label(
                        RichText::new(tr!(
                            "shortcuts.try_a_tool_name_like_orbit_or_a",
                            command = command,
                        ))
                        .color(colors.muted),
                    );
                } else if width >= 740. {
                    ui.columns(2, |columns| {
                        for (index, section) in sections.iter().enumerate() {
                            section.show(&mut columns[index % 2], &query);
                        }
                    });
                } else {
                    for section in &sections {
                        section.show(ui, &query);
                    }
                }
            });
            modal.footer(|ui| {
                if primary_button(ui, tr!("common.close"), true).clicked() {
                    ui.close();
                }
                ui.text_weak(tr!("shortcuts.lengths_accept_units_and_formulas_1m_20cm_2"));
            });
        });
        if response.should_close() {
            self.help = false;
        }
    }
}
