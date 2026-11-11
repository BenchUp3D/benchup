//! Image placement uses the same rectangle inference and locks as drawing.
use benchup_core::tr;
use super::*;
use benchup_core::{ImageAsset, ImageSettings, ReferenceImage};
use std::{
    io::Cursor,
    sync::{Arc, mpsc},
};

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub appearance: ImageSettings,
    pub offset: DVec3,
}
impl Preferences {
    pub fn sanitize(&mut self) {
        if !self.appearance.valid() {
            self.appearance = Default::default();
        }
        if !self.offset.is_finite() || self.offset.abs().max_element() > 1e8 {
            self.offset = DVec3::ZERO;
        }
    }
}
#[derive(Debug)]
struct ImportedImage {
    image: ReferenceImage,
    asset: ImageAsset,
}
struct Pending {
    result: mpsc::Receiver<Result<Option<ImportedImage>, String>>,
    points: [DVec3; 4],
    replace: Option<u64>,
    context: Option<u64>,
    project: String,
}
struct Adjustment {
    id: u64,
    source: ReferenceImage,
    settings: ImageSettings,
    context: Option<u64>,
    control: egui::Id,
    rect: egui::Rect,
    dragging: bool,
}
#[derive(Default)]
pub(super) struct State {
    pub preferences: Preferences,
    pending: Option<Pending>,
    adjustment: Option<Adjustment>,
    pub gpu: crate::reference_image_gpu::Cache,
}

/// Bound both decoding and GPU memory. Large photos are resampled once at import.
fn decode_image(
    name: String,
    bytes: &[u8],
    preferences: &Preferences,
) -> Result<ImportedImage, String> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(tr!("image_tools.choose_an_image_smaller_than_32_mb").into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    use image::ImageDecoder as _;
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| tr!("image_tools.could_not_open_image", e = e))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut source = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| tr!("image_tools.could_not_open_image", e = e))?;
    source.apply_orientation(orientation);
    let source = if source.width() > 4096 || source.height() > 4096 {
        source.resize(4096, 4096, image::imageops::FilterType::Lanczos3)
    } else {
        source
    };
    let pixels = [source.width(), source.height()];
    let mut png = Cursor::new(Vec::new());
    source
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    if png.get_ref().len() > 32 * 1024 * 1024 {
        return Err(tr!("image_tools.the_decoded_image_is_too_large_choose_a_smaller_image").into());
    }
    Ok(ImportedImage {
        image: ReferenceImage {
            name,
            asset: benchup_core::reference_images::ImageAssetId::new_v4(),
            vertices: [0; 4],
            settings: preferences.appearance.clone(),
            placement_offset: preferences.offset,
        },
        asset: ImageAsset {
            png: Arc::from(png.into_inner()),
            pixels,
        },
    })
}

impl BenchUpApp {
    #[cfg(test)]
    pub(super) fn pending_image_rectangle(&self) -> Option<[DVec3; 4]> {
        self.images.pending.as_ref().map(|p| p.points)
    }
    pub(super) fn reset_image_project(&mut self) {
        self.images.pending = None;
        self.images.adjustment = None;
        self.images.gpu = Default::default();
    }
    /// Appearance is a shader-only preview until the drag or text edit ends.
    /// In particular, do not clone the scene, rebuild geometry, record history,
    /// or synchronously save the document for every slider movement.
    pub(super) fn image_appearance_preview(&self) -> Option<(u64, &ImageSettings)> {
        self.images
            .adjustment
            .as_ref()
            .map(|edit| (edit.id, &edit.settings))
    }

    pub(super) fn finish_image_adjustment(&mut self, ctx: &egui::Context) {
        self.settle_image_adjustment(ctx, true);
    }

    pub(super) fn prepare_image_adjustment(&mut self, ctx: &egui::Context) {
        self.settle_image_adjustment(ctx, false);
    }

    fn settle_image_adjustment(&mut self, ctx: &egui::Context, finish_inactive: bool) {
        let Some(edit) = self.images.adjustment.as_ref() else {
            return;
        };
        if self.doc.model.edit_context != edit.context
            || self.doc.model.reference_images.get(&edit.id) != Some(&edit.source)
            || self.doc.model.is_locked(edit.id)
        {
            self.images.adjustment = None;
            return;
        }
        let focused = ctx.memory(|m| m.has_focus(edit.control));
        let ready = ctx.input(|i| {
            !i.focused
                || (finish_inactive && !i.pointer.primary_down() && (edit.dragging || !focused))
                || (i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !edit.rect.contains(p)))
                || i.key_pressed(egui::Key::Escape)
                || (i.modifiers.command
                    && i.events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Key { pressed: true, .. })))
        });
        if ready {
            self.commit_image_adjustment(ctx);
        }
    }

    fn commit_image_adjustment(&mut self, ctx: &egui::Context) {
        let Some(edit) = self.images.adjustment.take() else {
            return;
        };
        // A replacement, history navigation, scope change or remote edit must
        // never have its image state overwritten by an obsolete preview.
        if self.doc.model.edit_context != edit.context
            || self.doc.model.reference_images.get(&edit.id) != Some(&edit.source)
            || self.doc.model.is_locked(edit.id)
            || edit.settings == edit.source.settings
            || !edit.settings.valid()
        {
            return;
        }
        if !self.shared_edit_allowed(ctx) {
            self.say(ctx, self.shared_edit_block_reason(ctx));
            return;
        }
        let linked = self.doc.model.edit_path().iter().any(|id| {
            matches!(
                self.doc.model.objects[id].kind,
                benchup_core::ObjectKind::Component { .. }
            )
        });
        if !linked {
            self.doc
                .model
                .reference_images
                .get_mut(&edit.id)
                .unwrap()
                .settings = edit.settings.clone();
            let author = self.current_history_author();
            self.doc.history.record_image_settings_applied(
                edit.id,
                edit.source,
                edit.settings,
                Some(author),
                Some(sharing::now_timestamp()),
            );
            self.finish_recorded_edit(ctx, false);
            return;
        }
        let before = self.doc.model.clone();
        self.doc
            .model
            .reference_images
            .get_mut(&edit.id)
            .unwrap()
            .settings = edit.settings;
        self.finish_context_state_edit(tr!("image_tools.edit_reference_image"), before, ctx);
    }

    fn adjust_image_appearance(
        &mut self,
        id: u64,
        settings: ImageSettings,
        active: Option<(egui::Id, egui::Rect, bool)>,
        ctx: &egui::Context,
    ) {
        if !self.shared_edit_allowed(ctx) {
            self.images.adjustment = None;
            self.say(ctx, self.shared_edit_block_reason(ctx));
            return;
        }
        let source = self.doc.model.reference_images[&id].clone();
        let (control, rect, dragging) =
            active.unwrap_or((egui::Id::NULL, egui::Rect::NOTHING, false));
        self.images.adjustment = Some(Adjustment {
            id,
            source,
            settings,
            context: self.doc.model.edit_context,
            control,
            rect,
            dragging,
        });
        if active.is_none() {
            self.commit_image_adjustment(ctx);
        }
        ctx.request_repaint();
    }
    pub(super) fn choose_reference_image(
        &mut self,
        ctx: &egui::Context,
        points: [DVec3; 4],
        replace: Option<u64>,
    ) -> bool {
        if self.images.pending.is_some() {
            self.say(ctx, tr!("image_tools.finish_or_cancel_the_current_image_picker_first"));
            return false;
        }
        let (sender, result) = mpsc::channel();
        #[cfg(not(test))]
        let preferences = self.images.preferences.clone();
        #[cfg(not(test))]
        let repaint = ctx.clone();
        #[cfg(test)]
        {
            let _ = ctx;
            let _ = sender.send(Ok(None));
        }
        #[cfg(all(not(test), not(target_arch = "wasm32")))]
        benchup_core::i18n::spawn_localized(move || {
            let result = (|| {
                let Some(path) = rfd::FileDialog::new()
                    .set_title(tr!("image_tools.choose_reference_image"))
                    .add_filter(tr!("image_tools.images_png_jpeg_webp"), &["png", "jpg", "jpeg", "webp"])
                    .pick_file()
                else {
                    return Ok(None);
                };
                if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
                    return Err(tr!("image_tools.choose_an_image_smaller_than_32_mb").into());
                }
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                decode_image(
                    name,
                    &std::fs::read(path).map_err(|e| e.to_string())?,
                    &preferences,
                )
                .map(Some)
            })();
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        #[cfg(all(not(test), target_arch = "wasm32"))]
        wasm_bindgen_futures::spawn_local(async move {
            let result = if let Some(file) = rfd::AsyncFileDialog::new()
                .set_title(tr!("image_tools.choose_reference_image"))
                .add_filter(tr!("image_tools.images_png_jpeg_webp"), &["png", "jpg", "jpeg", "webp"])
                .pick_file()
                .await
            {
                decode_image(file.file_name(), &file.read().await, &preferences).map(Some)
            } else {
                Ok(None)
            };
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        self.images.pending = Some(Pending {
            result,
            points,
            replace,
            context: self.doc.model.edit_context,
            project: self.doc.name.clone(),
        });
        true
    }
    pub(super) fn poll_reference_image(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.images.pending else {
            return;
        };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(_) => Err(tr!("image_tools.the_image_picker_could_not_finish").into()),
        };
        let pending = self.images.pending.take().unwrap();
        match result {
            Ok(Some(ImportedImage { mut image, asset })) => {
                if self.doc.name != pending.project
                    || self.doc.model.edit_context != pending.context
                {
                    self.say(
                        ctx,
                        tr!("image_tools.image_import_cancelled_because_the_project_or_editing_context_changed"),
                    );
                    return;
                }
                let before = self.doc.model.clone();
                if let Some(face) = pending.replace {
                    if self.doc.model.is_locked(face) {
                        self.say(ctx, tr!("image_tools.unlock_the_image_before_replacing_it"));
                        return;
                    }
                    if let Some(old) = self.doc.model.reference_images.get(&face) {
                        image.vertices = old.vertices;
                        image.settings = old.settings.clone();
                        image.placement_offset = old.placement_offset;
                        self.doc.model.image_assets.insert(image.asset, asset);
                        self.doc.model.reference_images.insert(face, image);
                        self.finish_context_state_edit(tr!("image_tools.replace_reference_image"), before, ctx);
                    }
                } else {
                    let points = pending.points.map(|p| p + image.placement_offset);
                    match self.doc.model.add_reference_image(points, image, asset) {
                        Ok(id) => {
                            self.selected = BTreeSet::from([id]);
                            self.finish_edit(tr!("image_tools.add_reference_image"), before, ctx);
                        }
                        Err(message) => self.say(ctx, message),
                    }
                }
            }
            Err(message) => self.say(ctx, message),
            Ok(None) => {}
        }
    }

    pub(super) fn selected_reference_image(&self) -> Option<u64> {
        if self.selected.len() != 1 {
            return None;
        }
        let selected = *self.selected.first()?;
        let model = &self.doc.model;
        // Inspect only image ancestry. Expanding a selected assembly here
        // rebuilt a set of its entire mesh on every inspector redraw, even
        // for projects without any images.
        let mut images = model.reference_images.keys().copied().filter(|&id| {
            let mut ancestor = id;
            loop {
                if ancestor == selected {
                    return true;
                }
                let Some(parent) = model.parent(ancestor) else {
                    return false;
                };
                ancestor = parent;
            }
        });
        let image = images.next()?;
        if images.next().is_some() {
            return None;
        }
        (image == selected || model.parent(image) == Some(selected)).then_some(image)
    }

    pub(super) fn image_properties(&mut self, ui: &mut egui::Ui) -> bool {
        if let Some(id) = self.selected_reference_image() {
            let source = self.doc.model.reference_images[&id].clone();
            let mut edited = source.clone();
            if let Some(edit) = &self.images.adjustment
                && edit.id == id
                && edit.source == source
                && edit.context == self.doc.model.edit_context
            {
                edited.settings = edit.settings.clone();
            }
            let previous = edited.settings.clone();
            let mut active = None;
            let locked = self.doc.model.is_locked(id);
            ui.heading(tr!("image_tools.reference_image"));
            Self::property_row(ui, tr!("app.file"), &source.name);
            let pixels = self.doc.model.image_assets[&source.asset].pixels;
            Self::property_row(ui, tr!("screenshot_dialog.resolution"), &format!("{} × {}", pixels[0], pixels[1]));
            ui.add_enabled_ui(!locked, |ui| {
                sidebar::inspector_section(ui, tr!("image_tools.image"), "reference_image_appearance", true, |ui| {
                    active = appearance_fields(ui, &mut edited.settings);
                });
            });
            if locked {
                ui.label(tr!("image_tools.unlock_the_image_to_edit_its_properties"));
            }
            if edited.settings != previous {
                self.adjust_image_appearance(id, edited.settings, active, ui.ctx());
            }
            true
        } else if self.tool == Tool::Image {
            ui.heading(tr!("image_tools.image_tool"));
            ui.label(tr!("image_tools.draw_two_opposite_corners_then_choose_a_png_jpeg_or_webp_image_images_are_embedded_in_the_project"));
            sidebar::inspector_section(
                ui,
                tr!("image_tools.image_defaults"),
                "reference_image_defaults",
                true,
                |ui| {
                    appearance_fields(ui, &mut self.images.preferences.appearance);
                },
            );
            true
        } else {
            false
        }
    }
}
fn field(
    ui: &mut egui::Ui,
    key: &str,
    label: &str,
    content: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> Option<(egui::Id, egui::Rect, bool)> {
    let mut active = None;
    crate::inspector_table::inspector_field(ui, ("reference_image", key), label, |ui| {
        let response = content(ui);
        if response.changed() && (response.dragged() || response.has_focus()) {
            active = Some((response.id, response.rect, response.dragged()));
        }
        response
    });
    active
}
fn appearance_fields(
    ui: &mut egui::Ui,
    settings: &mut ImageSettings,
) -> Option<(egui::Id, egui::Rect, bool)> {
    let mut active = None;
    for response in [
        field(ui, "Zoom", tr!("image_tools.zoom"), |ui| {
            ui.add(
                egui::DragValue::new(&mut settings.zoom)
                    .speed(0.01)
                    .range(0.01..=100.)
                    .suffix("×"),
            )
        }),
        field(ui, "Rotation", tr!("image_tools.rotation"), |ui| {
            ui.add(
                egui::DragValue::new(&mut settings.rotation)
                    .speed(1.)
                    .range(-360. ..=360.)
                    .suffix("°"),
            )
        }),
        field(ui, "Opacity", tr!("image_tools.opacity"), |ui| {
            ui.add(egui::Slider::new(&mut settings.opacity, 0. ..=1.))
        }),
        field(ui, "Brightness", tr!("image_tools.brightness"), |ui| {
            ui.add(egui::Slider::new(&mut settings.brightness, 0. ..=2.))
        }),
        field(ui, "Contrast", tr!("image_tools.contrast"), |ui| {
            ui.add(egui::Slider::new(&mut settings.contrast, 0. ..=2.))
        }),
    ] {
        active = active.or(response);
    }
    for (key, label, value) in [
        ("Flip horizontal", tr!("image_tools.flip_horizontal"), &mut settings.flip_horizontal),
        ("Flip vertical", tr!("image_tools.flip_vertical"), &mut settings.flip_vertical),
        ("Grayscale", tr!("image_tools.grayscale"), &mut settings.grayscale),
    ] {
        field(ui, key, label, |ui| ui.checkbox(value, ""));
    }
    active
}

#[cfg(test)]
mod tests {
    use super::*;
    use benchup_core::ImageFit;
    fn png() -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            12,
            6,
            image::Rgba([20, 40, 60, 80]),
        ))
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
        out.into_inner()
    }
    #[test]
    fn import_keeps_small_images_and_transparency_and_reports_bad_files() {
        let imported = decode_image("Photo.png".into(), &png(), &Preferences::default()).unwrap();
        assert_eq!(imported.asset.pixels, [12, 6]);
        assert!(imported.asset.valid());
        let decoded = image::load_from_memory(&imported.asset.png)
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [20, 40, 60, 80]);
        assert!(decode_image("broken.png".into(), b"broken", &Preferences::default()).is_err());
    }
    fn queue(app: &mut BenchUpApp, result: Result<Option<ImportedImage>, String>) {
        let (sender, receiver) = mpsc::channel();
        sender.send(result).unwrap();
        app.images.pending = Some(Pending {
            result: receiver,
            points: [DVec3::ZERO, DVec3::X * 2., DVec3::new(2., 1., 0.), DVec3::Y],
            replace: None,
            context: app.doc.model.edit_context,
            project: app.doc.name.clone(),
        });
    }
    fn image_app() -> (BenchUpApp, u64) {
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        let imported = decode_image("Plan.png".into(), &png(), &Preferences::default()).unwrap();
        app.doc
            .model
            .add_reference_image(
                [DVec3::ZERO, DVec3::X * 2., DVec3::new(2., 1., 0.), DVec3::Y],
                imported.image,
                imported.asset,
            )
            .unwrap();
        let id = *app.doc.model.reference_images.keys().next().unwrap();
        (app, id)
    }

    #[test]
    fn image_adjustments_preview_without_scene_changes_and_commit_one_undo_step() {
        let ctx = egui::Context::default();
        let (mut app, id) = image_app();
        // Unrelated geometry makes accidental per-update scene cloning and
        // invalidation visible through the shared snapshot and revision.
        for n in 0..2048 {
            app.doc
                .model
                .add_edge(DVec3::new(n as f64, 10., 0.), DVec3::new(n as f64, 11., 0.))
                .unwrap();
        }
        let before = app.doc.model.clone();
        let revision = app.refresh_document_snapshot(None);
        let snapshot = app.document_model.shared();
        let control = Some((egui::Id::unique("image-test"), egui::Rect::EVERYTHING, true));
        for n in 1..=120 {
            let mut settings = before.reference_images[&id].settings.clone();
            settings.brightness = 1. + n as f64 / 120.;
            app.adjust_image_appearance(id, settings.clone(), control, &ctx);
            assert_eq!(app.image_appearance_preview(), Some((id, &settings)));
            assert!(!app.document_model_dirty);
            assert_eq!(app.document_model.revision(), revision);
            assert!(Arc::ptr_eq(&snapshot, &app.document_model.shared()));
            assert!(app.doc.history.is_empty());
            assert!(!app.project.modified);
            assert!(app.dirty_since.is_none());
        }
        assert_eq!(app.doc.model, before);
        // No pointer is down: releasing the slider commits once, even if the
        // slider itself kept keyboard focus.
        ctx.memory_mut(|m| m.request_focus(control.unwrap().0));
        app.finish_image_adjustment(&ctx);
        assert!(app.image_appearance_preview().is_none());
        assert_eq!(app.doc.history.position(), 1);
        assert_eq!(app.doc.model.reference_images[&id].settings.brightness, 2.);
        assert!(app.project.modified);
        assert!(!app.document_model_dirty);
        assert_eq!(app.refresh_document_snapshot(None), revision);
        assert!(Arc::ptr_eq(&snapshot, &app.document_model.shared()));
        assert!(app.doc.history.undo(&mut app.doc.model));
        assert_eq!(app.doc.model, before);
        assert!(app.doc.history.redo(&mut app.doc.model));
        assert_eq!(app.doc.model.reference_images[&id].settings.brightness, 2.);
    }

    #[test]
    fn obsolete_image_previews_cannot_overwrite_model_changes() {
        let ctx = egui::Context::default();
        for change in 0..4 {
            let (mut app, id) = image_app();
            let mut settings = app.doc.model.reference_images[&id].settings.clone();
            settings.opacity = 0.25;
            app.adjust_image_appearance(
                id,
                settings,
                Some((egui::Id::NULL, egui::Rect::EVERYTHING, true)),
                &ctx,
            );
            match change {
                0 => {
                    app.doc.model.reference_images.remove(&id);
                }
                1 => {
                    app.doc
                        .model
                        .reference_images
                        .get_mut(&id)
                        .unwrap()
                        .settings
                        .opacity = 0.75;
                }
                2 => {
                    app.doc.model.locked.insert(id);
                }
                _ => {
                    app.doc.model.edit_context = app.doc.model.parent(id);
                }
            }
            let changed = app.doc.model.clone();
            app.finish_image_adjustment(&ctx);
            assert!(app.image_appearance_preview().is_none());
            assert_eq!(app.doc.model, changed);
            assert!(app.doc.history.is_empty());
        }
    }

    #[test]
    fn image_toggles_commit_immediately_without_invalidating_scene_geometry() {
        let ctx = egui::Context::default();
        let (mut app, id) = image_app();
        let initial = app.doc.model.clone();
        let revision = app.refresh_document_snapshot(None);
        let snapshot = app.document_model.shared();
        for action in 0..3 {
            let mut settings = app.doc.model.reference_images[&id].settings.clone();
            match action {
                0 => settings.flip_horizontal = true,
                1 => settings.flip_vertical = true,
                _ => settings.grayscale = true,
            }
            app.adjust_image_appearance(id, settings.clone(), None, &ctx);
            assert_eq!(app.doc.model.reference_images[&id].settings, settings);
            assert!(app.images.adjustment.is_none());
            assert_eq!(app.doc.history.position(), action + 1);
            assert!(app.project.modified);
            assert!(
                app.dirty_since.is_none(),
                "Each toggle must save immediately"
            );
            assert!(!app.document_model_dirty);
            assert_eq!(app.refresh_document_snapshot(None), revision);
            assert!(Arc::ptr_eq(&snapshot, &app.document_model.shared()));
        }
        assert!(!initial.same_scene(&app.doc.model));
        assert!(initial.same_scene_geometry(&app.doc.model));
        for _ in 0..3 {
            assert!(app.doc.history.undo(&mut app.doc.model));
        }
        assert_eq!(app.doc.model, initial);
        // Frame edits must still invalidate the geometry snapshot.
        let vertex = app.doc.model.reference_images[&id].vertices[0];
        app.doc.model.vertices.get_mut(&vertex).unwrap().position.x += 0.1;
        assert_ne!(app.refresh_document_snapshot(None), revision);
    }

    #[test]
    fn image_toggles_inside_linked_components_still_synchronize_their_peers() {
        let ctx = egui::Context::default();
        let (mut app, id) = image_app();
        let group = app.doc.model.parent(id).unwrap();
        let component = app
            .doc
            .model
            .make_object(&BTreeSet::from([group]), true, "Plan component")
            .unwrap();
        let id = *app.doc.model.reference_images.keys().next().unwrap();
        let copy = app.doc.model.copy_entities(&BTreeSet::from([component]));
        app.doc.model.paste_entities(&copy, DVec3::X * 3.);
        assert_eq!(app.doc.model.reference_images.len(), 2);
        app.doc.model.edit_context = app.doc.model.parent(id);
        let before = app.doc.model.clone();
        let mut settings = app.doc.model.reference_images[&id].settings.clone();
        settings.grayscale = true;
        app.adjust_image_appearance(id, settings, None, &ctx);
        assert!(
            app.doc
                .model
                .reference_images
                .values()
                .all(|image| image.settings.grayscale)
        );
        assert_eq!(app.doc.history.position(), 1);
        app.doc.model.validate().unwrap();
        assert!(app.doc.history.undo(&mut app.doc.model));
        assert_eq!(app.doc.model, before);
    }

    #[test]
    fn slider_release_includes_the_final_pointer_position_in_one_commit() {
        use crate::test_ui::ContextExt as _;
        let ctx = egui::Context::default();
        let (mut app, id) = image_app();
        let mut frame = |events| {
            let mut rect = egui::Rect::NOTHING;
            ctx.run_test_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600., 200.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    app.prepare_image_adjustment(&ctx);
                    let mut settings = app.image_appearance_preview().map_or_else(
                        || app.doc.model.reference_images[&id].settings.clone(),
                        |(_, settings)| settings.clone(),
                    );
                    let previous = settings.clone();
                    let active = field(ui, "Brightness", "Brightness", |ui| {
                        let response =
                            ui.add(egui::Slider::new(&mut settings.brightness, 0. ..=2.));
                        rect = response.rect;
                        response
                    });
                    if settings != previous {
                        app.adjust_image_appearance(id, settings, active, &ctx);
                    }
                    app.finish_image_adjustment(&ctx);
                },
            );
            (
                rect,
                app.doc.history.position(),
                app.doc.model.reference_images[&id].settings.brightness,
            )
        };
        let (rect, _, original) = frame(vec![]);
        let start = egui::pos2(rect.left() + 20., rect.center().y);
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(start), pointer(start, true)]);
        let (_, history, committed) =
            frame(vec![egui::Event::PointerMoved(start + egui::vec2(30., 0.))]);
        assert_eq!(history, 0);
        assert_eq!(committed, original);
        // The release itself supplies a different final position, without a
        // preceding PointerMoved event (as on some browser backends).
        let (_, history, committed) = frame(vec![pointer(start + egui::vec2(60., 0.), false)]);
        assert_eq!(history, 1);
        assert_ne!(committed, original);
        assert!(app.images.adjustment.is_none());
    }

    #[test]
    fn text_adjustments_finish_on_blur_and_project_reset_discards_previews() {
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.focused = true);
        let (mut app, id) = image_app();
        let control = egui::Id::unique("image-text");
        ctx.memory_mut(|m| m.request_focus(control));
        let mut settings = app.doc.model.reference_images[&id].settings.clone();
        settings.rotation = 45.;
        app.adjust_image_appearance(
            id,
            settings,
            Some((control, egui::Rect::EVERYTHING, false)),
            &ctx,
        );
        app.finish_image_adjustment(&ctx);
        assert!(app.image_appearance_preview().is_some());
        assert!(app.doc.history.is_empty());
        ctx.memory_mut(|m| m.surrender_focus(control));
        app.finish_image_adjustment(&ctx);
        assert_eq!(app.doc.history.position(), 1);
        assert_eq!(app.doc.model.reference_images[&id].settings.rotation, 45.);
        let mut settings = app.doc.model.reference_images[&id].settings.clone();
        settings.rotation = 90.;
        app.adjust_image_appearance(
            id,
            settings,
            Some((control, egui::Rect::EVERYTHING, true)),
            &ctx,
        );
        app.reset_image_project();
        assert!(app.image_appearance_preview().is_none());
        assert_eq!(app.doc.model.reference_images[&id].settings.rotation, 45.);
    }
    #[test]
    fn cancel_and_failure_leave_document_and_history_untouched() {
        let ctx = egui::Context::default();
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        let before = app.doc.clone();
        for result in [Ok(None), Err("Broken image".into())] {
            queue(&mut app, result);
            app.poll_reference_image(&ctx);
            assert_eq!(app.doc, before);
            assert!(!app.project.modified);
            assert!(app.images.pending.is_none());
        }
    }
    #[test]
    fn imported_rectangle_applies_offset_once_and_undoes_atomically() {
        let ctx = egui::Context::default();
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        app.images.preferences.offset = DVec3::new(1., 2., 3.);
        let imported = decode_image("Plan.png".into(), &png(), &app.images.preferences).unwrap();
        queue(&mut app, Ok(Some(imported)));
        app.poll_reference_image(&ctx);
        assert_eq!(app.doc.model.reference_images.len(), 1);
        let image = app.doc.model.reference_images.values().next().unwrap();
        assert_eq!(
            image.points(&app.doc.model).unwrap()[0],
            DVec3::new(1., 2., 3.)
        );
        assert_eq!(image.placement_offset, DVec3::new(1., 2., 3.));
        assert_eq!(app.selected.len(), 1);
        app.doc.model.validate().unwrap();
        assert!(app.doc.history.undo(&mut app.doc.model));
        assert!(app.doc.model.reference_images.is_empty());
        assert!(app.doc.model.edges.is_empty());
        assert!(app.doc.model.faces.is_empty());
    }
    #[test]
    fn project_switch_discards_pending_imports_and_keeps_only_reusable_defaults() {
        let ctx = egui::Context::default();
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        app.images.preferences.appearance.fit = ImageFit::Cover;
        let imported = decode_image("Plan.png".into(), &png(), &app.images.preferences).unwrap();
        queue(&mut app, Ok(Some(imported)));
        app.replace_project(Document::default(), None, &ctx);
        app.poll_reference_image(&ctx);
        assert!(app.doc.model.reference_images.is_empty());
        assert!(app.doc.model.image_assets.is_empty());
        assert_eq!(app.images.preferences.appearance.fit, ImageFit::Cover);
    }
}
