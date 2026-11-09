use super::*;
use crate::picking::PickingCache;
use benchup_core::DisplaySettings;
use std::collections::BTreeMap;

pub(super) struct Anchor {
    pub part: u64,
    pub point: DVec3,
    normal: Option<DVec3>,
    targets: BTreeSet<u64>,
}

impl BenchUpApp {
    fn grain_parts(&self) -> BTreeSet<u64> {
        let model = &self.doc.model;
        self.selected
            .iter()
            .copied()
            .filter(|&part| {
                model.objects.contains_key(&part)
                    && model.entity_in_context(part) == Some(part)
                    && !model.is_hidden(part)
                    && !model.is_locked(part)
            })
            .collect()
    }

    pub(super) fn grain_direction_inspector(&mut self, ui: &mut egui::Ui) {
        let parts = self.grain_parts();
        if parts.is_empty() {
            return;
        }
        let assigned = parts
            .iter()
            .any(|part| self.doc.model.objects[part].grain_axis().is_some());
        let editable = self.shared_edit_allowed(ui.ctx());
        ui.text_label(RichText::new(Tool::GrainDirection.label()).strong());
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    editable,
                    egui::Button::new(tr!("grain_direction.set_direction")),
                )
                .clicked()
            {
                self.set_tool(Tool::GrainDirection);
            }
            if ui
                .add_enabled(
                    editable && assigned,
                    egui::Button::new(tr!("grain_direction.clear")),
                )
                .clicked()
            {
                if self.edit_grain(parts, None, ui.ctx()) && self.grain_anchor.is_some() {
                    self.stop_drawing();
                }
            }
        });
    }

    pub(super) fn update_grain_pointer(&mut self, pos: Pos2) -> Option<Anchor> {
        let selected_parts = self.grain_parts();
        if self
            .grain_anchor
            .as_ref()
            .is_some_and(|anchor| anchor.targets != selected_parts)
        {
            self.stop_drawing();
        }
        let size = dv(self.viewport.size());
        let pixel = dv(pos - self.viewport.min);
        let targets = self.snap_targets();
        let lock = self.axis_constraint();
        let evaluated = self.grain_uses_evaluated_geometry();
        self.prepare_picking();
        let anchor = self.grain_anchor.as_ref();
        let source = &self.doc.model;
        let camera = &self.doc.camera;
        let display = &self.doc.display;
        let (model, faces, entities, picking) = if evaluated {
            self.joinery_geometry.evaluated_picking(source, display)
        } else {
            (
                source,
                &self.face_cache,
                &self.entity_cache,
                &mut self.picking_cache,
            )
        };
        let inferred = benchup_core::inference::infer_with_constraint(
            model,
            camera,
            size,
            pixel,
            anchor.map(|anchor| anchor.point),
            lock,
            self.doc.snap_to_geometry,
            self.doc.snap_to_axes,
            Some(&picking.snap_query(faces, entities)),
            targets,
        );
        if let Some(anchor) = anchor {
            let mut snap = inferred;
            if lock.is_none()
                && snap.is_none_or(|snap| snap.kind == Inference::Plane)
                && let Some(normal) = anchor.normal
            {
                let ray = camera.ray(pixel, size);
                let denominator = ray.direction.dot(normal);
                snap = None;
                if denominator.abs() > 1e-8 {
                    let t = (anchor.point - ray.origin).dot(normal) / denominator;
                    snap = (0.0..1e7).contains(&t).then(|| Snap {
                        point: ray.origin + ray.direction * t,
                        kind: Inference::Plane,
                    });
                }
            }
            self.hover = Some(anchor.part);
            self.snap = if snap.is_some_and(|snap| {
                matches!(
                    snap.kind,
                    Inference::Endpoint
                        | Inference::Midpoint
                        | Inference::FaceCenter
                        | Inference::Origin
                        | Inference::OnEdge
                )
            }) {
                snap
            } else {
                self.construction_snap(pos, Some(anchor.point), self.lock)
                    .or(snap)
            };
            self.update_drawing_inference(pos);
            self.apply_grid_snap();
            return None;
        }
        let hit = part_hit(
            model,
            faces,
            entities,
            picking,
            camera,
            size,
            pixel,
            inferred,
            targets.edges,
        )
        .filter(|(part, _, _)| {
            source.objects.contains_key(part)
                && !source.is_hidden(*part)
                && !source.is_locked(*part)
                && (selected_parts.len() < 2 || selected_parts.contains(part))
        });
        self.hover = hit.map(|(part, _, _)| part);
        let (part, mut snap, normal) = match hit {
            Some(hit) => hit,
            None => {
                self.snap = None;
                return None;
            }
        };
        if self.snap_to_grid && matches!(snap.kind, Inference::OnEdge | Inference::OnFace) {
            snap = grid::snap_with_geometry(
                snap,
                grid::configured_spacing(camera, display),
                None,
                None,
                model,
                None,
            );
        }
        self.snap = Some(snap);
        Some(Anchor {
            part,
            point: snap.point,
            normal,
            targets: if selected_parts.contains(&part) {
                selected_parts
            } else {
                BTreeSet::from([part])
            },
        })
    }

    pub(super) fn grain_click(&mut self, pos: Pos2, ctx: &egui::Context) {
        let hit = self.update_grain_pointer(pos);
        if let Some(anchor) = &self.grain_anchor {
            let Some(direction) = self.grain_preview().map(|(_, direction)| direction) else {
                return;
            };
            let parts = grain_references(&self.doc.model, anchor)
                .into_values()
                .collect();
            if self.edit_grain(parts, Some(direction), ctx) {
                self.stop_drawing();
                self.hover = None;
            }
        } else if let Some(anchor) = hit {
            if !self.selected.contains(&anchor.part) {
                self.selected = BTreeSet::from([anchor.part]);
                self.outliner_anchor = None;
            }
            self.grain_anchor = Some(anchor);
            self.snap = None;
        }
    }

    pub(super) fn grain_preview(&self) -> Option<(u64, DVec3)> {
        let anchor = self.grain_anchor.as_ref()?;
        let delta = self.snap?.point - anchor.point;
        (delta.length_squared() > 1e-14).then(|| (anchor.part, delta.normalize()))
    }

    pub(super) fn grain_preview_axes(&self) -> BTreeMap<u64, DVec3> {
        let Some((_, direction)) = self.grain_preview() else {
            return BTreeMap::new();
        };
        let anchor = self.grain_anchor.as_ref().unwrap();
        if anchor.targets != self.grain_parts() {
            return BTreeMap::new();
        }
        let model = &self.doc.model;
        let frame = |id: u64| model.objects[&id].transform.unwrap_or_default().linear();
        grain_references(model, anchor)
            .into_iter()
            .filter_map(|(part, reference)| {
                let axis = frame(part) * (frame(reference).inverse() * direction);
                Some((part, axis.try_normalize()?))
            })
            .collect()
    }

    pub(super) fn grain_plane(&self) -> Option<DVec3> {
        self.grain_anchor.as_ref()?.normal
    }

    pub(super) fn grain_uses_evaluated_geometry(&self) -> bool {
        self.doc.model.connections.values().any(|connection| {
            matches!(
                connection.operation,
                benchup_core::connections::ConnectionOperation::Joinery(_)
            )
        })
    }

    fn edit_grain(
        &mut self,
        parts: BTreeSet<u64>,
        direction: Option<DVec3>,
        ctx: &egui::Context,
    ) -> bool {
        if !self.shared_edit_allowed(ctx) {
            self.say(ctx, self.shared_edit_block_reason(ctx));
            return false;
        }
        let before = self.doc.model.clone();
        for part in parts {
            if let Err(message) = self.doc.model.set_grain_direction(part, direction) {
                self.doc.model = before;
                self.say(ctx, message);
                return false;
            }
        }
        let label = if direction.is_some() {
            tr!("grain_direction.assign")
        } else {
            tr!("grain_direction.clear_history")
        };
        self.finish_edit(label, before, ctx);
        true
    }
}

// A linked definition gets one local axis. If it contains the anchor, use
// that instance's frame. Otherwise use the first selected instance's frame.
fn grain_references(model: &Model, anchor: &Anchor) -> BTreeMap<u64, u64> {
    let mut definitions = BTreeMap::new();
    std::iter::once(anchor.part)
        .chain(
            anchor
                .targets
                .iter()
                .copied()
                .filter(|&part| part != anchor.part),
        )
        .map(|part| {
            let reference = match model.objects[&part].kind {
                ObjectKind::Group => part,
                ObjectKind::Component { definition } => {
                    *definitions.entry(definition).or_insert(part)
                }
            };
            (part, reference)
        })
        .collect()
}

fn part_hit(
    model: &Model,
    faces: &renderer::FaceCache,
    entities: &renderer::EntityCache,
    picking: &mut PickingCache,
    camera: &Camera,
    size: DVec2,
    pixel: DVec2,
    inferred: Option<Snap>,
    snap_to_edge: bool,
) -> Option<(u64, Snap, Option<DVec3>)> {
    let ray = camera.ray(pixel, size);
    let face = picking.nearest(faces, entities, ray, false);
    if let Some((edge, point)) = picking.pick_edge(model, faces, entities, camera, size, pixel, 8.)
        && let Some(part) = entities
            .get(edge)
            .context
            .filter(|id| model.objects.contains_key(id))
        && (snap_to_edge || face.is_none_or(|(id, _, _)| entities.get(id).context != Some(part)))
    {
        let edge = &model.edges[&edge];
        let (a, b) = model.points(edge);
        let delta = b - a;
        let snap = inferred
            .filter(|snap| {
                let t = (snap.point - a).dot(delta) / delta.length_squared();
                snap.kind != Inference::Plane
                    && snap.point.distance(a + delta * t.clamp(0., 1.)) < 1e-7
            })
            .unwrap_or(Snap {
                point,
                kind: Inference::OnEdge,
            });
        let normal = face
            .filter(|(id, _, _)| {
                entities.get(*id).context == Some(part)
                    && faces.adjacent_faces(edge.a, edge.b).contains(id)
            })
            .map(|(id, _, _)| faces.get(id).normal)
            .or_else(|| {
                faces
                    .adjacent_faces(edge.a, edge.b)
                    .iter()
                    .find(|&&id| !entities.get(id).hidden && entities.get(id).context == Some(part))
                    .map(|&id| faces.get(id).normal)
            });
        return Some((part, snap, normal));
    }
    let (id, _, point) = face?;
    let face = faces.get(id);
    let snap = inferred
        .filter(|snap| snap.kind != Inference::Plane && face.contains_point(snap.point))
        .unwrap_or(Snap {
            point,
            kind: Inference::OnFace,
        });
    Some((entities.get(id).context?, snap, Some(face.normal)))
}

pub(super) fn paint(
    mesh: &mut renderer::Mesh,
    model: &Model,
    faces: &[renderer::FaceDraw<'_>],
    display: &DisplaySettings,
    preview: &BTreeMap<u64, DVec3>,
) {
    if !display.grain_direction && preview.is_empty() {
        return;
    }
    let mut axes = BTreeMap::new();
    if display.grain_direction {
        axes.extend(
            model
                .objects
                .values()
                .filter_map(|object| Some((object.id, (object.world_grain_axis()?, false)))),
        );
    }
    axes.extend(preview.iter().map(|(&part, &axis)| (part, (axis, true))));
    if axes.is_empty() {
        return;
    }
    let parents: BTreeMap<_, _> = model
        .objects
        .values()
        .flat_map(|object| object.entities.iter().map(move |&child| (child, object.id)))
        .collect();
    for face in faces {
        let mut ancestors = std::iter::successors(parents.get(&face.face.id).copied(), |id| {
            parents.get(id).copied()
        });
        // Prefer the preview on its own geometry even when a nested part has
        // an assignment. Otherwise use the closest assigned ancestor.
        let part = ancestors
            .clone()
            .find(|id| axes.get(id).is_some_and(|(_, preview)| *preview))
            .or_else(|| ancestors.find(|id| axes.contains_key(id)));
        let Some(part) = part else { continue };
        let (axis, preview) = axes[&part];
        // Painted faces can be light or dark in either theme.
        let color = if face.color.intensity() > 0.52 {
            Color32::from_rgb(67, 27, 143)
        } else {
            Color32::from_rgb(231, 197, 255)
        };
        let width = if preview { 1.15 } else { 0.85 };
        mesh.surface_grain(face.cached, axis, width, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains(triangles: &[[DVec2; 3]], point: DVec2) -> bool {
        triangles.iter().any(|&[a, b, c]| {
            let area = (b - a).perp_dot(c - a);
            area.abs() > 1e-6
                && [(a, b), (b, c), (c, a)]
                    .into_iter()
                    .all(|(a, b)| (b - a).perp_dot(point - a) * area.signum() >= -1e-7)
        })
    }

    #[test]
    fn grain_density_and_dash_length_are_independent_of_face_size() {
        let size = DVec2::new(800., 650.);
        let mut camera = Camera {
            distance: 2.,
            projection: benchup_core::Projection::Parallel,
            ..Camera::default()
        };
        camera.set_standard_view(benchup_core::StandardView::Top);
        let view = camera.view(size);
        // Vary each dimension independently as well as scaling the whole face.
        for (width, height) in [(0.4, 0.3), (0.8, 0.3), (0.4, 0.6), (1.6, 1.2)] {
            let mut model = Model::default();
            let corners = [
                DVec3::new(-width, -height, 0.) * 0.5,
                DVec3::new(width, -height, 0.) * 0.5,
                DVec3::new(width, height, 0.) * 0.5,
                DVec3::new(-width, height, 0.) * 0.5,
            ];
            for i in 0..4 {
                model.ensure_edge(corners[i], corners[(i + 1) % 4]).unwrap();
            }
            let part = model
                .make_object(&model.selectable_entities(), false, "Board")
                .unwrap();
            let mut cache = renderer::FaceCache::default();
            cache.update(&model);
            for axis in [DVec3::X, -DVec3::X, DVec3::Z] {
                model.set_grain_direction(part, Some(axis)).unwrap();
                let faces: Vec<_> = model
                    .faces
                    .values()
                    .map(|face| renderer::FaceDraw {
                        face,
                        cached: cache.get(face.id),
                        color: Color32::WHITE,
                        shadow: None,
                    })
                    .collect();
                for preview in [false, true] {
                    let display = DisplaySettings {
                        grain_direction: !preview,
                        ..DisplaySettings::default()
                    };
                    let mut mesh = renderer::Mesh::new(&camera, size);
                    mesh.draw_faces(&faces);
                    mesh.set_entity(None);
                    let axes = if preview {
                        BTreeMap::from([(part, axis)])
                    } else {
                        BTreeMap::new()
                    };
                    paint(&mut mesh, &model, &faces, &display, &axes);
                    let strokes = mesh.stroke_triangles();
                    let samples = if axis.z == 0. {
                        // The center dash stays short, and adjacent dashes keep
                        // their pitch when the face grows in either direction.
                        [(0., true), (0.02, true), (0.06, false), (0.135, true)]
                    } else {
                        // End-grain dots use the same fixed model-space scale.
                        [(0., true), (0.05, false), (0.1, true), (0.15, false)]
                    };
                    for (offset, covered) in samples {
                        for sign in [-1., 1.] {
                            let point = view.project(DVec3::X * (offset * sign)).unwrap();
                            assert_eq!(
                                contains(&strokes, point), covered,
                                "face {width} x {height}, axis {axis:?}, preview {preview}, offset {}",
                                offset * sign
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn grain_stays_anchored_and_thin_while_orbiting_and_zooming() {
        let mut model = Model::default();
        let corners = [
            DVec3::ZERO,
            DVec3::X * 0.8,
            DVec3::new(0.8, 0.6, 0.),
            DVec3::Y * 0.6,
        ];
        for i in 0..corners.len() {
            model
                .ensure_edge(corners[i], corners[(i + 1) % corners.len()])
                .unwrap();
        }
        let mut cache = renderer::FaceCache::default();
        cache.update(&model);
        let (id, face) = cache.iter().next().unwrap();
        let size = DVec2::new(800., 650.);
        let render = |camera: &Camera, axis| {
            let mut mesh = renderer::Mesh::new(camera, size);
            mesh.draw_faces(&[renderer::FaceDraw {
                face: &model.faces[&id],
                cached: face,
                color: Color32::WHITE,
                shadow: None,
            }]);
            mesh.set_entity(None);
            mesh.surface_grain(face, axis, 0.85, Color32::BLACK);
            mesh.stroke_triangles()
        };
        let mut camera = Camera {
            target: face.center,
            distance: 2.4,
            ..Camera::default()
        };
        for (yaw, pitch, distance) in [(0.1, 1.1, 2.4), (1.2, 0.65, 2.4), (-0.8, 0.95, 1.8)] {
            camera.yaw = yaw;
            camera.pitch = pitch;
            camera.distance = distance;
            let view = camera.view(size);
            let strokes = render(&camera, DVec3::X);
            for (offset, covered) in [
                (-0.06, false),
                (-0.024, true),
                (0., true),
                (0.024, true),
                (0.06, false),
            ] {
                let point = view.project(face.center + DVec3::X * offset).unwrap();
                assert_eq!(contains(&strokes, point), covered);
            }
            let dots = render(&camera, DVec3::Z);
            assert!(contains(&dots, view.project(face.center).unwrap()));
            assert!(!contains(
                &dots,
                view.project(face.center + DVec3::X * 0.05).unwrap()
            ));
        }

        camera.set_standard_view(benchup_core::StandardView::Top);
        for distance in [0.6, 2.4, 4.8] {
            camera.distance = distance;
            let strokes = render(&camera, DVec3::X);
            let view = camera.view(size);
            let center = view.project(face.center).unwrap();
            let direction =
                (view.project(face.center + DVec3::X * 0.1).unwrap() - center).normalize();
            let across = DVec2::new(-direction.y, direction.x);
            for sign in [-1., 1.] {
                assert!(contains(&strokes, center + across * (0.88 * sign)));
                assert!(!contains(&strokes, center + across * sign));
            }
        }
    }

    #[test]
    fn assigned_and_preview_grain_repeat_on_each_visible_face() {
        let mut model = Model::default();
        for along in [-DVec3::X * 0.8, -DVec3::Z * 0.8] {
            let corners = [DVec3::ZERO, along, along + DVec3::Y * 0.8, DVec3::Y * 0.8];
            for i in 0..4 {
                model.ensure_edge(corners[i], corners[(i + 1) % 4]).unwrap();
            }
        }
        let part = model
            .make_object(&model.selectable_entities(), false, "Board")
            .unwrap();
        model.set_grain_direction(part, Some(DVec3::X)).unwrap();
        let camera = Camera {
            target: DVec3::new(-0.3, 0.4, -0.3),
            yaw: 0.,
            pitch: 0.55,
            distance: 2.8,
            projection: benchup_core::Projection::Parallel,
            ..Camera::default()
        };
        let size = DVec2::new(600., 500.);
        let view = camera.view(size);
        let seam = view.project(DVec3::ZERO).unwrap().y;
        let mut cache = renderer::FaceCache::default();
        cache.update(&model);
        let faces: Vec<_> = model
            .faces
            .values()
            .map(|face| renderer::FaceDraw {
                face,
                cached: cache.get(face.id),
                color: Color32::WHITE,
                shadow: None,
            })
            .collect();
        assert_eq!(faces.len(), 2);
        let render = |show, preview, section, occlude| {
            let display = DisplaySettings {
                grain_direction: show,
                section_cuts: section,
                section_axes: [true, false, false],
                section_offsets: [-0.2, 0.],
                ..DisplaySettings::default()
            };
            let mut mesh = renderer::Mesh::new(&camera, size);
            mesh.set_display(&display);
            mesh.enable_gpu_edge_occlusion();
            mesh.draw_faces(&faces);
            mesh.set_entity(None);
            if occlude {
                let toward_eye = (camera.eye() - camera.target).normalize();
                for face in &faces {
                    if face.cached.normal.z.abs() > 0.9 {
                        for triangle in &face.cached.triangles {
                            mesh.world_triangle(
                                &camera,
                                triangle.map(|p| p + toward_eye * 0.2),
                                Color32::WHITE,
                            );
                        }
                    }
                }
            }
            paint(&mut mesh, &model, &faces, &display, preview);
            mesh.stroke_triangles()
        };
        let is_side = |triangle: &[DVec2; 3]| triangle.iter().map(|p| p.y).sum::<f64>() / 3. < seam;
        let preview = BTreeMap::from([(part, DVec3::X)]);
        let no_preview = BTreeMap::new();
        let assigned_marks = render(true, &no_preview, false, false);
        let preview_marks = render(false, &preview, false, false);
        assert_eq!(preview_marks, render(true, &preview, false, false));
        assert!(render(false, &no_preview, false, false).is_empty());
        for marks in [assigned_marks, preview_marks] {
            let (side, end): (Vec<_>, Vec<_>) = marks.into_iter().partition(is_side);
            for marks in [&side, &end] {
                let (lo, hi) = marks
                    .iter()
                    .flatten()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                        (lo.min(p.x), hi.max(p.x))
                    });
                assert!(hi - lo > 30., "both faces need repeated marks");
            }
            let height = |triangle: &[DVec2; 3]| {
                let [a, b, c] = triangle.map(|p| p.y);
                a.max(b).max(c) - a.min(b).min(c)
            };
            let side_unit = (view.project(-DVec3::X * 0.2).unwrap().y - seam).abs();
            let end_unit = (view.project(-DVec3::Z * 0.2).unwrap().y - seam).abs();
            assert!(
                side.iter()
                    .any(|triangle| height(triangle) > 0.35 * side_unit)
            );
            assert!(side.iter().all(|triangle| {
                let [a, b, c] = triangle.map(|p| p.x);
                a.max(b).max(c) - a.min(b).min(c) < 2.2
            }));
            assert!(end.iter().all(|triangle| height(triangle) < 0.2 * end_unit));
        }

        let cut = render(true, &no_preview, true, false);
        let cut_y = view.project(-DVec3::X * 0.2).unwrap().y;
        assert!(!cut.is_empty());
        assert!(cut.iter().flatten().all(|p| p.y <= cut_y + 1e-3));

        let occluded = render(true, &no_preview, false, true);
        assert!(occluded.iter().any(|triangle| !is_side(triangle)));
        assert!(
            occluded
                .iter()
                .filter(|triangle| is_side(triangle))
                .all(|triangle| {
                    let [a, b, c] = *triangle;
                    (b - a).perp_dot(c - a).abs() < 1e-3
                })
        );
    }
}
