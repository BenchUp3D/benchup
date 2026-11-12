//! Embedded, portable reference pictures attached to independent rectangular faces.
use crate::{DVec3, Edge, Face, Model, Object, ObjectKind, Vertex};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    #[default]
    Contain,
    Cover,
    Stretch,
}
impl ImageFit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Contain => tr!("core.reference_images.contain"),
            Self::Cover => tr!("core.reference_images.cover"),
            Self::Stretch => tr!("core.reference_images.stretch"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageSettings {
    pub fit: ImageFit,
    /// Position of the picture in its frame: top left (0,0), center (.5,.5).
    pub anchor: [f64; 2],
    /// Additional displacement measured in frame widths/heights.
    pub pan: [f64; 2],
    pub zoom: f64,
    pub rotation: f64,
    pub opacity: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub grayscale: bool,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub smoothing: bool,
    pub double_sided: bool,
}
impl Default for ImageSettings {
    fn default() -> Self {
        Self {
            fit: ImageFit::Contain,
            anchor: [0.5; 2],
            pan: [0.; 2],
            zoom: 1.,
            rotation: 0.,
            opacity: 1.,
            brightness: 1.,
            contrast: 1.,
            grayscale: false,
            flip_horizontal: false,
            flip_vertical: false,
            smoothing: true,
            double_sided: true,
        }
    }
}
impl ImageSettings {
    pub fn valid(&self) -> bool {
        self.anchor
            .iter()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            && self
                .pan
                .iter()
                .all(|v| v.is_finite() && (-10. ..=10.).contains(v))
            && self.zoom.is_finite()
            && (0.01..=100.).contains(&self.zoom)
            && self.rotation.is_finite()
            && (-360. ..=360.).contains(&self.rotation)
            && self.opacity.is_finite()
            && (0. ..=1.).contains(&self.opacity)
            && self.brightness.is_finite()
            && (0. ..=2.).contains(&self.brightness)
            && self.contrast.is_finite()
            && (0. ..=2.).contains(&self.contrast)
    }
    /// Image size as a fraction of the frame. Rotation happens in physical
    /// frame coordinates in the shader, avoiding distortion in non-square frames.
    pub fn fitted_size(&self, frame: [f64; 2], pixels: [u32; 2]) -> [f64; 2] {
        let aspect = pixels[0] as f64 / pixels[1].max(1) as f64;
        let ratio = aspect / (frame[0] / frame[1].max(1e-10));
        let size = match self.fit {
            ImageFit::Stretch => [1., 1.],
            ImageFit::Contain if ratio > 1. => [1., 1. / ratio],
            ImageFit::Contain => [ratio, 1.],
            ImageFit::Cover if ratio > 1. => [ratio, 1.],
            ImageFit::Cover => [1., 1. / ratio],
        };
        size.map(|v| v * self.zoom)
    }
}

pub type ImageAssetId = uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageAsset {
    /// Canonical PNG bytes, stored once per project, shared by snapshots/copies.
    pub png: Arc<[u8]>,
    pub pixels: [u32; 2],
}
impl ImageAsset {
    pub fn valid(&self) -> bool {
        self.png.len() >= 24
            && self.png.len() <= 32 * 1024 * 1024
            && self.png.starts_with(b"\x89PNG\r\n\x1a\n")
            && &self.png[12..16] == b"IHDR"
            && u32::from_be_bytes(self.png[16..20].try_into().unwrap()) == self.pixels[0]
            && u32::from_be_bytes(self.png[20..24].try_into().unwrap()) == self.pixels[1]
            && self.pixels.iter().all(|v| *v > 0 && *v <= 4096)
            && self.pixels[0] as u64 * self.pixels[1] as u64 <= 16_777_216
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceImage {
    pub name: String,
    pub asset: ImageAssetId,
    /// Preserve image orientation when a face winding changes after a reflection.
    pub vertices: [u64; 4],
    #[serde(default)]
    pub settings: ImageSettings,
    #[serde(default)]
    pub placement_offset: DVec3,
}
impl ReferenceImage {
    pub fn points(&self, model: &Model) -> Option<[DVec3; 4]> {
        let mut points = [DVec3::ZERO; 4];
        for (i, id) in self.vertices.iter().enumerate() {
            points[i] = model.vertices.get(id)?.position;
        }
        Some(points)
    }
}
impl Model {
    pub fn add_reference_image(
        &mut self,
        points: [DVec3; 4],
        mut image: ReferenceImage,
        asset: ImageAsset,
    ) -> Result<u64, &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.add_reference_image(points, image, asset));
        }
        if points.iter().any(|p| !crate::measurements::valid_point(*p))
            || points[0].distance(points[1]) < 1e-7
            || points[0].distance(points[3]) < 1e-7
            || (points[1] - points[0])
                .cross(points[3] - points[0])
                .length()
                < 1e-14
            || (points[0] + points[2] - points[1] - points[3]).length() > 1e-6
            || !valid_reference(&image)
            || !asset.valid()
        {
            return Err(tr!("core.reference_images.invalid_rectangle"));
        }
        // Never split or paint the model surface under the reference image.
        let vertices = points.map(|position| {
            let id = self.id();
            self.vertices.insert(id, Vertex { id, position });
            id
        });
        image.vertices = vertices;
        let mut entities = BTreeSet::new();
        for i in 0..4 {
            let id = self.id();
            self.edges.insert(
                id,
                Edge {
                    id,
                    a: vertices[i],
                    b: vertices[(i + 1) % 4],
                },
            );
            entities.insert(id);
        }
        let face = self.id();
        self.faces.insert(
            face,
            Face {
                id: face,
                vertices: vertices.to_vec(),
                holes: vec![],
            },
        );
        entities.insert(face);
        let id = self.id();
        self.objects.insert(
            id,
            Object {
                id,
                name: image.name.clone(),
                kind: ObjectKind::Group,
                entities,
                surface_treatment: Default::default(),
                surface_treatment_customized: false,
                stock_material: None,
                stock_material_id: None,
                outliner_order: None,
                transform: Some(Default::default()),
                grain_axis: None,
                definition_entities: Default::default(),
            },
        );
        self.image_assets.insert(image.asset, asset);
        self.reference_images.insert(face, image);
        Ok(id)
    }
    pub(crate) fn validate_reference_images(&self) -> Result<(), &'static str> {
        for (id, image) in &self.reference_images {
            let Some(face) = self.faces.get(id) else {
                return Err(tr!("core.reference_images.missing_frame"));
            };
            if !valid_reference(image)
                || !self
                    .image_assets
                    .get(&image.asset)
                    .is_some_and(ImageAsset::valid)
                || image.vertices.iter().any(|id| !face.vertices.contains(id))
                || image.vertices.iter().collect::<BTreeSet<_>>().len() != 4
            {
                return Err(tr!("core.reference_images.invalid_image"));
            }
        }
        Ok(())
    }
}
fn valid_reference(image: &ReferenceImage) -> bool {
    !image.name.is_empty()
        && image.name.len() <= 1024
        && crate::measurements::valid_point(image.placement_offset)
        && image.settings.valid()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    fn fixture() -> (ReferenceImage, ImageAsset) {
        // One transparent PNG pixel, with a real IHDR and zlib payload.
        let png: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 96, 0, 2,
            0, 0, 5, 0, 1, 122, 94, 171, 63, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        (
            ReferenceImage {
                name: "Plan.png".into(),
                asset: ImageAssetId::new_v4(),
                vertices: [0; 4],
                settings: Default::default(),
                placement_offset: DVec3::ZERO,
            },
            ImageAsset {
                png: Arc::from(png),
                pixels: [1, 1],
            },
        )
    }

    #[test]
    fn image_settings_history_is_a_small_attributed_delta_with_undo_redo_and_branches() {
        let (image, asset) = fixture();
        let mut doc = crate::Document::default();
        doc.model
            .add_reference_image(rectangle(), image, asset)
            .unwrap();
        let id = *doc.model.reference_images.keys().next().unwrap();
        let initial = doc.model.clone();
        let author = crate::HistoryAuthor {
            id: "image-editor".into(),
            name: "Image editor".into(),
            kind: "local".into(),
            verified: false,
        };
        for action in 0..3 {
            let before = doc.model.reference_images[&id].clone();
            let mut settings = before.settings.clone();
            match action {
                0 => settings.flip_horizontal = true,
                1 => settings.flip_vertical = true,
                _ => settings.grayscale = true,
            }
            doc.model.reference_images.get_mut(&id).unwrap().settings = settings.clone();
            doc.history.record_image_settings_applied(
                id,
                before,
                settings,
                Some(author.clone()),
                Some(123),
            );
        }
        let edited = doc.model.clone();
        let json = serde_json::to_value(&doc.history).unwrap();
        for entry in json["undo"].as_array().unwrap() {
            assert_eq!(entry["author"]["id"], "image-editor");
            assert_eq!(entry["timestamp"], 123);
            let change = entry["change"].as_object().unwrap();
            assert_eq!(change.len(), 1);
            assert!(change.contains_key("reference_images"));
        }
        let base = crate::Document {
            model: initial.clone(),
            ..Default::default()
        };
        let restored = doc
            .checkpoint(&base.model, &base.history)
            .restore(&base.model, &base.history)
            .unwrap();
        assert_eq!(restored, doc);
        for _ in 0..3 {
            assert!(doc.history.undo(&mut doc.model));
        }
        assert_eq!(doc.model, initial);
        for _ in 0..3 {
            assert!(doc.history.redo(&mut doc.model));
        }
        assert_eq!(doc.model, edited);
        assert!(doc.history.undo(&mut doc.model));
        let before = doc.model.reference_images[&id].clone();
        let mut settings = before.settings.clone();
        settings.opacity = 0.5;
        doc.model.reference_images.get_mut(&id).unwrap().settings = settings.clone();
        doc.history
            .record_image_settings_applied(id, before, settings, None, None);
        assert!(!doc.history.redo(&mut doc.model));
    }
    fn rectangle() -> [DVec3; 4] {
        [
            DVec3::ZERO,
            DVec3::X * 4.,
            DVec3::new(4., 2., 0.),
            DVec3::Y * 2.,
        ]
    }
    #[test]
    fn fit_modes_preserve_aspect_and_zoom_for_both_orientations() {
        let mut settings = ImageSettings::default();
        assert_eq!(settings.fitted_size([2., 2.], [400, 200]), [1., 0.5]);
        assert_eq!(settings.fitted_size([2., 2.], [200, 400]), [0.5, 1.]);
        settings.fit = ImageFit::Cover;
        assert_eq!(settings.fitted_size([2., 2.], [400, 200]), [2., 1.]);
        assert_eq!(settings.fitted_size([2., 2.], [200, 400]), [1., 2.]);
        settings.fit = ImageFit::Stretch;
        settings.zoom = 2.;
        assert_eq!(settings.fitted_size([4., 2.], [200, 400]), [2., 2.]);
    }
    #[test]
    fn image_save_reload_history_and_legacy_defaults() {
        let mut doc = Document::default();
        let (image, asset) = fixture();
        let before = doc.model.clone();
        doc.model
            .add_reference_image(rectangle(), image, asset)
            .unwrap();
        doc.history.record("Image", before, &doc.model);
        let id = *doc.model.reference_images.keys().next().unwrap();
        let before = doc.model.clone();
        doc.model
            .reference_images
            .get_mut(&id)
            .unwrap()
            .settings
            .opacity = 0.4;
        doc.history.record("Opacity", before, &doc.model);
        let json = serde_json::to_value(&doc).unwrap();
        let latest = &json["history"];
        // Appearance-only history entries never duplicate an embedded asset.
        let latest_text = serde_json::to_string(latest).unwrap();
        assert_eq!(latest_text.matches("\"png\"").count(), 1);
        let mut restored = Document::from_json(&json.to_string()).unwrap();
        assert_eq!(restored.model.reference_images[&id].settings.opacity, 0.4);
        assert!(restored.history.undo(&mut restored.model));
        assert_eq!(restored.model.reference_images[&id].settings.opacity, 1.);
        assert!(restored.history.undo(&mut restored.model));
        assert!(restored.model.reference_images.is_empty());
        assert!(restored.history.redo(&mut restored.model));
        restored.model.validate().unwrap();
        let mut legacy = serde_json::to_value(Document::default()).unwrap();
        legacy["model"]
            .as_object_mut()
            .unwrap()
            .remove("reference_images");
        legacy["model"]
            .as_object_mut()
            .unwrap()
            .remove("image_assets");
        assert!(
            Document::from_json(&legacy.to_string())
                .unwrap()
                .model
                .reference_images
                .is_empty()
        );
        assert_eq!(
            serde_json::from_str::<ImageSettings>("{}").unwrap(),
            ImageSettings::default()
        );
    }
    #[test]
    fn copying_transforming_and_erasing_keep_images_attached_and_portable() {
        let mut model = Model::default();
        let (image, asset) = fixture();
        let object = model
            .add_reference_image(rectangle(), image, asset)
            .unwrap();
        let roots = BTreeSet::from([object]);
        let copy = model.copy_entities(&roots);
        let pasted = model.paste_entities(&copy, DVec3::X * 10.);
        assert_eq!(model.reference_images.len(), 2);
        assert_eq!(model.image_assets.len(), 1);
        model.move_selection(&pasted, DVec3::Z).unwrap();
        model.scale_selection(&pasted, DVec3::ZERO, 2.).unwrap();
        model.validate().unwrap();
        let face = model
            .reference_images
            .iter()
            .find(|(id, _)| model.parent(**id) == pasted.first().copied())
            .unwrap()
            .0;
        assert_eq!(
            model.reference_images[face].points(&model).unwrap()[0],
            DVec3::new(20., 0., 2.)
        );
        let a = &model.image_assets.values().next().unwrap().png;
        let b = &copy.image_assets.values().next().unwrap().png;
        assert!(Arc::ptr_eq(a, b));
        model.erase(&pasted);
        assert_eq!(model.reference_images.len(), 1);
        model.erase(&roots);
        assert!(model.reference_images.is_empty());
        model.validate().unwrap();
    }
    #[test]
    fn scoped_images_survive_edits_and_reject_locked_contexts() {
        let mut model = Model::default();
        let (image, asset) = fixture();
        let group = model
            .add_reference_image(rectangle(), image, asset)
            .unwrap();
        assert!(model.enter_object(group));
        let (image, asset) = fixture();
        let nested = model
            .add_reference_image(rectangle().map(|p| p + DVec3::Z), image, asset)
            .unwrap();
        assert_eq!(model.parent(nested), Some(group));
        model.validate().unwrap();
        model.locked.insert(group);
        let before = model.clone();
        let (image, asset) = fixture();
        assert!(
            model
                .add_reference_image(rectangle(), image, asset)
                .is_err()
        );
        assert_eq!(model, before);
    }
    #[test]
    fn image_import_and_property_edits_survive_autosave_checkpoints() {
        let base = Document::default();
        let mut doc = base.clone();
        let (image, asset) = fixture();
        let png = Arc::clone(&asset.png);
        let before = doc.model.clone();
        let group = doc
            .model
            .add_reference_image(rectangle(), image, asset)
            .unwrap();
        doc.history
            .record("Add reference image", before, &doc.model);
        doc.selection.insert(group);
        let id = *doc.model.reference_images.keys().next().unwrap();
        let before = doc.model.clone();
        let settings = &mut doc.model.reference_images.get_mut(&id).unwrap().settings;
        settings.zoom = 1.5;
        settings.rotation = 30.;
        settings.opacity = 0.4;
        settings.brightness = 0.9;
        settings.contrast = 1.2;
        settings.flip_horizontal = true;
        settings.flip_vertical = true;
        settings.grayscale = true;
        doc.history
            .record("Edit reference image", before, &doc.model);
        let json = serde_json::to_string(&doc.checkpoint(&base.model, &base.history)).unwrap();
        let checkpoint: crate::DocumentCheckpoint = serde_json::from_str(&json).unwrap();
        let mut restored = checkpoint.restore(&base.model, &base.history).unwrap();
        assert_eq!(restored, doc);
        assert_eq!(
            restored
                .model
                .image_assets
                .values()
                .next()
                .unwrap()
                .png
                .as_ref(),
            png.as_ref()
        );
        assert!(restored.history.undo(&mut restored.model));
        assert_eq!(
            restored.model.reference_images[&id].settings,
            ImageSettings::default()
        );
        assert!(restored.history.undo(&mut restored.model));
        assert!(restored.model.reference_images.is_empty());
    }

    #[test]
    fn invalid_imports_do_not_allocate_geometry() {
        let mut model = Model::default();
        let (mut image, mut asset) = fixture();
        image.settings.zoom = f64::NAN;
        assert!(
            model
                .add_reference_image(rectangle(), image.clone(), asset.clone())
                .is_err()
        );
        image.settings.zoom = 1.;
        asset.pixels[0] = 4097;
        assert!(
            model
                .add_reference_image(rectangle(), image, asset)
                .is_err()
        );
        assert_eq!(model, Model::default());
    }
}
