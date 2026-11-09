//! Compare retained edit layers with a complete fresh scene, without a GPU.
use super::*;
use std::collections::BTreeSet;

impl SceneCache {
    pub(crate) fn validate_shadow_cache(doc: &benchup_core::Document) -> Result<usize, String> {
        let key = SceneKey {
            joinery_members: None,
            joint_preview: None,
            model_revision: 1,
            edit_context: doc.model.edit_context,
            camera: doc.camera.clone(),
            size: DVec2::new(1440., 900.),
            pixels_per_point: 1.,
            display: doc.display.clone(),
            dark: false,
            tool: benchup_core::Tool::Select,
            selected: Default::default(),
            hover: None,
            outliner_hover: Default::default(),
            erasing: Default::default(),
        };
        shadows::validate(&key, doc.model.bounds().ok_or("Empty scene")?)
    }
    pub(crate) fn validate_edits(
        mut doc: benchup_core::Document,
        context: u64,
        edge: u64,
    ) -> Result<usize, String> {
        let original = doc.model.clone();
        let base_history = doc.history.clone();
        let corner = original.bounds().ok_or("Empty scene")?.1 + DVec3::splat(3.);
        let mut cache = Self::default();
        let mut faces = FaceCache::default();
        let mut entities = EntityCache::default();
        let ctx = egui::Context::default();
        let mut added_face = None;
        let mut added_group = None;
        let mut checks = 0;
        for step in 0..9 {
            let before = doc.model.clone();
            match step {
                1 => {
                    doc.model.edit_context = Some(context);
                    let points = [
                        corner,
                        corner + DVec3::X,
                        corner + DVec3::X + DVec3::Y,
                        corner + DVec3::Y,
                        corner,
                    ];
                    doc.model
                        .draw_polyline(&points, true)
                        .map_err(str::to_owned)?;
                    added_face = doc
                        .model
                        .faces
                        .keys()
                        .find(|id| !before.faces.contains_key(id))
                        .copied();
                }
                2 => doc.model.erase(&added_face.into_iter().collect()),
                3 => doc.model.erase(&BTreeSet::from([edge])),
                4 => {
                    doc.model.edit_context = doc.model.parent(context);
                    doc.model.erase(&BTreeSet::from([context]));
                }
                5 => {
                    doc.model.edit_context = None;
                    let mut drawing = Model::default();
                    let points = [
                        corner,
                        corner + DVec3::X,
                        corner + DVec3::X + DVec3::Y,
                        corner + DVec3::Y,
                        corner,
                    ];
                    drawing
                        .draw_polyline(&points, true)
                        .map_err(str::to_owned)?;
                    let ids = doc.model.paste_entities(&drawing, DVec3::ZERO);
                    added_group = doc.model.make_object(&ids, false, "New drawing");
                }
                6 => doc.model.erase(&added_group.into_iter().collect()),
                7 => {
                    doc.model
                        .add_guide(corner + DVec3::splat(10.), None)
                        .map_err(str::to_owned)?;
                }
                8 => doc.model = original.clone(),
                _ => {}
            }
            doc.model.validate().map_err(str::to_owned)?;
            if step > 0 {
                doc.history.record("Edit regression", before, &doc.model);
            }
            faces.update_for_revision(&doc.model, step + 1);
            entities.update_for_revision(&doc.model, step + 1);
            let mut fresh_entities = EntityCache::default();
            fresh_entities.update(&doc.model);
            for id in doc
                .model
                .edges
                .keys()
                .chain(doc.model.faces.keys())
                .chain(doc.model.objects.keys())
                .chain(doc.model.guides.keys())
            {
                if entities.get(*id) != fresh_entities.get(*id) {
                    return Err(format!(
                        "Inherited entity state differs after edit {step}: {id}"
                    ));
                }
            }
            checks += 1;
            let key = SceneKey {
                joinery_members: None,
                joint_preview: None,
                model_revision: step + 1,
                edit_context: doc.model.edit_context,
                camera: doc.camera.clone(),
                size: DVec2::new(1064., 748.),
                pixels_per_point: 1.,
                display: Default::default(),
                dark: false,
                tool: benchup_core::Tool::Select,
                selected: Default::default(),
                hover: None,
                outliner_hover: Default::default(),
                erasing: Default::default(),
            };
            let mut failure = None;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                cache.prepare_resident(&key, &doc.model, &faces, &entities, ui);
                if cache.resident.builds != 1 {
                    failure = Some(format!(
                        "A local edit rebuilt the retained scene at step {step}"
                    ));
                    return;
                }
                let hierarchy = cache.resident.hierarchy.as_ref().unwrap();
                let base = cache.resident.geometry.as_ref().unwrap();
                let fresh = Geometry::subset(
                    &key,
                    &doc.model,
                    &faces,
                    &entities,
                    ui,
                    hierarchy,
                    None,
                    Some(base.origin),
                );
                let removed: BTreeSet<_> = cache
                    .resident
                    .removed
                    .iter()
                    .map(|id| hierarchy.slot(*id))
                    .collect();
                let face_bytes = |face: &FaceVertex| -> [u8; std::mem::size_of::<FaceVertex>()] {
                    bytemuck::bytes_of(face).try_into().unwrap()
                };
                let edge_bytes =
                    |edge: &EdgeInstance| -> [u8; std::mem::size_of::<EdgeInstance>()] {
                        bytemuck::bytes_of(edge).try_into().unwrap()
                    };
                let mut expected: Vec<_> = fresh.faces.iter().map(face_bytes).collect();
                let mut actual: Vec<_> = base
                    .faces
                    .iter()
                    .filter(|face| !removed.contains(&face.interaction[1]))
                    .chain(
                        cache
                            .resident
                            .overlay
                            .iter()
                            .flat_map(|geometry| &geometry.faces),
                    )
                    .map(face_bytes)
                    .collect();
                expected.sort_unstable();
                actual.sort_unstable();
                if actual != expected {
                    failure = Some(format!(
                        "Face geometry differs after edit {step}: {} != {}",
                        actual.len(),
                        expected.len()
                    ));
                    return;
                }
                let mut expected: Vec<_> = fresh.edges.iter().map(edge_bytes).collect();
                let mut actual: Vec<_> = base
                    .edges
                    .iter()
                    .filter(|edge| !removed.contains(&edge.interaction[1]))
                    .chain(
                        cache
                            .resident
                            .overlay
                            .iter()
                            .flat_map(|geometry| &geometry.edges),
                    )
                    .map(edge_bytes)
                    .collect();
                expected.sort_unstable();
                actual.sort_unstable();
                if actual != expected {
                    failure = Some(format!(
                        "Edge geometry differs after edit {step}: {} != {}",
                        actual.len(),
                        expected.len()
                    ));
                    return;
                }
                // Shared-vertex batching may pack incident edges differently
                // in retained and edited layers; their visible handle positions
                // must still match a complete fresh scene after each edit/undo.
                let point_bytes = |p: [f32; 3]| p.map(f32::to_bits);
                for selected in [
                    doc.model.edges.keys().copied().collect::<BTreeSet<_>>(),
                    doc.model.edges.keys().take(3).copied().collect(),
                ] {
                    let slots: BTreeSet<_> =
                        selected.iter().map(|id| hierarchy.slot(*id)).collect();
                    let mut expected: Vec<_> = fresh
                        .markers
                        .iter()
                        .filter(|marker| marker.edges.iter().any(|slot| slots.contains(slot)))
                        .map(|marker| point_bytes(marker.position))
                        .collect();
                    let mut actual: Vec<_> = base
                        .markers
                        .iter()
                        .filter(|marker| {
                            marker
                                .edges
                                .iter()
                                .any(|slot| slots.contains(slot) && !removed.contains(slot))
                        })
                        .chain(
                            cache
                                .resident
                                .overlay
                                .iter()
                                .flat_map(|geometry| &geometry.markers)
                                .filter(|marker| {
                                    marker.edges.iter().any(|slot| slots.contains(slot))
                                }),
                        )
                        .map(|marker| point_bytes(marker.position))
                        .collect();
                    expected.sort_unstable();
                    expected.dedup();
                    actual.sort_unstable();
                    actual.dedup();
                    if expected != actual {
                        failure = Some(format!("Endpoint handles differ after edit {step}"));
                        return;
                    }
                }
                if cache.resident.bounds != doc.model.bounds() {
                    failure = Some(format!("Scene bounds differ after edit {step}"));
                    return;
                }
                if doc.model.edit_context.is_some()
                    && cache.resident_context_bounds(
                        doc.model.edit_context.unwrap(),
                        &doc.model,
                        &entities,
                    ) != faces.context_bounds(&doc.model, &entities)
                {
                    failure = Some(format!("Editing bounds differ after edit {step}"));
                }
            });
            output.textures_delta.clear();
            if let Some(error) = failure {
                return Err(error);
            }
            checks += 4;
            // A recovery checkpoint must restore exact geometry and history,
            // including an undone/branched history, before a normal full save.
            let json = serde_json::to_string(&doc.checkpoint(&original, &base_history))
                .map_err(|e| e.to_string())?;
            let checkpoint: benchup_core::DocumentCheckpoint =
                serde_json::from_str(&json).map_err(|e| e.to_string())?;
            let restored = checkpoint.restore(&original, &base_history)?;
            if serde_json::to_value(&restored).unwrap() != serde_json::to_value(&doc).unwrap() {
                return Err(format!("Autosave checkpoint differs after edit {step}"));
            }
            checks += 1;
        }
        Ok(checks)
    }
}
