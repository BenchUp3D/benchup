use super::*;
use benchup_core::{
    router::{Profile, Settings},
    tr,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    // Expansion uses the shared "router" section key. Serde ignores the old
    // standalone panel's `open` field when loading existing preferences.
    pub settings: Settings,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            settings: Settings {
                profile: Profile::None,
                ..Default::default()
            },
        }
    }
}

#[derive(Default)]
struct ProfileInteraction {
    activated: bool,
}

fn apply_draft(model: &mut Model, draft: &BTreeMap<u64, Settings>) {
    for (&edge, &settings) in draft {
        if settings.profile == Profile::None {
            model.routes.remove(&edge);
        } else {
            model.routes.insert(edge, settings);
        }
    }
}

#[derive(Default)]
pub(super) struct Editor {
    pub(super) reveal: bool,
    scroll_to: bool,
    selection: BTreeSet<u64>,
    board: Option<Target>,
    scope: Option<u64>,
    surface: bool,
    surface_revision: Option<u64>,
    original: BTreeMap<u64, Option<Settings>>,
    draft: BTreeMap<u64, Settings>,
    validation: Option<Validation>,
    evaluation: benchup_core::joinery::EvaluationCache,
}

/// A grouped piece or the loose geometry at the document root. This is editor
/// state only; machining settings stay attached to their original document edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Loose,
    Object(u64),
}

impl Target {
    fn for_edge(model: &Model, edge: u64) -> Option<Self> {
        model
            .edges
            .contains_key(&edge)
            .then(|| model.parent(edge).map_or(Self::Loose, Self::Object))
    }

    fn protected(self, model: &Model, edges: &BTreeSet<u64>) -> bool {
        if model.locked.is_empty() {
            return false;
        }
        match self {
            Self::Object(id) => model.selection_protected(&BTreeSet::from([id])),
            Self::Loose => edges.iter().any(|edge| {
                model
                    .route_entities(*edge)
                    .is_ok_and(|ids| model.selection_protected(&ids))
            }),
        }
    }
}

struct Validation {
    revision: u64,
    draft: BTreeMap<u64, Settings>,
    result: benchup_core::joinery::GeometryEvaluation,
}

fn size_field(ui: &mut egui::Ui, id: &str, label: &str, meters: &mut f64) {
    let mut mm = *meters * 1000.;
    if crate::inspector_table::inspector_field(ui, id, label, |ui| {
        ui.add(
            egui::DragValue::new(&mut mm)
                .range(*Settings::SIZE.start() * 1000. ..=*Settings::SIZE.end() * 1000.)
                .speed(0.1)
                .suffix(tr!("joinery.mm")),
        )
    })
    .changed()
    {
        *meters = mm / 1000.;
    }
}

/// Match the joinery cards: responsive tiles, visible selection, tooltips and
/// keyboard navigation. Selection still writes the persisted Settings field.
fn profile_selector(ui: &mut egui::Ui, current: &mut Profile) -> ProfileInteraction {
    let mut interaction = ProfileInteraction::default();
    let colors = Palette::for_ui(ui);
    let spacing = ui.spacing().item_spacing.x;
    let columns = ((ui.available_width() + spacing) / (145. + spacing))
        .floor()
        .clamp(1., 3.) as usize;
    let width = ((ui.available_width() - spacing * (columns - 1) as f32) / columns as f32).max(1.);
    let mut responses = Vec::new();
    ui.push_id("route_profile", |ui| {
        for row in std::iter::once(&Profile::ALL[..1]).chain(Profile::ALL[1..].chunks(columns)) {
            let width = if row[0] == Profile::None {
                ui.available_width()
            } else {
                width
            };
            ui.horizontal(|ui| {
                for &profile in row {
                    let mut response = ui
                        .push_id(profile as u8, |ui| {
                            ui.add_sized(
                                [width, if profile == Profile::None { 48. } else { 111. }],
                                egui::Button::new("")
                                    .selected(*current == profile)
                                    .corner_radius(5),
                            )
                        })
                        .inner
                        .on_hover_text(profile.label());
                    if response.clicked() {
                        *current = profile;
                        interaction.activated = true;
                        response.mark_changed();
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::Role::RadioButton,
                            ui.is_enabled(),
                            *current == profile,
                            profile.label(),
                        )
                    });
                    if ui.is_rect_visible(response.rect) {
                        let p = ui.painter().with_clip_rect(response.rect.shrink(3.));
                        if profile == Profile::None {
                            let center =
                                egui::pos2(response.rect.left() + 26., response.rect.center().y);
                            p.circle_stroke(center, 10., Stroke::new(1.5, colors.ink));
                            p.line_segment(
                                [center + egui::vec2(-7., 7.), center + egui::vec2(7., -7.)],
                                Stroke::new(1.5, colors.ink),
                            );
                            p.text(
                                response.rect.center(),
                                Align2::CENTER_CENTER,
                                profile.label(),
                                FontId::proportional(12.),
                                colors.ink,
                            );
                            if *current == profile {
                                p.text(
                                    response.rect.right_center() + egui::vec2(-12., 0.),
                                    Align2::RIGHT_CENTER,
                                    "✓",
                                    FontId::proportional(14.),
                                    colors.ink,
                                );
                            }
                        } else {
                            profile_icon(
                                &p,
                                Rect::from_min_max(
                                    response.rect.min + egui::vec2(12., 8.),
                                    egui::pos2(
                                        response.rect.right() - 12.,
                                        response.rect.top() + 76.,
                                    ),
                                ),
                                profile,
                                colors.ink,
                                ui.is_enabled(),
                            );
                            if *current == profile {
                                p.text(
                                    response.rect.right_top() + egui::vec2(-9., 7.),
                                    Align2::RIGHT_TOP,
                                    "✓",
                                    FontId::proportional(14.),
                                    colors.ink,
                                );
                            }
                            let mut label = egui::text::LayoutJob::simple(
                                profile.label().to_owned(),
                                FontId::proportional(12.),
                                colors.ink,
                                width - 14.,
                            );
                            label.halign = egui::Align::Center;
                            let galley = ui.fonts_mut(|fonts| fonts.layout_job(label));
                            p.galley(
                                egui::pos2(response.rect.center().x, response.rect.top() + 82.),
                                galley,
                                colors.ink,
                            );
                        }
                        if response.has_focus() {
                            p.rect_stroke(
                                response.rect.shrink(2.),
                                4,
                                Stroke::new(1.5, colors.ink),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                    responses.push((profile, response));
                }
            });
        }
    });
    if ui.is_enabled()
        && let Some(index) = responses.iter().position(|(_, r)| r.has_focus())
    {
        let step = ui.input(|i| {
            if i.key_pressed(Key::ArrowRight) {
                1
            } else if i.key_pressed(Key::ArrowLeft) {
                -1
            } else if i.key_pressed(Key::ArrowDown) {
                columns as isize
            } else if i.key_pressed(Key::ArrowUp) {
                -(columns as isize)
            } else {
                0
            }
        });
        if step != 0 {
            let next = if index == 0 && step == columns as isize {
                1
            } else if index <= columns && step == -(columns as isize) {
                0
            } else {
                (index as isize + step).clamp(0, responses.len() as isize - 1) as usize
            };
            *current = responses[next].0;
            interaction.activated = true;
            responses[next].1.request_focus();
        }
    }
    interaction
}

/// Cross sections show the retained wood and highlighted cutting boundary.
/// Each span is filled separately so concave profiles tessellate correctly.
fn profile_icon(p: &egui::Painter, rect: Rect, profile: Profile, ink: Color32, enabled: bool) {
    let v = egui::vec2;
    let quarter = |radius: f32, origin: egui::Vec2, convex: bool| {
        (0..=16).map(move |i| {
            let angle = std::f32::consts::FRAC_PI_2 * i as f32 / 16.;
            origin
                + radius
                    * if convex {
                        v(1. - angle.cos(), 1. - angle.sin())
                    } else {
                        v(angle.sin(), angle.cos())
                    }
        })
    };
    let boundary: Vec<_> = match profile {
        Profile::None => vec![v(0., 0.)],
        Profile::Chamfer => vec![v(0., 0.85), v(0.85, 0.)],
        Profile::Bevel => vec![v(0., 0.45), v(0.95, 0.)],
        Profile::Roundover => quarter(0.9, egui::Vec2::ZERO, true).collect(),
        Profile::Cove => quarter(0.9, egui::Vec2::ZERO, false).collect(),
        Profile::Rabbet => vec![v(0., 0.55), v(0.85, 0.55), v(0.85, 0.)],
        Profile::Groove => vec![
            v(0., 0.),
            v(0.3, 0.),
            v(0.3, 0.5),
            v(0.75, 0.5),
            v(0.75, 0.),
        ],
        Profile::Ogee => quarter(0.45, v(0., 0.45), true)
            .chain(quarter(0.45, v(0.45, 0.), false).skip(1))
            .collect(),
        Profile::Bead => std::iter::once(v(0., 1.05))
            .chain(quarter(0.75, v(0.2, 0.2), true))
            .chain(std::iter::once(v(1.05, 0.)))
            .collect(),
    };
    let scale = (rect.width() / 1.5).min(rect.height() / 1.5);
    let origin = rect.center() - v(0.675, 0.675) * scale;
    let at = |point: egui::Vec2| origin + point * scale;
    let wood = Color32::from_rgb(207, 165, 107).gamma_multiply(if enabled { 1. } else { 0.45 });
    let cut = if enabled {
        Color32::from_rgb(66, 140, 194)
    } else {
        ink
    };
    let mut path: Vec<_> = boundary.iter().copied().map(at).collect();
    let last = *boundary.last().unwrap();
    let final_span = [last, v(1.35, 0.)];
    for pair in boundary.windows(2).chain(std::iter::once(&final_span[..])) {
        if pair[1].x > pair[0].x {
            p.add(egui::Shape::convex_polygon(
                vec![
                    at(pair[0]),
                    at(pair[1]),
                    at(v(pair[1].x, 1.35)),
                    at(v(pair[0].x, 1.35)),
                ],
                wood,
                Stroke::NONE,
            ));
        }
    }
    path.extend([at(v(1.35, 0.)), at(v(1.35, 1.35)), at(v(0., 1.35))]);
    p.add(egui::Shape::closed_line(path, Stroke::new(1.2, ink)));
    p.add(egui::Shape::line(
        vec![at(v(0., 1.1)), at(v(0., 0.)), at(v(1.1, 0.))],
        Stroke::new(1., ink.gamma_multiply(0.25)),
    ));
    p.add(egui::Shape::line(
        boundary.into_iter().map(at).collect(),
        Stroke::new(2.2, cut),
    ));
}

fn fields(ui: &mut egui::Ui, settings: &mut Settings, surface: bool) -> ProfileInteraction {
    use crate::inspector_table::inspector_field;
    ui.text_label(tr!("router.profile"));
    let interaction = profile_selector(ui, &mut settings.profile);
    if settings.profile == Profile::None {
        return interaction;
    }
    let width = if surface {
        tr!("router.width")
    } else {
        match settings.profile {
            Profile::Roundover | Profile::Cove | Profile::Bead => tr!("router.radius"),
            Profile::Ogee => tr!("router.size"),
            _ => tr!("router.width"),
        }
    };
    size_field(ui, "width", width, &mut settings.width);
    if surface
        && !matches!(
            settings.profile,
            Profile::Bevel | Profile::Groove | Profile::Rabbet
        )
    {
        size_field(ui, "depth", tr!("router.depth"), &mut settings.depth);
    }
    match settings.profile {
        Profile::Bevel => {
            inspector_field(ui, "angle", tr!("router.angle"), |ui| {
                ui.add(
                    egui::DragValue::new(&mut settings.angle_degrees)
                        .range(Settings::ANGLE)
                        .suffix(tr!("joinery.degree_symbol")),
                )
            });
        }
        Profile::Rabbet | Profile::Groove => {
            size_field(ui, "depth", tr!("router.depth"), &mut settings.depth);
            if settings.profile == Profile::Groove && !surface {
                size_field(ui, "offset", tr!("router.offset"), &mut settings.offset);
            }
        }
        Profile::Bead => {
            size_field(ui, "shoulder", tr!("router.shoulder"), &mut settings.offset);
        }
        _ => {}
    }
    if !surface
        && matches!(
            settings.profile,
            Profile::Bevel | Profile::Rabbet | Profile::Groove
        )
    {
        ui.checkbox(&mut settings.reverse, tr!("router.reverse"));
    }
    interaction
}

impl BenchUpApp {
    pub(super) fn selected_route_at(&mut self, pos: Pos2) -> Option<u64> {
        if !self
            .selected
            .iter()
            .any(|id| self.doc.model.routes.contains_key(id))
        {
            return None;
        }
        let edge = self.router_edge_at(pos)?;
        (self.selected.contains(&edge) && self.doc.model.routes.contains_key(&edge)).then_some(edge)
    }

    pub(super) fn add_route_selection(
        &self,
        mesh: &mut renderer::Mesh,
        camera: &Camera,
        model: &Model,
        selected_color: Color32,
        hover_color: Color32,
    ) {
        if self.history_preview.is_some() {
            return;
        }
        for id in self
            .selected
            .iter()
            .copied()
            .chain(self.hover.filter(|id| !self.selected.contains(id)))
        {
            if let Some(control) = model.routing_appearance.edge_controls.get(&id)
                && !control.hidden
                && control.parent.is_none_or(|parent| !model.is_hidden(parent))
            {
                let color = if self.selected.contains(&id) {
                    selected_color
                } else {
                    hover_color
                };
                mesh.set_entity(Some(id));
                mesh.world_line(camera, control.points[0], control.points[1], 3., color);
                mesh.set_entity(None);
            }
        }
    }

    pub(super) fn close_router(&mut self) {
        if self.tool == Tool::Router {
            self.set_tool(Tool::Select);
        }
        self.router_editor = Default::default();
    }

    pub(super) fn router_edge_at(&mut self, pos: Pos2) -> Option<u64> {
        self.prepare_picking();
        let model = &self.doc.model;
        let (edge, _) = self.picking_cache.pick_edge(
            model,
            &self.face_cache,
            &self.entity_cache,
            &self.doc.camera,
            dv(self.viewport.size()),
            dv(pos - self.viewport.min),
            8.,
        )?;
        Target::for_edge(model, edge)?;
        model.entity_in_context(edge)?;
        (!model.is_hidden(edge)).then_some(edge)
    }

    pub(super) fn router_click(&mut self, pos: Pos2, ctx: &egui::Context) {
        let Some(edge) = self.router_edge_at(pos) else {
            self.say(ctx, Tool::Router.instructions());
            return;
        };
        self.inspect_router_edge(edge, ctx);
    }

    fn load_route_edges(&mut self, board: Target, edges: BTreeSet<u64>) {
        let model = &self.doc.model;
        let original: BTreeMap<_, _> = edges
            .into_iter()
            .filter(|id| Target::for_edge(model, *id) == Some(board))
            .map(|id| (id, model.routes.get(&id).copied()))
            .collect();
        let draft = original
            .iter()
            .map(|(&id, settings)| {
                (
                    id,
                    settings.unwrap_or(Settings {
                        profile: Profile::None,
                        ..self.router.settings
                    }),
                )
            })
            .collect();
        self.router_editor = Editor {
            reveal: self.router_editor.reveal,
            scroll_to: self.router_editor.scroll_to,
            selection: self.selected.clone(),
            board: Some(board),
            scope: model.edit_context,
            surface: original.keys().any(|id| model.route_on_surface(*id)),
            surface_revision: None,
            original,
            draft,
            validation: None,
            evaluation: Default::default(),
        };
    }

    fn route_lease_targets(&self, board: Target, edges: &BTreeSet<u64>) -> Option<BTreeSet<u64>> {
        let model = &self.doc.model;
        edges
            .iter()
            .map(|id| {
                (Target::for_edge(model, *id) == Some(board) && !model.is_hidden(*id))
                    .then(|| model.entity_in_context(*id))
                    .flatten()
            })
            .collect()
    }

    fn route_edges_editable(
        &self,
        ctx: &egui::Context,
        board: Target,
        edges: &BTreeSet<u64>,
    ) -> bool {
        !self.project.blocked()
            && self.history_preview.is_none()
            && self.shared_edit_allowed(ctx)
            && !board.protected(&self.doc.model, edges)
            && self
                .route_lease_targets(board, edges)
                .is_some_and(|targets| self.shared_fastener_edit_allowed(ctx, &targets))
    }

    fn validate_router_draft(&mut self, ctx: &egui::Context) {
        if self.router_editor.draft.is_empty() {
            self.router_editor.validation = None;
            return;
        }
        let revision = self.document_revision(ctx);
        let editor = &mut self.router_editor;
        if editor
            .validation
            .as_ref()
            .is_some_and(|preview| preview.revision == revision && preview.draft == editor.draft)
        {
            return;
        }
        let mut source = self.doc.model.clone();
        super::joinery::scope_joints_for_view(&mut source);
        apply_draft(&mut source, &editor.draft);
        editor.validation = Some(Validation {
            revision,
            draft: editor.draft.clone(),
            result: editor.evaluation.evaluate(&source),
        });
    }

    /// Reveal Properties once per tool activation or edge click. Disclosure
    /// state is then owned by the shared, immediately persisted preferences.
    pub(super) fn reveal_router_properties(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.router_editor.reveal) {
            return;
        }
        self.inspector = true;
        self.fasteners.open = false;
        self.joinery.open = false;
        sidebar::set_section_open(ctx, "entity_info", true);
        sidebar::set_section_open(ctx, "router", true);
        self.router_editor.scroll_to = true;
        ctx.request_repaint();
    }

    fn inspect_router_edge(&mut self, edge: u64, ctx: &egui::Context) {
        let Some(board) = Target::for_edge(&self.doc.model, edge) else {
            return;
        };
        let edges = BTreeSet::from([edge]);
        if let Some(targets) = self.route_lease_targets(board, &edges) {
            self.selected = targets;
        }
        self.load_route_edges(board, edges);
        self.router_editor.reveal = true;
        self.reveal_router_properties(ctx);
    }

    pub(super) fn router_selection_click(&mut self, ctx: &egui::Context, pos: Pos2, hit: u64) {
        // Selection still resolves grouped geometry to its normal selectable
        // owner. Keep the actual clicked edge only in the transient editor.
        let edge = if self.doc.model.edges.contains_key(&hit) {
            Some(hit)
        } else if self.doc.model.objects.contains_key(&hit) {
            self.router_edge_at(pos)
        } else {
            None
        };
        if let Some(edge) = edge
            && self
                .doc
                .model
                .entity_in_context(edge)
                .is_some_and(|id| self.selected.contains(&id))
        {
            let board = Target::for_edge(&self.doc.model, edge).unwrap();
            self.load_route_edges(board, BTreeSet::from([edge]));
            self.router_editor.reveal = true;
            self.reveal_router_properties(ctx);
        } else {
            self.router_editor = Default::default();
        }
    }

    pub(super) fn router_properties(&mut self, ui: &mut egui::Ui) {
        if self.router_editor.scope != self.doc.model.edit_context
            || self.router_editor.selection != self.selected
        {
            self.router_editor = Editor {
                scroll_to: self.router_editor.scroll_to,
                ..Default::default()
            };
        }
        let selected_edge = self
            .selected
            .iter()
            .copied()
            .find(|id| self.doc.model.edges.contains_key(id));
        if self.router_editor.board.is_none()
            && let Some(edge) = selected_edge
        {
            let board = Target::for_edge(&self.doc.model, edge).unwrap();
            self.load_route_edges(board, self.selected.clone());
        }
        if self.selected.is_empty()
            || !self
                .router_editor
                .draft
                .keys()
                .any(|id| self.doc.model.edges.contains_key(id))
        {
            self.router_editor.validation = None;
            return;
        }
        ui.add_space(8.);
        // Reuse the original stable key when nesting the former Router panel.
        let response =
            sidebar::inspector_section(ui, tr!("router.router"), "router", false, |ui| {
                self.router_contents(ui);
            });
        if std::mem::take(&mut self.router_editor.scroll_to) {
            response.inner.scroll_to_me(Some(egui::Align::Center));
        }
    }

    fn router_contents(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let scope = self.doc.model.edit_context;
        let board = self
            .router_editor
            .board
            .unwrap_or_else(|| scope.map_or(Target::Loose, Target::Object));
        if self.router_editor.board != Some(board) {
            self.load_route_edges(board, self.selected.clone());
        } else if self.router_editor.original.iter().any(|(id, original)| {
            Target::for_edge(&self.doc.model, *id) != Some(board)
                || self.doc.model.routes.get(id).copied() != *original
        }) {
            self.load_route_edges(board, self.router_editor.draft.keys().copied().collect());
        }
        let revision = self.document_revision(&ctx);
        if self.router_editor.surface_revision != Some(revision) {
            self.router_editor.surface = self
                .router_editor
                .draft
                .keys()
                .any(|id| self.doc.model.route_on_surface(*id));
            self.router_editor.surface_revision = Some(revision);
        }
        ui.text_weak(tr!("router.select_edges"));
        let mut settings = self
            .router_editor
            .draft
            .values()
            .next()
            .copied()
            .unwrap_or(self.router.settings);
        let previous = settings;
        let interaction = fields(ui, &mut settings, self.router_editor.surface);
        if interaction.activated || settings != previous {
            self.router.settings = settings;
            for draft in self.router_editor.draft.values_mut() {
                *draft = settings;
            }
            self.apply_router_profile(&ctx);
        }
    }

    fn apply_router_profile(&mut self, ctx: &egui::Context) -> bool {
        let Some(board) = self.router_editor.board else {
            return false;
        };
        let edges: BTreeSet<_> = self.router_editor.draft.keys().copied().collect();
        if edges.is_empty() {
            return false;
        }
        if !self.route_edges_editable(ctx, board, &edges) {
            self.say(ctx, tr!("router.protected"));
            return false;
        }
        if self
            .router_editor
            .draft
            .values()
            .all(|settings| settings.profile == Profile::None)
        {
            return self.commit_route_draft(ctx);
        }
        self.validate_router_draft(ctx);
        if let Some((id, error)) = self
            .router_editor
            .validation
            .as_ref()
            .unwrap()
            .result
            .errors
            .iter()
            .find(|(id, _)| Target::for_edge(&self.doc.model, *id) == Some(board))
        {
            self.say(ctx, tr!("router.edge_error", id = id, error = error));
            return false;
        }
        self.commit_route_draft(ctx)
    }

    // The caller checks the evaluated draft and editing permission before committing.
    fn commit_route_draft(&mut self, ctx: &egui::Context) -> bool {
        let before = self.doc.model.clone();
        apply_draft(&mut self.doc.model, &self.router_editor.draft);
        if self.doc.model == before {
            return true;
        }
        if let Err(error) = self.doc.model.validate_routes() {
            self.doc.model = before;
            self.say(ctx, error);
            return false;
        }
        self.finish_edit(tr!("router.applied"), before, ctx);
        for (id, original) in &mut self.router_editor.original {
            *original = self.doc.model.routes.get(id).copied();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ui::ContextExt as _;

    #[test]
    fn an_open_line_drawn_on_a_face_applies_a_surface_groove_only_on_activation() {
        for grouped in [false, true] {
            let mut model = Model::default();
            let corners = [DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y];
            for i in 0..4 {
                model.add_edge(corners[i], corners[(i + 1) % 4]).unwrap();
            }
            let face = *model.faces.keys().next().unwrap();
            let normal = benchup_core::faces::plane(&model, &model.faces[&face].vertices)
                .unwrap()
                .normal;
            model.push_pull(face, 0.1 / normal.z).unwrap();
            let group = grouped.then(|| {
                model
                    .make_object(&model.root_entities(), false, "Stock")
                    .unwrap()
            });
            model.edit_context = group;
            let face_count = model.faces.len();
            let edge = model
                .add_edge(DVec3::new(0.2, 0.5, 0.1), DVec3::new(0.8, 0.5, 0.1))
                .unwrap();
            assert_eq!(
                model.faces.len(),
                face_count,
                "an open path must not create a filled face"
            );
            model.edit_context = None;
            let mut app = BenchUpApp::with_document(
                Document {
                    model,
                    ..Default::default()
                },
                None,
                true,
            );
            let stock = app.doc.model.clone();
            let ctx = egui::Context::default();
            app.inspect_router_edge(edge, &ctx);
            assert!(app.router_editor.surface);
            let input = |events| egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(640., 1200.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run_test_ui(input(vec![]), |ui| app.router_properties(ui));
            let output = ctx.run_test_ui(input(vec![]), |ui| app.router_properties(ui));
            fn label(shape: &egui::Shape, text: &str) -> Option<Pos2> {
                match shape {
                    egui::Shape::Text(t) if t.galley.job.text == text => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| label(s, text)),
                    _ => None,
                }
            }
            let card = output
                .shapes
                .iter()
                .find_map(|s| label(&s.shape, Profile::Groove.label()))
                .unwrap();
            let _ = ctx.run_test_ui(input(vec![egui::Event::PointerMoved(card)]), |ui| {
                app.router_properties(ui)
            });
            assert!(
                app.router_editor.validation.is_none(),
                "hover must not evaluate a routing preview"
            );
            assert_eq!(app.doc.model, stock);
            assert!(app.doc.history.is_empty());
            for pressed in [true, false] {
                let _ = ctx.run_test_ui(
                    input(vec![
                        egui::Event::PointerMoved(card),
                        egui::Event::PointerButton {
                            pos: card,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]),
                    |ui| app.router_properties(ui),
                );
            }
            assert_eq!(app.doc.model.routes[&edge].profile, Profile::Groove);
            assert_eq!(app.doc.model.vertices, stock.vertices);
            assert!(app.doc.model.evaluate_joinery().errors.is_empty());
            let output = ctx.run_test_ui(input(vec![]), |ui| app.router_properties(ui));
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|s| label(&s.shape, tr!("router.depth")).is_some())
            );
            assert!(app.doc.history.undo(&mut app.doc.model));
            assert!(app.doc.model.routes.is_empty());
        }
    }

    #[test]
    fn router_section_requires_an_edge_selection_even_with_router_tool_active() {
        for tool in [Tool::Select, Tool::Router] {
            let mut model = Model::default();
            let edge = model.add_edge(DVec3::ZERO, DVec3::X).unwrap();
            model.add_edge(DVec3::X, DVec3::Y).unwrap();
            model.add_edge(DVec3::Y, DVec3::ZERO).unwrap();
            let face = *model.faces.keys().next().unwrap();
            let mut app = BenchUpApp::with_document(
                Document {
                    model,
                    ..Default::default()
                },
                None,
                true,
            );
            app.set_tool(tool);
            let ctx = egui::Context::default();
            let mut height = 0.;
            let mut render = |app: &mut BenchUpApp| {
                let _ = ctx.run_test_ui(egui::RawInput::default(), |ui| {
                    let top = ui.cursor().min.y;
                    app.router_properties(ui);
                    height = ui.cursor().min.y - top;
                });
                height
            };
            assert_eq!(render(&mut app), 0.);
            app.selected.insert(face);
            assert_eq!(render(&mut app), 0.);
            app.selected = BTreeSet::from([edge]);
            assert!(render(&mut app) > 0.);
            app.selected.clear();
            assert_eq!(render(&mut app), 0.);
            assert!(app.router_editor.validation.is_none());
            assert!(app.doc.history.is_empty());
        }
    }

    #[test]
    fn router_validates_without_editing_stock_and_commits_one_undo_step() {
        let mut model = Model::default();
        let corners = [DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y];
        for i in 0..4 {
            model.add_edge(corners[i], corners[(i + 1) % 4]).unwrap();
        }
        let face = *model.faces.keys().next().unwrap();
        model.push_pull(face, 0.04).unwrap();
        let board = model
            .make_object(&model.root_entities(), false, "Board")
            .unwrap();
        let edge = *model.edges.keys().next().unwrap();
        // Exercise the large-scene cache path with unrelated geometry.
        model.paste_entities(
            &crate::picking::tests::triangle_grid(1024),
            DVec3::splat(5.),
        );
        let source_volume = model.solid_volume(board).unwrap();
        assert!(model.enter_object(board));
        let doc = Document {
            model,
            ..Default::default()
        };
        let mut app = BenchUpApp::with_document(doc, None, true);
        assert!(app.doc.model.enter_object(board) || app.doc.model.edit_context == Some(board));
        app.selected.insert(edge);
        app.set_tool(Tool::Router);
        let ctx = egui::Context::default();
        app.load_route_edges(Target::Object(board), BTreeSet::from([edge]));
        for settings in app.router_editor.draft.values_mut() {
            settings.profile = Profile::Chamfer;
        }
        app.validate_router_draft(&ctx);
        let mut preview = app
            .router_editor
            .validation
            .as_ref()
            .unwrap()
            .result
            .model
            .clone();
        preview.edit_context = None;
        assert!(preview.solid_volume(board).unwrap() < source_volume);
        assert!(app.doc.model.routes.is_empty());
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
        let render = |app: &mut BenchUpApp| {
            let _ = ctx.run_test_ui(
                egui::RawInput {
                    screen_rect: Some(rect),
                    ..Default::default()
                },
                |ui| app.paint_scene(ui, rect),
            );
            assert!(
                app.render_face_cache.iter().next().is_some()
                    || app.face_cache.iter().next().is_some()
            );
        };
        render(&mut app);
        let stock = app.doc.model.vertices.clone();
        app.commit_route_draft(&ctx);
        assert!(app.doc.model.routes.contains_key(&edge));
        assert_eq!(app.doc.model.vertices, stock);
        let mut reopened = BenchUpApp::with_document(app.doc.clone(), None, true);
        reopened.doc.model.edit_context = None;
        render(&mut reopened);
        let machined = reopened.joinery_geometry.result.as_ref().unwrap();
        assert!(machined.errors.is_empty());
        assert!(!machined.model.edges.contains_key(&edge));
        assert!(reopened.doc.model.enter_object(board));
        render(&mut reopened);
        let stock_in_scope = reopened.joinery_geometry.result.as_ref().unwrap();
        assert!(stock_in_scope.model.edges.contains_key(&edge));
        assert!(app.doc.history.undo(&mut app.doc.model));
        assert!(app.doc.model.routes.is_empty());
    }
}
