//! Image placement uses the same rectangle inference and locks as drawing.
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
#[derive(Default)]
pub(super) struct State {
    pub preferences: Preferences,
    pending: Option<Pending>,
    pub gpu: crate::reference_image_gpu::Cache,
}

/// Bound both decoding and GPU memory. Large photos are resampled once at import.
fn decode_image(
    name: String,
    bytes: &[u8],
    preferences: &Preferences,
) -> Result<ImportedImage, String> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("Choose an image smaller than 32 MB.".into());
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
        .map_err(|e| format!("Could not open image: {e}"))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut source = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| format!("Could not open image: {e}"))?;
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
        return Err("The decoded image is too large. Choose a smaller image.".into());
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
        self.images.gpu = Default::default();
    }
    pub(super) fn choose_reference_image(
        &mut self,
        ctx: &egui::Context,
        points: [DVec3; 4],
        replace: Option<u64>,
    ) -> bool {
        if self.images.pending.is_some() {
            self.say(ctx, "Finish or cancel the current image picker first.");
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
        std::thread::spawn(move || {
            let result = (|| {
                let Some(path) = rfd::FileDialog::new()
                    .set_title("Choose reference image")
                    .add_filter("Images (PNG, JPEG, WebP)", &["png", "jpg", "jpeg", "webp"])
                    .pick_file()
                else {
                    return Ok(None);
                };
                if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
                    return Err("Choose an image smaller than 32 MB.".into());
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
                .set_title("Choose reference image")
                .add_filter("Images (PNG, JPEG, WebP)", &["png", "jpg", "jpeg", "webp"])
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
            Err(_) => Err("The image picker could not finish.".into()),
        };
        let pending = self.images.pending.take().unwrap();
        match result {
            Ok(Some(ImportedImage { mut image, asset })) => {
                if self.doc.name != pending.project
                    || self.doc.model.edit_context != pending.context
                {
                    self.say(
                        ctx,
                        "Image import cancelled because the project or editing context changed.",
                    );
                    return;
                }
                let before = self.doc.model.clone();
                if let Some(face) = pending.replace {
                    if self.doc.model.is_locked(face) {
                        self.say(ctx, "Unlock the image before replacing it.");
                        return;
                    }
                    if let Some(old) = self.doc.model.reference_images.get(&face) {
                        image.vertices = old.vertices;
                        image.settings = old.settings.clone();
                        image.placement_offset = old.placement_offset;
                        self.doc.model.image_assets.insert(image.asset, asset);
                        self.doc.model.reference_images.insert(face, image);
                        self.finish_context_state_edit("Replace reference image", before, ctx);
                    }
                } else {
                    let points = pending.points.map(|p| p + image.placement_offset);
                    match self.doc.model.add_reference_image(points, image, asset) {
                        Ok(id) => {
                            self.selected = BTreeSet::from([id]);
                            self.finish_edit("Add reference image", before, ctx);
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
            let locked = self.doc.model.is_locked(id);
            ui.heading("Reference image");
            Self::property_row(ui, "File", &source.name);
            let pixels = self.doc.model.image_assets[&source.asset].pixels;
            Self::property_row(ui, "Resolution", &format!("{} × {}", pixels[0], pixels[1]));
            ui.add_enabled_ui(!locked, |ui| {
                sidebar::inspector_section(ui, "Image", "reference_image_appearance", true, |ui| {
                    appearance_fields(ui, &mut edited.settings);
                });
            });
            if locked {
                ui.label("Unlock the image to edit its properties.");
            }
            if edited != source {
                let before = self.doc.model.clone();
                self.doc.model.reference_images.insert(id, edited);
                self.finish_context_state_edit("Edit reference image", before, ui.ctx());
            }
            true
        } else if self.tool == Tool::Image {
            ui.heading("Image tool");
            ui.label("Draw two opposite corners, then choose a PNG, JPEG or WebP image. Images are embedded in the project.");
            sidebar::inspector_section(
                ui,
                "Image defaults",
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
fn field(ui: &mut egui::Ui, label: &str, content: impl FnOnce(&mut egui::Ui) -> egui::Response) {
    crate::inspector_table::inspector_field(ui, ("reference_image", label), label, content);
}
fn appearance_fields(ui: &mut egui::Ui, settings: &mut ImageSettings) {
    field(ui, "Zoom", |ui| {
        ui.add(
            egui::DragValue::new(&mut settings.zoom)
                .speed(0.01)
                .range(0.01..=100.)
                .suffix("×"),
        )
    });
    field(ui, "Rotation", |ui| {
        ui.add(
            egui::DragValue::new(&mut settings.rotation)
                .speed(1.)
                .range(-360. ..=360.)
                .suffix("°"),
        )
    });
    field(ui, "Opacity", |ui| {
        ui.add(egui::Slider::new(&mut settings.opacity, 0. ..=1.))
    });
    field(ui, "Brightness", |ui| {
        ui.add(egui::Slider::new(&mut settings.brightness, 0. ..=2.))
    });
    field(ui, "Contrast", |ui| {
        ui.add(egui::Slider::new(&mut settings.contrast, 0. ..=2.))
    });
    for (label, value) in [
        ("Flip horizontal", &mut settings.flip_horizontal),
        ("Flip vertical", &mut settings.flip_vertical),
        ("Grayscale", &mut settings.grayscale),
    ] {
        field(ui, label, |ui| ui.checkbox(value, ""));
    }
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
