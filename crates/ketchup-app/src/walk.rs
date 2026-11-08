use super::*;
#[path = "walk_physics.rs"]
mod physics;
pub(super) use physics::Settings;
use physics::{Person, World};

#[derive(Default)]
pub(super) struct Walk {
    pub person: Option<Person>,
    // Reusable posture preference, without carrying a project's position into
    // another model. The selected Walk tool restores to face-placement mode.
    pub sitting: bool,
    world: World,
    geometry: Option<(u64, Vec<u64>)>,
    last_time: Option<f64>,
    alt_down: bool,
    look_drag: bool,
    settings: Option<Settings>,
    camera: Option<Camera>,
    pub(super) original_camera: Option<Camera>,
}

impl Walk {
    pub fn reset(&mut self) {
        self.person = None;
        self.world = World::default();
        self.geometry = None;
        self.last_time = None;
        self.alt_down = false;
        self.look_drag = false;
        self.settings = None;
        self.camera = None;
        self.original_camera = None;
    }
}

impl KetchupApp {
    pub(super) fn walk_button(&mut self, ui: &mut egui::Ui) {
        let icon = egui::Atom::paint(egui::vec2(20., 20.), |ui, args| {
            icons::tool(ui.painter(), args.rect, Tool::Walk)
        });
        let response = ui
            .add(
                egui::Button::new((icon, "Walk"))
                    .min_size(egui::vec2(0., 32.))
                    .selected(self.tool == Tool::Walk),
            )
            .on_hover_text(
                "Walk through the model. Click a floor face to place yourself at eye level.",
            );
        if response.clicked() {
            response.surrender_focus();
            if self.tool == Tool::Walk {
                self.exit_walk(ui.ctx());
            } else {
                self.set_tool(Tool::Walk);
            }
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(canvas) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("ketchup-canvas"))
        {
            let _ = canvas.set_attribute(
                "data-walk-active",
                if self.tool == Tool::Walk {
                    "true"
                } else {
                    "false"
                },
            );
        }
    }

    pub(super) fn walk_status(&self) -> &'static str {
        match self.walk.person.as_ref() {
            None => Tool::Walk.instructions(),
            Some(person) if person.sitting => {
                "Seated · Drag to look (180°) · Alt: stand · Escape: exit Walk"
            }
            Some(_) => {
                "WASD / arrows: walk · Drag to look · Ctrl: crouch · Space: jump · Alt: sit · Escape: exit Walk"
            }
        }
    }

    fn walk_input_blocked(&self, ctx: &egui::Context) -> bool {
        let blocked = self.project.blocked()
            || self.settings_open
            || self.screenshot.open
            || self.help
            || self.about
            || self.release_notes.open
            || self.account.open
            || self.object_creation.is_some()
            || self.object_rename.is_some()
            || self.viewports.renaming.is_some()
            || self.text_editor.is_some()
            || self.joinery.open
            || self.fasteners.open
            || self.sharing.preferences.open
            || egui::Popup::is_any_open(ctx)
            || ctx.text_edit_focused()
            || !ctx.input(|i| i.focused);
        #[cfg(not(target_arch = "wasm32"))]
        let blocked = blocked || self.updates.open;
        blocked
    }

    pub(super) fn walk_keyboard(&mut self, ctx: &egui::Context) {
        if self.walk_input_blocked(ctx) {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.exit_walk(ctx);
        }
    }

    fn exit_walk(&mut self, ctx: &egui::Context) {
        if let Some(camera) = self.walk.original_camera.take() {
            if self.doc.camera != camera {
                self.doc.camera = camera;
                self.changed(ctx);
            }
        }
        self.set_tool(Tool::Select);
        ctx.request_repaint();
    }

    fn prepare_walk_world(&mut self) {
        // Use the same evaluated faces as the viewport, including holes,
        // transformed groups, and joinery. Visibility affects placement and
        // obstacles consistently; locked geometry still blocks walking.
        let (_, faces, entities, _) = self
            .joinery_geometry
            .evaluated_picking(&self.doc.model, &self.doc.display);
        let visible: Vec<_> = faces
            .iter()
            .filter(|(id, _)| entities.get(*id).displayed(&self.doc.display))
            .map(|(id, _)| id)
            .collect();
        let key = (faces.revision(), visible);
        if self.walk.geometry.as_ref() != Some(&key) {
            self.walk.world = World::new(
                key.1
                    .iter()
                    .flat_map(|id| faces.get(*id).triangles.iter().copied()),
            );
            self.walk.geometry = Some(key);
            // Live model edits may remove the floor or place a wall through the
            // person. Require a new valid placement rather than trapping them.
            self.walk.person = None;
        }
    }

    pub(super) fn walk_place(&mut self, ctx: &egui::Context, pos: Pos2) {
        self.prepare_walk_world();
        let ray = self
            .doc
            .camera
            .ray(dv(pos - self.viewport.min), dv(self.viewport.size()));
        let (_, faces, entities, picking) = self
            .joinery_geometry
            .evaluated_picking(&self.doc.model, &self.doc.display);
        let hit = picking.nearest_unscoped(faces, entities, ray);
        let Some((id, _, point)) = hit else {
            self.say(ctx, "Click a floor face to place the walking person.");
            return;
        };
        if faces.get(id).normal.z.abs() + 1e-10 < std::f64::consts::FRAC_1_SQRT_2 {
            self.say(ctx, "Choose a floor or a slope of 45° or less.");
            return;
        }
        let settings = &self.general_settings.walk;
        let Some(person) = Person::place(
            &self.walk.world,
            point,
            self.doc.camera.yaw,
            settings,
            self.walk.sitting,
        ) else {
            self.say(
                ctx,
                "There is not enough space here for the configured body size and eye height.",
            );
            return;
        };
        self.walk.person = Some(person);
        self.walk.settings = Some(settings.clone());
        self.walk.last_time = Some(ctx.input(|i| i.time));
        self.walk.alt_down = ctx.input(|i| i.modifiers.alt);
        self.doc.camera.pitch = 0.;
        self.doc.camera.projection = ketchup_core::Projection::Perspective;
        self.hover = None;
        self.snap = None;
        self.notice = None;
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        self.sync_walk_camera();
        self.changed(ctx);
    }

    fn sync_walk_camera(&mut self) {
        let Some(person) = &self.walk.person else {
            return;
        };
        let eye = person.feet + DVec3::Z * person.eye_height(&self.general_settings.walk);
        let camera = &mut self.doc.camera;
        camera.yaw = person.yaw;
        camera.projection = ketchup_core::Projection::Perspective;
        // Keep the eye fixed as the target changes when turning the head.
        camera.distance = 1.;
        camera.target = eye
            - DVec3::new(
                camera.pitch.cos() * camera.yaw.cos(),
                camera.pitch.cos() * camera.yaw.sin(),
                camera.pitch.sin(),
            );
        self.walk.camera = Some(camera.clone());
    }

    pub(super) fn walk_viewport_input(&mut self, ctx: &egui::Context, response: &egui::Response) {
        let now = ctx.input(|i| i.time);
        let dt = self
            .walk
            .last_time
            .replace(now)
            .map_or(0., |last| (now - last).clamp(0., 0.1));
        if !response.enabled() || self.walk_input_blocked(ctx) {
            self.walk.look_drag = false;
            self.walk.alt_down = ctx.input(|i| i.modifiers.alt);
            return;
        }
        if self.walk.person.is_some()
            && self.walk.camera.as_ref().is_some_and(|camera| {
                camera.target != self.doc.camera.target
                    || camera.distance != self.doc.camera.distance
                    || camera.yaw != self.doc.camera.yaw
                    || camera.pitch != self.doc.camera.pitch
                    || camera.projection != self.doc.camera.projection
            })
        {
            // Fit, standard views and projection changes take over explicitly.
            // FOV adjustments are allowed while continuing to walk.
            self.set_tool(Tool::Select);
            return;
        }
        self.prepare_walk_world();
        if self
            .walk
            .settings
            .as_ref()
            .is_some_and(|saved| saved != &self.general_settings.walk)
        {
            if self
                .walk
                .person
                .as_ref()
                .is_some_and(|person| !person.fits(&self.walk.world, &self.general_settings.walk))
            {
                self.walk.person = None;
                self.say(
                    ctx,
                    "The new body dimensions do not fit here. Click a floor face to reposition.",
                );
            }
            self.walk.settings = Some(self.general_settings.walk.clone());
        }
        if self.walk.person.is_none() {
            // Walk is a placement tool until a face has been accepted. Empty
            // space, edges, and walls never silently become an implicit floor.
            if response.clicked_by(egui::PointerButton::Primary)
                && let Some(pos) = response.interact_pointer_pos()
            {
                self.walk_place(ctx, pos);
            }
            return;
        }
        let settings = self.general_settings.walk.clone();
        let (alt, crouch, jump, direction, delta, held, pressed) = ctx.input(|i| {
            let axis = |positive, negative| if positive { 1. } else { 0. } - if negative { 1. } else { 0. };
            let jump = i.events.iter().any(|event| matches!(event, egui::Event::Key { key: Key::Space, pressed: true, repeat: false, .. }));
            (i.modifiers.alt, i.modifiers.ctrl, jump,
                DVec2::new(axis(i.key_down(Key::D) || i.key_down(Key::ArrowRight), i.key_down(Key::A) || i.key_down(Key::ArrowLeft)),
                    axis(i.key_down(Key::W) || i.key_down(Key::ArrowUp), i.key_down(Key::S) || i.key_down(Key::ArrowDown))),
                i.pointer.delta(),
                i.pointer.primary_down() || i.pointer.secondary_down(),
                i.pointer.any_pressed())
        });
        if pressed && response.contains_pointer() {
            self.walk.look_drag = true;
        }
        if !held {
            self.walk.look_drag = false;
        }
        let before = self.doc.camera.clone();
        let person = self.walk.person.as_mut().unwrap();
        if alt && !self.walk.alt_down {
            if !person.toggle_sit(&self.walk.world, &settings) {
                self.notice = Some((
                    "Cannot change posture here: check head clearance and floor support.".into(),
                    now + 5.,
                ));
            }
            self.walk.sitting = person.sitting;
        }
        self.walk.alt_down = alt;
        if self.walk.look_drag && !pressed {
            person.turn(-delta.x as f64 * 0.004, &self.walk.world, &settings);
            self.doc.camera.pitch = (self.doc.camera.pitch + delta.y as f64 * 0.004)
                .clamp(-80_f64.to_radians(), 80_f64.to_radians());
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        person.step(&self.walk.world, &settings, direction, crouch, jump, dt);
        self.sync_walk_camera();
        if self.doc.camera != before {
            self.changed(ctx);
        }
        // Gravity, held movement, and modifier transitions need frames even
        // when the operating system does not generate repeated key events.
        ctx.request_repaint_after(Duration::from_secs_f64(1. / 60.));
    }
}

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
