use benchup_core::tr;
use super::*;
#[path = "walk_physics.rs"]
mod physics;
pub(super) use physics::Settings;
use physics::{Gait, Person, World};

#[derive(Default)]
pub(super) struct Walk {
    pub person: Option<Person>,
    // Reusable posture preference, without carrying a project's position into
    // another model. The selected Walk tool restores to face-placement mode.
    pub sitting: bool,
    world: World,
    geometry: Option<(u64, Option<u64>, bool, bool)>,
    gait: Gait,
    #[cfg(test)]
    pub(super) world_builds: usize,
    last_time: Option<f64>,
    alt_down: bool,
    #[cfg(not(target_arch = "wasm32"))]
    cursor_context: Option<egui::Context>,
    settings: Option<Settings>,
    camera: Option<Camera>,
    pub(super) original_camera: Option<Camera>,
}

impl Walk {
    pub fn reset(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.release_cursor();
        self.person = None;
        self.world = World::default();
        self.geometry = None;
        self.gait = Gait::default();
        self.last_time = None;
        self.alt_down = false;
        self.settings = None;
        self.camera = None;
        self.original_camera = None;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn release_cursor(&mut self) {
        if let Some(ctx) = self.cursor_context.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        }
    }
}

impl BenchUpApp {
    pub(super) fn walk_button(&mut self, ui: &mut egui::Ui) {
        let icon = egui::Atom::paint(egui::vec2(20., 20.), |ui, args| {
            icons::tool(ui.painter(), args.rect, Tool::Walk)
        });
        let response = ui
            .add(
                egui::Button::new((icon, tr!("walk.walk")))
                    .min_size(egui::vec2(0., 32.))
                    .selected(self.tool == Tool::Walk),
            )
            .on_hover_text(
                tr!("walk.walk_through_the_model_click_a_floor_face_to_place_yourself_at_eye_level"),
            );
        if response.clicked() {
            response.surrender_focus();
            if self.tool == Tool::Walk {
                self.exit_walk(ui.ctx());
            } else {
                self.set_tool(Tool::Walk);
            }
        }
    }

    // Run after dialogs and viewport input, so capture reflects the final state
    // of the frame. Capture itself is transient, never a saved user preference.
    pub(super) fn sync_walk_cursor(&mut self, ctx: &egui::Context) {
        let enabled = self.tool == Tool::Walk && !self.walk_input_blocked(ctx);
        let placed = self.tool == Tool::Walk && self.walk.person.is_some();
        #[cfg(not(target_arch = "wasm32"))]
        if enabled && placed {
            if self.walk.cursor_context.is_none() {
                // X11 supports confinement rather than locked positioning.
                // Raw device motion still allows unlimited turning at edges.
                #[cfg(target_os = "linux")]
                let grab = egui::CursorGrab::Confined;
                #[cfg(not(target_os = "linux"))]
                let grab = egui::CursorGrab::Locked;
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(grab));
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
                self.walk.cursor_context = Some(ctx.clone());
            }
        } else {
            self.walk.release_cursor();
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(canvas) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("benchup-canvas"))
        {
            // Attribute writes notify the browser bridge even when their
            // values do not change. Publish only transitions, so ordinary
            // editor frames do not repeatedly release capture or unlock keys.
            for (key, enabled) in [
                ("data-walk-active", self.tool == Tool::Walk),
                ("data-walk-input-enabled", enabled),
                ("data-walk-placed", placed),
            ] {
                let value = if enabled { "true" } else { "false" };
                if canvas.get_attribute(key).as_deref() != Some(value) {
                    let _ = canvas.set_attribute(key, value);
                }
            }
        }
    }

    pub(super) fn walk_status(&self) -> &'static str {
        match self.walk.person.as_ref() {
            None => Tool::Walk.instructions(),
            Some(person) if person.sitting => {
                tr!("walk.seated_move_mouse_to_look_180_alt_stand_tab_sidebar_escape_exit_walk")
            }
            Some(_) => {
                tr!("walk.walking_controls")
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
        #[cfg(target_arch = "wasm32")]
        if let Some(canvas) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("benchup-canvas"))
        {
            if canvas.get_attribute("data-walk-exit-requested").as_deref() == Some("true") {
                let _ = canvas.remove_attribute("data-walk-exit-requested");
                self.exit_walk(ctx);
                return;
            }
        }
        if self.walk_input_blocked(ctx) {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.exit_walk(ctx);
            return;
        }
        self.sidebar_shortcut(ctx);
    }

    fn exit_walk(&mut self, ctx: &egui::Context) {
        if let Some(camera) = self.walk.original_camera.take() {
            if self.doc.camera != camera {
                self.doc.camera = camera;
                self.camera_changed(ctx);
            }
        }
        self.set_tool(Tool::Select);
        ctx.request_repaint();
    }

    fn prepare_walk_world(&mut self, ctx: &egui::Context) {
        let key = (
            self.document_revision(ctx),
            self.doc.model.edit_context,
            self.doc.display.hidden_geometry,
            self.doc.display.hidden_objects,
        );
        if self.walk.geometry == Some(key) {
            return;
        }
        // Use the same evaluated faces as the viewport, including holes,
        // transformed groups, and joinery. Visibility affects placement and
        // obstacles consistently; locked geometry still blocks walking.
        let (faces, entities) = if !self.doc.model.has_machining() && self.doc.model.connections.is_empty() {
            self.prepare_picking();
            (&self.face_cache, &self.entity_cache)
        } else {
            let (_, faces, entities, _) = self
                .joinery_geometry
                .evaluated_picking(&self.doc.model, &self.doc.display);
            (faces, entities)
        };
        self.walk.world = World::new(
            faces
                .iter()
                .filter(|(id, _)| entities.get(*id).displayed(&self.doc.display))
                .flat_map(|(_, face)| face.triangles.iter().copied()),
        );
        self.walk.geometry = Some(key);
        #[cfg(test)]
        {
            self.walk.world_builds += 1;
        }
        // Live edits can remove a floor or put a wall through the character.
        self.walk.person = None;
        self.walk.gait = Gait::default();
    }

    pub(super) fn walk_place(&mut self, ctx: &egui::Context, pos: Pos2) {
        self.prepare_walk_world(ctx);
        let ray = self
            .doc
            .camera
            .ray(dv(pos - self.viewport.min), dv(self.viewport.size()));
        let (faces, entities, picking) = if !self.doc.model.has_machining() && self.doc.model.connections.is_empty() {
            self.prepare_picking();
            (&self.face_cache, &self.entity_cache, &self.picking_cache)
        } else {
            let (_, faces, entities, picking) = self
                .joinery_geometry
                .evaluated_picking(&self.doc.model, &self.doc.display);
            (faces, entities, &*picking)
        };
        let hit = picking.nearest_unscoped(faces, entities, ray);
        let Some((id, _, point)) = hit else {
            self.say(ctx, tr!("walk.click_a_floor_face_to_place_the_walking_person"));
            return;
        };
        if faces.get(id).normal.z.abs() + 1e-10 < std::f64::consts::FRAC_1_SQRT_2 {
            self.say(ctx, tr!("walk.choose_a_floor_or_a_slope_of_45_or_less"));
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
                tr!("walk.there_is_not_enough_space_here_for_the_configured_body_size_and_eye_height"),
            );
            return;
        };
        self.walk.person = Some(person);
        self.walk.settings = Some(settings.clone());
        self.walk.last_time = Some(ctx.input(|i| i.time));
        self.walk.alt_down = ctx.input(|i| i.modifiers.alt);
        self.doc.camera.pitch = 0.;
        self.doc.camera.projection = benchup_core::Projection::Perspective;
        self.hover = None;
        self.snap = None;
        self.notice = None;
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        self.sync_walk_camera();
        self.camera_changed(ctx);
    }

    fn sync_walk_camera(&mut self) {
        let Some(person) = &self.walk.person else {
            return;
        };
        let eye = person.feet
            + DVec3::Z * (person.eye_height(&self.general_settings.walk) + self.walk.gait.offset);
        let camera = &mut self.doc.camera;
        camera.yaw = person.yaw;
        camera.projection = benchup_core::Projection::Perspective;
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
        self.prepare_walk_world(ctx);
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
                    tr!("walk.the_new_body_dimensions_do_not_fit_here_click_a_floor_face_to_reposition"),
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
        let delta = self.walk_mouse_motion(ctx, response);
        let (alt, crouch, jump, direction) = ctx.input(|i| {
            let axis = |positive, negative| if positive { 1. } else { 0. } - if negative { 1. } else { 0. };
            let jump = i.events.iter().any(|event| matches!(event, egui::Event::Key { key: Key::Space, pressed: true, repeat: false, .. }));
            (i.modifiers.alt, i.modifiers.shift, jump,
                DVec2::new(axis(i.key_down(Key::D) || i.key_down(Key::ArrowRight), i.key_down(Key::A) || i.key_down(Key::ArrowLeft)),
                    axis(i.key_down(Key::W) || i.key_down(Key::ArrowUp), i.key_down(Key::S) || i.key_down(Key::ArrowDown))))
        });
        let before = self.doc.camera.clone();
        let person = self.walk.person.as_mut().unwrap();
        if alt && !self.walk.alt_down {
            if !person.toggle_sit(&self.walk.world, &settings) {
                self.notice = Some((
                    tr!("walk.cannot_change_posture_here_check_head_clearance_and_floor_support").into(),
                    now + 5.,
                ));
            }
            self.walk.sitting = person.sitting;
        }
        self.walk.alt_down = alt;
        if delta != Vec2::ZERO {
            let sensitivity = 0.007 * self.general_settings.orbit_speed * settings.look_sensitivity;
            person.turn(-delta.x as f64 * sensitivity);
            self.doc.camera.pitch = (self.doc.camera.pitch + delta.y as f64 * sensitivity)
                .clamp(-80_f64.to_radians(), 80_f64.to_radians());
        }
        let feet = person.feet;
        person.step(&self.walk.world, &settings, direction, crouch, jump, dt);
        self.walk.gait.update(
            (person.feet - feet).truncate().length(),
            dt,
            person.grounded,
            person.crouched,
            person.sitting,
        );
        self.sync_walk_camera();
        if self.doc.camera != before {
            self.camera_changed(ctx);
        }
        // Gravity, held movement, and modifier transitions need frames even
        // when the operating system does not generate repeated key events.
        ctx.request_repaint_after(Duration::from_secs_f64(1. / 60.));
    }

    fn walk_mouse_motion(&self, ctx: &egui::Context, response: &egui::Response) -> Vec2 {
        #[cfg(target_arch = "wasm32")]
        if let Some(canvas) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("benchup-canvas"))
        {
            if canvas.get_attribute("data-walk-pointer-capture").as_deref() == Some("locked") {
                // eframe's web backend only supplies absolute positions, which
                // stay fixed under pointer lock. Consume the JS movement deltas.
                let mut delta = Vec2::ZERO;
                for (axis, key) in ["data-walk-look-x", "data-walk-look-y"].iter().enumerate() {
                    delta[axis] = canvas
                        .get_attribute(key)
                        .and_then(|v| v.parse::<f32>().ok())
                        .filter(|v| v.is_finite())
                        .unwrap_or(0.);
                    let _ = canvas.set_attribute(key, "0");
                }
                return delta / ctx.zoom_factor();
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.walk.cursor_context.is_some() {
            return ctx.input(|i| i.pointer.motion().unwrap_or_else(|| i.pointer.delta()));
        }
        if response.hovered() {
            ctx.input(|i| i.pointer.motion().unwrap_or_else(|| i.pointer.delta()))
        } else {
            Vec2::ZERO
        }
    }
}

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
