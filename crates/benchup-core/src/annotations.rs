//! Persistent leader text attached to vertices, edges, and surfaces.
use crate::{DVec3, DimensionAnchor, Model, measurements::valid_point};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextAnnotation {
    pub id: u64,
    pub anchor: DimensionAnchor,
    /// Model-space displacement from the attachment to the readable label.
    pub offset: DVec3,
    pub text: String,
}
impl TextAnnotation {
    /// Screen-space hit bounds matching the 12px monospace viewport labels.
    pub fn label_size(&self) -> crate::DVec2 {
        let mut width = 0_f64;
        let mut rows = 0;
        for line in self.text.split('\n') {
            let length = line.chars().count() as f64 * 7.2;
            width = width.max(length.min(320.));
            rows += (length / 320.).ceil().max(1.) as usize;
        }
        crate::DVec2::new(width + 12., rows as f64 * 14. + 8.)
    }
    pub fn layout(&self, model: &Model) -> [DVec3; 2] {
        let anchor = self.anchor.position(model);
        [anchor, anchor + self.offset]
    }
}
impl Model {
    pub fn add_text(
        &mut self,
        point: DVec3,
        offset: DVec3,
        text: &str,
    ) -> Result<u64, &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.add_text(point, offset, text));
        }
        if !valid_point(point) || !valid_point(offset) || !valid_point(point + offset) {
            return Err(tr!("core.annotations.text_coordinates_must_be_finite_and_within_the"));
        }
        let text = checked_text(text)?;
        let anchor = self.dimension_anchor(point);
        let id = self.id();
        self.texts.insert(
            id,
            TextAnnotation {
                id,
                anchor,
                offset,
                text,
            },
        );
        Ok(id)
    }
    pub fn edit_text(&mut self, id: u64, text: &str) -> Result<(), &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.edit_text(id, text));
        }
        if self.is_locked(id) {
            return Err(tr!("core.annotations.unlock_the_text_before_editing_it"));
        }
        let text = checked_text(text)?;
        self.texts.get_mut(&id).ok_or(tr!("core.annotations.text_no_longer_exists"))?.text = text;
        Ok(())
    }
    pub(crate) fn sync_texts(&mut self) {
        let positions: Vec<_> = self
            .texts
            .values()
            .map(|t| (t.id, t.anchor.position(self)))
            .collect();
        for (id, point) in positions {
            let text = self.texts.get_mut(&id).unwrap();
            text.anchor.point = point;
            if text
                .anchor
                .weights
                .iter()
                .any(|(id, _)| !self.vertices.contains_key(id))
            {
                text.anchor.weights.clear();
            }
        }
    }
    pub(crate) fn validate_texts(&self) -> Result<(), &'static str> {
        for (id, text) in &self.texts {
            if *id != text.id || !valid_point(text.offset) || checked_text(&text.text).is_err() {
                return Err(tr!("core.annotations.invalid_text_annotation"));
            }
            text.anchor.validate(self)?;
            if text.layout(self).iter().any(|p| !valid_point(*p)) {
                return Err(tr!("core.annotations.text_is_outside_the_model_limits"));
            }
        }
        Ok(())
    }
}
fn checked_text(text: &str) -> Result<String, &'static str> {
    let text = text.trim();
    if text.is_empty() || text.len() > 4096 {
        return Err(tr!("core.annotations.enter_text_between_1_and_4096_bytes"));
    }
    Ok(text.to_owned())
}
