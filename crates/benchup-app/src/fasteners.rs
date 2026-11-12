use benchup_core::tr;
use super::*;
use crate::inspector_table::inspector_field;
use benchup_core::connections::{
    Connection, ConnectionOperation, FastenerKind, FastenerSettings, Pattern,
};

// Temporarily hide the feature without changing saved projects or preferences.
pub(super) const AVAILABLE: bool = false;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub settings: FastenerSettings,
    pub reverse: bool,
    pub open: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            settings: Default::default(),
            reverse: false,
            open: false,
        }
    }
}
impl Preferences {
    pub fn sanitize(&mut self) {
        if !self.settings.valid() {
            self.settings = Default::default();
        }
    }
}
#[derive(Default)]
pub(super) struct Dialog {
    pub members: Option<[u64; 2]>,
    pub entry_face: Option<u64>,
    pub picking: Option<PickTarget>,
}
#[derive(Clone, Copy)]
pub(super) enum PickTarget {
    Draft,
    Connection(u64),
}

fn fields(ui: &mut egui::Ui, settings: &mut FastenerSettings) {
    inspector_field(ui, "fastener_kind", tr!("fasteners.fastener"), |ui| {
        egui::ComboBox::from_id_salt("kind")
            .selected_text(settings.kind.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.kind, FastenerKind::Screw, tr!("fasteners.screw"));
                ui.selectable_value(&mut settings.kind, FastenerKind::Nail, tr!("fasteners.nail"));
                ui.selectable_value(
                    &mut settings.kind,
                    FastenerKind::InclinedScrew,
                    tr!("fasteners.angled_screw"),
                );
            })
            .response
    });
    inspector_field(ui, "fastener_pattern", tr!("fasteners.pattern"), |ui| {
        egui::ComboBox::from_id_salt("pattern")
            .selected_text(match settings.pattern {
                Pattern::Single => tr!("fasteners.single"),
                Pattern::Row => tr!("fasteners.row"),
                Pattern::DoubleRow => tr!("fasteners.two_rows"),
                Pattern::Grid => tr!("fasteners.grid"),
            })
            .show_ui(ui, |ui| {
                for (value, label) in [
                    (Pattern::Single, tr!("fasteners.single")),
                    (Pattern::Row, tr!("fasteners.row")),
                    (Pattern::DoubleRow, tr!("fasteners.two_rows")),
                    (Pattern::Grid, tr!("fasteners.grid")),
                ] {
                    ui.selectable_value(&mut settings.pattern, value, label);
                }
            })
            .response
    });
    if settings.pattern != Pattern::Single {
        inspector_field(ui, "fastener_count", tr!("fasteners.count_per_row"), |ui| {
            ui.add(egui::DragValue::new(&mut settings.count).range(1..=100))
        });
        millimetres(
            ui,
            "spacing",
            tr!("fasteners.spacing_0_auto"),
            &mut settings.spacing,
            0.,
            10000.,
        );
    }
    if matches!(settings.pattern, Pattern::Grid | Pattern::DoubleRow) {
        if settings.pattern == Pattern::Grid {
            inspector_field(ui, "rows", tr!("fasteners.rows"), |ui| {
                ui.add(egui::DragValue::new(&mut settings.rows).range(1..=100))
            });
        }
        millimetres(
            ui,
            "row_spacing",
            tr!("fasteners.row_spacing_0_auto"),
            &mut settings.row_spacing,
            0.,
            10000.,
        );
    }
    millimetres(
        ui,
        "diameter",
        tr!("fasteners.preview_diameter"),
        &mut settings.diameter,
        0.1,
        100.,
    );
    millimetres(
        ui,
        "length",
        tr!("fasteners.length_0_geometry"),
        &mut settings.length,
        0.,
        10000.,
    );
    millimetres(
        ui,
        "clearance",
        tr!("fasteners.geometric_clearance"),
        &mut settings.clearance,
        0.,
        1000.,
    );
    if settings.kind == FastenerKind::InclinedScrew {
        inspector_field(ui, "inclination", tr!("fasteners.angle_from_face_normal"), |ui| {
            ui.add(
                egui::DragValue::new(&mut settings.angle_degrees)
                    .range(0. ..=60.)
                    .suffix("°"),
            )
        });
        ui.text_weak(
            tr!("fasteners.angled_axis_only_the_pocket_recess_and_kreg"),
        );
    }
}
fn millimetres(ui: &mut egui::Ui, id: &str, label: &str, value: &mut f64, min: f64, max: f64) {
    let mut mm = *value * 1000.;
    if inspector_field(ui, id, label, |ui| {
        ui.add(
            egui::DragValue::new(&mut mm)
                .speed(0.1)
                .range(min..=max)
                .suffix(" mm"),
        )
    })
    .changed()
    {
        *value = mm / 1000.;
    }
}
impl BenchUpApp {
    pub(super) fn fastener_contents(&mut self, ui: &mut egui::Ui) {
        let Some(members) = self.fastener_dialog.members else {
            ui.text_weak(
                tr!("fasteners.select_exactly_two_objects_and_reopen_automatic_fasteners"),
            );
            return;
        };
        let eligible = self.fastener_pair_eligible(ui.ctx(), members);
        if !eligible {
            ui.colored_label(Color32::LIGHT_RED, self.fastener_pair_block_reason(ui.ctx(), members));
        }
        let mut objects = members.to_vec();
        ui.checkbox(
            &mut self.doc.display.fasteners,
            tr!("fasteners.show_fasteners_through_the_model"),
        );

        ui.checkbox(
            &mut self.doc.display.fastener_preview,
            tr!("fasteners.preview_before_creating"),
        );
        if ui
            .checkbox(&mut self.fasteners.reverse, tr!("fasteners.swap_the_selected_parts"))
            .changed()
        {
            self.fastener_dialog.entry_face = None;
        }
        if ui
            .add_enabled(eligible, egui::Button::new(tr!("fasteners.pick_entry_face_in_viewport")))
            .clicked()
        {
            self.fastener_dialog.picking = Some(PickTarget::Draft);
            self.doc.display.fastener_face_handles = true;
        }
        if ui.button(tr!("fasteners.automatic_entry_face")).clicked() {
            self.fastener_dialog.entry_face = None;
        }
        if let Some(face) = self.fastener_dialog.entry_face {
            ui.text_label(tr!("fasteners.entry_face", face = face));
        }
        ui.text_weak(tr!("fasteners.select_exactly_two_touching_closed_solid_parts_the"));
        ui.push_id("draft", |ui| fields(ui, &mut self.fasteners.settings));
        if self.fasteners.reverse {
            objects.reverse();
        }
        if objects.len() == 2 {
            let connection = Connection {
                members: [objects[0], objects[1]],
                entry_face: self.fastener_dialog.entry_face,
                operation: ConnectionOperation::Fasteners(self.fasteners.settings.clone()),
            };
            for (index, id) in connection.members.iter().enumerate() {
                let Some(object) = self.doc.model.objects.get(id) else {
                    continue;
                };
                let material = self
                    .doc
                    .model
                    .object_stock_material(object)
                    .map(|m| m.name.as_str())
                    .unwrap_or(tr!("fasteners.no_material_assigned"));
                ui.text_label(tr!("fasteners.part", index + 1, object.name, material = material));
            }
            let evaluation = connection.evaluate(&self.doc.model);
            for error in &evaluation.errors {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            for warning in evaluation
                .placements
                .iter()
                .flat_map(|p| &p.warnings)
                .collect::<BTreeSet<_>>()
            {
                ui.colored_label(Color32::YELLOW, warning);
            }
            ui.text_label(tr!(
                "fasteners.preview_fasteners",
                count = evaluation.placements.len()
            ));
            if ui
                .add_enabled(
                    evaluation.errors.is_empty() && !evaluation.placements.is_empty() && eligible,
                    egui::Button::new(tr!("fasteners.create_fasteners")),
                )
                .clicked()
            {
                self.create_fasteners(ui.ctx(), connection);
            }
        }
        ui.separator();
        ui.text_weak(tr!("fasteners.geometric_sizing_automatic_length_passes_through_the_first"));
        ui.hyperlink_to(
            tr!("fasteners.kreg_screw_selector_thicknesses_and_materials"),
            "https://www.kregtool.com/screw-selector/",
        );
        ui.hyperlink_to(
            tr!("fasteners.simpson_strong_tie_technical_guides"),
            "https://www.strongtie.com/resources/literature/fastening-systems-technical-supplement",
        );
        ui.text_weak(tr!("fasteners.for_cut_joints_choose_wood_joinery_from_the"));
        let connections: Vec<_> = self
            .doc
            .model
            .connections
            .iter()
            .filter(|(_, c)| same_pair(c.members, members) && matches!(c.operation, ConnectionOperation::Fasteners(_)))
            .map(|(id, c)| (*id, c.clone()))
            .collect();
        for (id, mut connection) in connections {
            ui.separator();
            ui.push_id(id, |ui| {
                let names = connection.members.map(|id| {
                    self.doc
                        .model
                        .objects
                        .get(&id)
                        .map(|o| o.name.clone())
                        .unwrap_or_else(|| tr!("fasteners.missing_part", id = id))
                });
                ui.text_label(tr!("fasteners.connection_direction", names[0], names[1], id = id));
                let original = connection.clone();
                let locked = !self.fastener_pair_eligible(ui.ctx(), connection.members);
                let mut remove = false;
                ui.add_enabled_ui(!locked, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button(tr!("fasteners.reverse_direction")).clicked() {
                            connection.members.swap(0, 1);
                            connection.entry_face = None;
                        }
                        if ui.button(tr!("fasteners.auto_face")).clicked() {
                            connection.entry_face = None;
                        }
                        remove = ui.button(tr!("common.delete")).clicked();
                    });
                    ui.horizontal(|ui| {
                        if ui.button(tr!("fasteners.pick_entry_face_in_viewport")).clicked() {
                            self.fastener_dialog.picking = Some(PickTarget::Connection(id));
                            self.doc.display.fastener_face_handles = true;
                        }
                    });
                    let faces = connection.entry_faces(&self.doc.model);
                    egui::ComboBox::from_id_salt("entry")
                        .selected_text(
                            connection
                                .entry_face
                                .map(|id| tr!("fasteners.face", id = id))
                                .unwrap_or_else(|| tr!("fasteners.automatic_face").into()),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut connection.entry_face, None, tr!("fasteners.automatic"));
                            for id in faces {
                                ui.selectable_value(
                                    &mut connection.entry_face,
                                    Some(id),
                                    tr!("fasteners.face", id = id),
                                );
                            }
                        });
                    let ConnectionOperation::Fasteners(settings) = &mut connection.operation else { return; };
                    fields(ui, settings);
                });
                let evaluation = connection.evaluate(&self.doc.model);
                for error in &evaluation.errors {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                for warning in evaluation
                    .placements
                    .iter()
                    .flat_map(|p| &p.warnings)
                    .collect::<BTreeSet<_>>()
                {
                    ui.colored_label(Color32::YELLOW, warning);
                }
                let ConnectionOperation::Fasteners(settings) = &connection.operation else { return; };
                ui.text_label(tr!(
                    "fasteners.hardware_mm",
                    evaluation.placements.len(),
                    settings.kind.label(),
                    settings.diameter * 1000.
                ));
                for (index, p) in evaluation.placements.iter().enumerate().take(8) {
                    ui.text_weak(tr!(
                        "fasteners.length_first_part_penetration_mm",
                        index + 1,
                        p.head.distance(p.tip) * 1000.,
                        p.first_depth * 1000.,
                        p.penetration * 1000.
                    ));
                }
                if evaluation.placements.len() > 8 {
                    ui.text_weak(tr!("fasteners.dimensions_shown_for_the_first_eight_fasteners"));
                }
                if remove || original != connection {
                    self.commit_fastener_change(
                        ui.ctx(),
                        id,
                        &original,
                        (!remove).then_some(connection),
                    );
                }
            });
        }
        self.hardware_summary(ui);
    }
    fn hardware_summary(&self, ui: &mut egui::Ui) {
        use crate::inspector_table::{Column, InspectorTable};
        let mut items = std::collections::BTreeMap::<(String, u64, u64), u32>::new();
        let mut invalid = 0;
        for connection in self.doc.model.connections.values().filter(|c| {
            self.fastener_dialog
                .members
                .is_some_and(|m| same_pair(m, c.members))
        }) {
            let evaluation = connection.evaluate(&self.doc.model);
            let ConnectionOperation::Fasteners(settings) = &connection.operation else { continue; };
            if !evaluation.errors.is_empty() {
                invalid += 1;
            }
            let kind = settings.kind.label();
            for placement in evaluation.placements {
                *items
                    .entry((
                        kind.into(),
                        (settings.diameter * 1e6).round() as u64,
                        (placement.head.distance(placement.tip) * 1e6).round() as u64,
                    ))
                    .or_default() += 1;
            }
        }
        if items.is_empty() && invalid == 0 {
            return;
        }
        ui.separator();
        ui.text_label(tr!("fasteners.hardware_list_preview_dimensions"));
        let mut table =
            InspectorTable::new(ui.available_width(), [Column::flex(2.), Column::flex(1.)]);
        table.header(ui, [tr!("fasteners.type_length_mm"), tr!("fasteners.quantity")]);
        for ((kind, diameter, length), quantity) in items {
            table.row(ui, (kind.clone(), diameter, length), |column, ui| {
                if column == 0 {
                    ui.text_label(tr!(
                        "fasteners.hardware_dimensions",
                        diameter as f64 / 1000.,
                        length as f64 / 1000.,
                        kind = kind,
                    ));
                } else {
                    ui.text_label(quantity.to_string());
                }
            });
        }
        if invalid > 0 {
            ui.colored_label(Color32::LIGHT_RED,tr!("fasteners.invalid_connections", count = invalid));
        }
        ui.text_weak(tr!("fasteners.review_geometric_warnings_before_purchasing_structural_strength_has"));
    }
    pub(super) fn paint_fasteners(&self, p: &egui::Painter, rect: Rect, model: &Model) {
        if !self.doc.display.fasteners {
            return;
        }
        let view = self.doc.camera.view(dv(rect.size()));
        let screen = |p| view.project(p).map(|p| rect.min + pv(p));
        let mut draft = None;
        if AVAILABLE
            && self.doc.display.fastener_preview
            && self.fasteners.open
            && self.history_preview.is_none()
        {
            if let Some(mut members) = self.fastener_dialog.members {
                if self.fasteners.reverse {
                    members.swap(0, 1);
                }
                draft = Some(Connection {
                    members,
                    entry_face: self.fastener_dialog.entry_face,
                    operation: ConnectionOperation::Fasteners(self.fasteners.settings.clone()),
                });
            }
        }
        for connection in model.connections.values().chain(draft.iter()).filter(|c| matches!(c.operation, ConnectionOperation::Fasteners(_))) {
            let result = connection.evaluate(model);
            for placement in result.placements {
                if let (Some(head), Some(tip), Some(interface)) = (
                    screen(placement.head),
                    screen(placement.tip),
                    screen(placement.interface),
                ) {
                    let color = if placement.warnings.is_empty() && result.errors.is_empty() {
                        Color32::from_rgb(80, 210, 240)
                    } else {
                        Color32::LIGHT_RED
                    };
                    p.line_segment(
                        [head, interface],
                        Stroke::new(3., color.gamma_multiply(0.65)),
                    );
                    p.line_segment([interface, tip], Stroke::new(3., color));
                    p.circle_filled(head, 4., color);
                    p.circle_filled(interface, 2., Color32::WHITE);
                    p.arrow(
                        tip - (tip - head).normalized() * 12.,
                        (tip - head).normalized() * 12.,
                        Stroke::new(2., color),
                    );
                }
            }
            if !result.errors.is_empty() {
                // Keep invalid relations visible even when no insertion axis remains.
                for member in connection.members {
                    let entities = model.expand_entities(&BTreeSet::from([member]));
                    if let Some(point) = entities
                        .iter()
                        .filter_map(|id| model.vertices.get(id))
                        .find_map(|v| screen(v.position))
                    {
                        p.text(
                            point,
                            Align2::LEFT_BOTTOM,
                            tr!("fasteners.invalid_fastener_connection"),
                            FontId::proportional(12.),
                            Color32::LIGHT_RED,
                        );
                    }
                }
            }
        }
    }
    fn fastener_pick_connection(&self) -> Option<Connection> {
        match self.fastener_dialog.picking? {
            PickTarget::Draft => {
                let mut members = self.fastener_dialog.members?;
                if self.fasteners.reverse {
                    members.swap(0, 1);
                }
                Some(Connection {
                    members,
                    entry_face: self.fastener_dialog.entry_face,
                    operation: ConnectionOperation::Fasteners(self.fasteners.settings.clone()),
                })
            }
            PickTarget::Connection(id) => self.doc.model.connections.get(&id).cloned(),
        }
    }
    fn fastener_handles(&self, rect: Rect) -> Vec<(u64, Pos2)> {
        let Some(connection) = self.fastener_pick_connection() else {
            return Vec::new();
        };
        connection
            .entry_faces(&self.doc.model)
            .into_iter()
            .filter_map(|id| {
                let face = self.doc.model.faces.get(&id)?;
                let center = face
                    .vertices
                    .iter()
                    .filter_map(|id| self.doc.model.vertices.get(id))
                    .map(|v| v.position)
                    .sum::<DVec3>()
                    / face.vertices.len() as f64;
                let point = rect.min + pv(self.doc.camera.project(center, dv(rect.size()))?);
                rect.contains(point).then_some((id, point))
            })
            .collect()
    }
    pub(super) fn fastener_handle_under_pointer(&self, _ctx: &egui::Context, _rect: Rect) -> bool {
        // While choosing a face, clicks must never select/move the underlying parts.
        AVAILABLE && self.fastener_dialog.picking.is_some()
    }
    pub(super) fn fastener_face_handles(&mut self, ui: &mut egui::Ui, rect: Rect) {
        if !AVAILABLE || !self.fasteners.open || self.history_preview.is_some() {
            return;
        }
        let Some(target) = self.fastener_dialog.picking else {
            return;
        };
        let Some(connection) = self.fastener_pick_connection() else {
            return;
        };
        let eligible = self.fastener_pair_eligible(ui.ctx(), connection.members);
        for (face, point) in self.fastener_handles(rect) {
            let response = ui
                .put(
                    Rect::from_center_size(point, egui::vec2(42., 24.)),
                    egui::Button::new(format!("{face}")).small(),
                )
                .on_hover_text(tr!("fasteners.use_this_entry_face"));
            if response.clicked() && eligible {
                match target {
                    PickTarget::Draft => self.fastener_dialog.entry_face = Some(face),
                    PickTarget::Connection(id) => {
                        let mut changed = connection.clone();
                        changed.entry_face = Some(face);
                        self.commit_fastener_change(ui.ctx(), id, &connection, Some(changed));
                    }
                }
                self.fastener_dialog.picking = None;
            }
        }
    }
}
fn same_pair(a: [u64; 2], b: [u64; 2]) -> bool {
    a == b || a == [b[1], b[0]]
}

impl BenchUpApp {
    pub(super) fn fastener_pair_block_reason(&self, ctx: &egui::Context, members: [u64; 2]) -> &'static str {
        let pair = BTreeSet::from(members);
        if self.selected != pair {
            tr!("fasteners.select_both_objects_belonging_to_this_joint_to")
        } else if self.doc.model.selection_protected(&pair) {
            tr!("fasteners.unlock_both_objects_before_editing_or_removing_their")
        } else if !members.iter().all(|id| self.doc.model.objects.contains_key(id)
            && self.doc.model.entity_in_context(*id) == Some(*id)
            && !self.doc.model.is_hidden(*id)) {
            tr!("fasteners.both_objects_must_be_visible_and_selectable_in")
        } else if self.project.blocked() || self.screenshot.open || self.history_preview.is_some() {
            tr!("fasteners.finish_the_current_dialog_or_history_preview_before")
        } else {
            self.shared_edit_block_reason(ctx)
        }
    }
    pub(super) fn fastener_pair_eligible(&self, ctx: &egui::Context, members: [u64; 2]) -> bool {
        self.fastener_pair_selected(members)
            && self.shared_fastener_edit_allowed(ctx, &BTreeSet::from(members))
    }
    fn fastener_pair_selected(&self, members: [u64; 2]) -> bool {
        let pair = BTreeSet::from(members);
        members[0] != members[1]
            && self.selected == pair
            && !self.project.blocked()
            && !self.screenshot.open
            && self.history_preview.is_none()
            && members.iter().all(|id| {
                self.doc.model.objects.contains_key(id)
                    && self.doc.model.entity_in_context(*id) == Some(*id)
                    && !self.doc.model.is_hidden(*id)
            })
            && !self.doc.model.selection_protected(&pair)
    }
    pub(super) fn selected_fastener_pair(&self, _ctx: &egui::Context) -> Option<[u64; 2]> {
        if self.selected.len() != 2 {
            return None;
        }
        let mut selected = self.selected.iter().copied();
        let pair = [selected.next()?, selected.next()?];
        // Opening a preview must not depend on a network round trip. Applying
        // any change still uses fastener_pair_eligible and its exclusive lease.
        self.fastener_pair_selected(pair).then_some(pair)
    }
    pub(super) fn open_fasteners(&mut self, ctx: &egui::Context) {
        let Some(members) = self.selected_fastener_pair(ctx) else {
            return;
        };
        self.fastener_dialog = Dialog {
            members: Some(members),
            ..Default::default()
        };
        self.joinery.open = false;
        self.close_router();
        self.fasteners.open = true;
        self.inspector = true;
        self.set_tool(Tool::Select);
    }
    fn create_fasteners(&mut self, ctx: &egui::Context, connection: Connection) -> bool {
        if !self
            .fastener_dialog
            .members
            .is_some_and(|pair| same_pair(pair, connection.members))
            || !self.fastener_pair_eligible(ctx, connection.members)
        {
            self.say(
                ctx,
                tr!("fasteners.the_object_pair_is_no_longer_editable_no"),
            );
            return false;
        }
        let before = self.doc.model.clone();
        match self.doc.model.add_connection(connection) {
            Ok(_) => {
                self.finish_edit(tr!("fasteners.create_fasteners"), before, ctx);
                true
            }
            Err(error) => {
                self.say(ctx, error);
                false
            }
        }
    }
    fn commit_fastener_change(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        original: &Connection,
        replacement: Option<Connection>,
    ) -> bool {
        let valid = self
            .fastener_dialog
            .members
            .is_some_and(|pair| same_pair(pair, original.members))
            && self.fastener_pair_eligible(ctx, original.members)
            && self.doc.model.connections.get(&id) == Some(original)
            && replacement.as_ref().is_none_or(|c| {
                let ConnectionOperation::Fasteners(s) = &c.operation else { return false; };
                same_pair(c.members, original.members)
                    && s.valid()
                    && c.entry_face
                        .is_none_or(|face| c.entry_faces(&self.doc.model).contains(&face))
            });
        if !valid {
            self.say(ctx,tr!("fasteners.the_objects_permissions_or_connection_changed_reopen_automatic"));
            return false;
        }
        let before = self.doc.model.clone();
        if let Some(connection) = replacement {
            self.doc.model.connections.insert(id, connection);
        } else {
            self.doc.model.connections.remove(&id);
        }
        self.finish_edit(tr!("fasteners.edit_fasteners"), before, ctx);
        true
    }
    pub(super) fn connection_sidebar_selection(&mut self, ctx: &egui::Context) {
        let pair = if self.selected.len() == 2 {
            let mut selected = self.selected.iter().copied();
            Some([selected.next().unwrap(), selected.next().unwrap()])
        } else {
            None
        };
        if self.fasteners.open {
            if self.fastener_dialog.members.is_none() {
                self.fastener_dialog.members = self.selected_fastener_pair(ctx);
            }
            if pair.is_none() || self.fastener_dialog.members != pair {
                self.fasteners.open = false;
            }
        }
        if self.joinery.open {
            if self.joinery_dialog.members.is_none() {
                self.joinery_dialog.members = self.selected_fastener_pair(ctx);
            }
            if pair.is_none() || self.joinery_dialog.members != pair {
                self.joinery.open = false;
            }
        }
        if !self.fasteners.open {
            self.fastener_dialog = Default::default();
        }
        if !self.joinery.open {
            self.joinery_dialog = Default::default();
        }
    }

    pub(super) fn automatic_fasteners_sidebar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let scroll = sidebar::restored_scroll_area(
            ui, "connections/fasteners",
            egui::ScrollArea::vertical().id_salt("automatic_fasteners_scroll")
                .content_margin(18)
                .auto_shrink([false, false]),
        ).show(ui, |ui| {
            sidebar::inspector_section(ui, tr!("fasteners.automatic_fasteners"), "connections/fasteners", true, |ui| {
        ui.text_weak(tr!("fasteners.changes_apply_immediately_change_the_selection_to_return"));
            if self.fastener_dialog.picking.is_some() {
                ui.text_label(tr!("fasteners.click_a_numbered_entry_face_handle_in_the"));
                if ui.button(tr!("fasteners.back_to_automatic_fasteners")).clicked()
                    || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
                {
                    self.fastener_dialog.picking = None;
                }
            } else {
                self.fastener_contents(ui);
            }
            });
        });
        sidebar::remember_scroll(&ctx, "connections/fasteners", scroll.state.clamped_offset());
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (BenchUpApp, egui::Context, [u64; 2]) {
        let mut model = Model::default();
        let mut members = [0; 2];
        for (i, member) in members.iter_mut().enumerate() {
            let edge = model
                .add_edge(DVec3::new(i as f64, 0., 0.), DVec3::new(i as f64, 1., 0.))
                .unwrap();
            *member = model
                .make_object(&BTreeSet::from([edge]), false, "Part")
                .unwrap();
        }
        let app = BenchUpApp::with_document(
            Document {
                model,
                ..Default::default()
            },
            None,
            true,
        );
        (app, egui::Context::default(), members)
    }
    #[test]
    fn connection_sidebars_stay_open_until_selection_changes() {
        let (mut app, ctx, members) = fixture();
        app.selected = BTreeSet::from(members);
        app.inspector = false;
        app.open_fasteners(&ctx);
        assert!(app.inspector);
        app.connection_sidebar_selection(&ctx);
        assert!(app.fasteners.open);
        app.fastener_dialog.picking = Some(PickTarget::Draft);
        app.selected.remove(&members[0]);
        app.connection_sidebar_selection(&ctx);
        assert!(!app.fasteners.open);
        assert!(app.fastener_dialog.picking.is_none());
        assert!(app.fastener_dialog.members.is_none());

        app.selected = BTreeSet::from(members);
        app.open_joinery(&ctx);
        app.connection_sidebar_selection(&ctx);
        assert!(app.joinery.open);
        assert!(!app.fasteners.open);
        app.selected.clear();
        app.connection_sidebar_selection(&ctx);
        assert!(!app.joinery.open);
        assert!(app.joinery_dialog.members.is_none());
    }

    #[test]
    fn menu_requires_exactly_two_editable_objects_in_current_context() {
        let (mut app, ctx, members) = fixture();
        assert!(app.selected_fastener_pair(&ctx).is_none());
        app.selected = BTreeSet::from([members[0]]);
        assert!(app.selected_fastener_pair(&ctx).is_none());
        app.selected = BTreeSet::from(members);
        assert!(app.selected_fastener_pair(&ctx).is_some());
        let edge = *app.doc.model.edges.keys().next().unwrap();
        app.selected.insert(edge);
        assert!(app.selected_fastener_pair(&ctx).is_none());
        app.selected = BTreeSet::from([members[0], edge]);
        assert!(app.selected_fastener_pair(&ctx).is_none());
        app.selected = BTreeSet::from(members);
        app.doc.model.locked.insert(members[0]);
        app.open_fasteners(&ctx);
        assert!(!app.fasteners.open);
        app.doc.model.locked.clear();
        app.open_fasteners(&ctx);
        assert!(app.fasteners.open);
        app.doc.model.edit_context = Some(members[0]);
        assert!(!app.fastener_pair_eligible(&ctx, members));
    }
    #[test]
    fn shared_pair_can_open_both_dialogs_while_awaiting_its_lease() {
        let (mut app, ctx, members) = fixture();
        app.account.server = "http://127.0.0.1:0".into();
        app.account.server_reachable = true;
        app.selected = BTreeSet::from(members);
        app.doc.shared_project = Some(benchup_core::SharedProject {
            server: "https://sharing.example".into(),
            id: "a".repeat(48),
            revision: 1,
            server_updated_at: 0,
            file_updated_at: 0,
        });
        assert!(app.selected_fastener_pair(&ctx).is_some());
        assert!(!app.fastener_pair_eligible(&ctx, members));
        app.open_fasteners(&ctx);
        assert!(app.fasteners.open);
        app.open_joinery(&ctx);
        assert!(app.joinery.open);
        assert!(!app.fastener_pair_eligible(&ctx, members));
    }
    #[test]
    fn stale_deleted_or_locked_connections_cannot_commit() {
        let (mut app, ctx, members) = fixture();
        app.selected = BTreeSet::from(members);
        app.open_fasteners(&ctx);
        let connection = Connection {
            members,
            entry_face: None,
            operation: ConnectionOperation::Fasteners(Default::default()),
        };
        let id = u64::MAX - 1;
        app.doc.model.connections.insert(id, connection.clone());
        let mut changed = connection.clone();
        let ConnectionOperation::Fasteners(s) = &mut changed.operation else { panic!("Expected fasteners"); };
        s.count = 7;
        assert!(app.commit_fastener_change(&ctx, id, &connection, Some(changed.clone())));
        // A remote edit invalidates a locally retained snapshot.
        assert!(!app.commit_fastener_change(&ctx, id, &connection, None));
        app.doc.model.locked.insert(members[1]);
        assert!(!app.commit_fastener_change(&ctx, id, &changed, None));
        app.doc.model.locked.clear();
        app.doc.model.objects.remove(&members[1]);
        let before = app.doc.model.clone();
        assert!(!app.create_fasteners(&ctx, connection));
        assert!(!app.commit_fastener_change(&ctx, id, &changed, None));
        assert_eq!(app.doc.model, before);
    }
}
