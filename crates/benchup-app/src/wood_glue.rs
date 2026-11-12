use super::*;
use benchup_core::tr;

impl BenchUpApp {
    pub(super) fn wood_glue_face_at(&mut self, pos: Pos2) -> Option<u64> {
        self.prepare_picking();
        let ray = self
            .doc
            .camera
            .ray(dv(pos - self.viewport.min), dv(self.viewport.size()));
        let (id, _, _) =
            self.picking_cache
                .nearest(&self.face_cache, &self.entity_cache, ray, false)?;
        let model = &self.doc.model;
        (model.faces.contains_key(&id)
            && model.entity_in_context(id).is_some()
            && !model.is_locked(id)
            && !model.is_hidden(id)
            && !model.reference_images.contains_key(&id))
        .then_some(id)
    }

    fn wood_glue_edit_allowed(&mut self, ctx: &egui::Context) -> bool {
        if self.project.blocked() || self.history_preview.is_some() {
            return false;
        }
        if !self.shared_edit_allowed(ctx) {
            self.say(ctx, self.shared_edit_block_reason(ctx));
            return false;
        }
        true
    }

    pub(super) fn wood_glue_click(&mut self, pos: Pos2, ctx: &egui::Context) {
        if !self.wood_glue_edit_allowed(ctx) {
            return;
        }
        let Some(face) = self.wood_glue_face_at(pos) else {
            self.say(ctx, Tool::WoodGlue.instructions());
            return;
        };
        let before = self.doc.model.clone();
        if self.doc.model.wood_glue.remove(&face).is_some() {
            self.finish_edit(tr!("wood_glue.removed"), before, ctx);
        } else if self.doc.model.glue_face(face) {
            self.finish_edit(tr!("wood_glue.applied"), before, ctx);
        }
    }

    pub(super) fn glue_selection(&mut self, ctx: &egui::Context) {
        if !self.wood_glue_edit_allowed(ctx) {
            return;
        }
        if !self.shared_fastener_edit_allowed(ctx, &self.selected) {
            self.say(ctx, self.shared_edit_block_reason(ctx));
            return;
        }
        let before = self.doc.model.clone();
        if self.doc.model.glue_contacts(&self.selected) > 0 {
            self.finish_edit(tr!("wood_glue.contacts_applied"), before, ctx);
        } else {
            self.say(ctx, tr!("wood_glue.no_new_contacts"));
        }
    }
}
