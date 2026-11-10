//! Scale factors come from the same world-space targets as Move.
use super::*;

/// Idle redraws must not repeat scene snapping or geometry work. Camera,
/// precision, grid, guide and lock changes still invalidate the last query.
#[derive(PartialEq)]
pub(super) struct Input {
    pointer: Pos2,
    camera: Camera,
    viewport: Rect,
    constraint: Option<AxisConstraint>,
    text: String,
    units: benchup_core::UnitSettings,
    display: benchup_core::DisplaySettings,
    snapping: [bool; 7],
    spacing: f64,
    reference: Option<(DVec3, DVec3)>,
}
impl Input {
    pub(super) fn capture(
        app: &BenchUpApp,
        pointer: Pos2,
        constraint: Option<AxisConstraint>,
    ) -> Self {
        Self {
            pointer,
            camera: app.doc.camera.clone(),
            viewport: app.viewport,
            constraint,
            text: app.length_input.clone(),
            units: app.doc.display.units,
            display: app.doc.display.clone(),
            snapping: [
                app.snap_to_grid,
                app.doc.snap_to_geometry,
                app.doc.snap_to_axes,
                app.snap_to_edge,
                app.snap_to_face,
                app.snap_to_face_center,
                app.doc.display.guides,
            ],
            spacing: grid::configured_spacing(&app.doc.camera, &app.doc.display),
            reference: app.modeling.as_ref().map(|edit| (edit.center, edit.anchor)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Grip {
    pub point: DVec3,
    pub origin: DVec3,
    pub constraint: Option<AxisConstraint>,
    pub index: usize,
}

#[derive(Default)]
pub(super) struct Cage {
    key: Option<(u64, BTreeSet<u64>, Option<u64>)>,
    bounds: Option<(DVec3, DVec3)>,
    gpu: std::cell::RefCell<renderer::ScaleHandleCache>,
}

impl Cage {
    pub(super) fn bounds(&self) -> Option<(DVec3, DVec3)> {
        self.bounds
    }
}

fn scale_corners(edit: &Modeling) -> impl Iterator<Item = DVec3> {
    edit.scale_bounds.into_iter().flat_map(|(lo, hi)| {
        (0..8).map(move |index| {
            DVec3::new(
                if index & 1 == 0 { lo.x } else { hi.x },
                if index & 2 == 0 { lo.y } else { hi.y },
                if index & 4 == 0 { lo.z } else { hi.z },
            )
        })
    })
}

fn grips(lo: DVec3, hi: DVec3) -> Vec<Grip> {
    let mut result = Vec::with_capacity(26);
    for z in 0..3 {
        for y in 0..3 {
            for x in 0..3 {
                let coordinates = [x, y, z];
                if coordinates == [1; 3]
                    || (0..3).any(|i| hi[i] - lo[i] <= 1e-7 && coordinates[i] != 1)
                {
                    continue;
                }
                let axes: Vec<_> = [Axis::X, Axis::Y, Axis::Z]
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| coordinates[*i] != 1)
                    .map(|(_, axis)| axis)
                    .collect();
                let constraint = match axes.as_slice() {
                    [axis] => Some(AxisConstraint::Axis(*axis)),
                    [_, _] => Some(AxisConstraint::Plane(
                        [Axis::X, Axis::Y, Axis::Z]
                            .into_iter()
                            .find(|axis| !axes.contains(axis))
                            .unwrap(),
                    )),
                    _ => None,
                };
                let t = DVec3::new(x as f64, y as f64, z as f64) * 0.5;
                result.push(Grip {
                    point: lo + (hi - lo) * t,
                    origin: hi - (hi - lo) * t,
                    constraint,
                    index: x + y * 3 + z * 9,
                });
            }
        }
    }
    result
}

impl BenchUpApp {
    pub(super) fn refresh_scale_cage(&mut self, ctx: &egui::Context) {
        if self.tool != Tool::Scale || self.modeling.is_some() {
            return;
        }
        let revision = self.document_revision(ctx);
        let key = (revision, self.selected.clone(), self.doc.model.edit_context);
        if self.scale_cage.key.as_ref() != Some(&key) {
            self.scale_cage.bounds = (!self.doc.model.selection_locked(&self.selected))
                .then(|| self.doc.model.selection_bounds(&self.selected))
                .flatten();
            self.scale_cage.key = Some(key);
        }
    }

    pub(super) fn scale_grip_at(&mut self, pos: Pos2, ctx: &egui::Context) -> Option<Grip> {
        self.refresh_scale_cage(ctx);
        self.scale_grip_in_view(pos, self.viewport)
    }

    pub(super) fn scale_grip_in_view(&self, pos: Pos2, rect: Rect) -> Option<Grip> {
        let (lo, hi) = self.scale_cage.bounds?;
        // Only these at-most-26 interaction targets are projected on the CPU;
        // cage geometry, billboard expansion and clipping stay on the GPU.
        grips(lo, hi)
            .into_iter()
            .filter_map(|grip| {
                let projected = self.doc.camera.project(grip.point, dv(rect.size()))?;
                let point = rect.min + pv(projected);
                let distance = point.distance(pos);
                (rect.contains(point) && distance <= 9.).then_some((
                    grip,
                    distance,
                    self.doc.camera.eye().distance_squared(grip.point),
                ))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)))
            .map(|(grip, _, _)| grip)
    }

    pub(super) fn paint_scale_cage(&self, painter: &egui::Painter, rect: Rect) {
        if self.tool != Tool::Scale || self.navigating() || self.paste_preview.is_some() {
            return;
        }
        let Some((mut lo, mut hi)) = self.scale_cage.bounds else {
            return;
        };
        let active = self.modeling.as_ref().and_then(|edit| edit.scale_grip);
        let hovered = painter
            .ctx()
            .input(|i| i.pointer.hover_pos())
            .filter(|pos| rect.contains(*pos))
            .and_then(|pos| self.scale_grip_in_view(pos, rect));
        if let Some(edit) = &self.modeling {
            let factor = precision::modeling_value_with_units(
                Tool::Scale,
                &self.length_input,
                self.doc.display.units,
            )
            .unwrap_or(edit.value);
            let factors = DVec3::ONE
                + edit
                    .scale_constraint
                    .map_or(DVec3::ONE, |constraint| constraint.project(DVec3::ONE))
                    * (factor - 1.);
            lo = edit.center + (lo - edit.center) * factors;
            hi = edit.center + (hi - edit.center) * factors;
        }
        renderer::paint_scale_handles(
            painter,
            rect,
            &self.doc.camera,
            lo,
            hi,
            active.or(hovered).map(|grip| grip.index),
            active.map(|grip| grip.origin),
            &mut self.scale_cage.gpu.borrow_mut(),
        );
        if active.is_none()
            && let Some(grip) = hovered
        {
            let label = match grip.constraint {
                Some(AxisConstraint::Axis(axis)) => format!("Scale {} axis", axis.label()),
                Some(AxisConstraint::Plane(axis)) => format!(
                    "Scale in the {} plane",
                    match axis {
                        Axis::X => "YZ",
                        Axis::Y => "XZ",
                        Axis::Z => "XY",
                    }
                ),
                None => "Scale uniformly".into(),
            };
            painter.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if let Some(point) = self.doc.camera.project(grip.point, dv(rect.size())) {
                painter.text(
                    rect.min + pv(point) + egui::vec2(14., 14.),
                    Align2::LEFT_TOP,
                    label,
                    FontId::proportional(12.),
                    Color32::from_rgb(255, 210, 35),
                );
            }
        }
    }
}

impl BenchUpApp {
    // A tiny reference amplifies pointer rounding and snap changes. Measure
    // usable handles in pixels so sensitivity stays consistent at every zoom.
    const MIN_SCALE_REFERENCE_PIXELS: f64 = 24.;

    fn scale_reference(edit: &Modeling, camera: &Camera, size: DVec2) -> Option<DVec3> {
        let reference = edit.anchor - edit.center;
        let reference = edit
            .scale_constraint
            .map_or(reference, |lock| lock.project(reference));
        let start = camera.project(edit.anchor, size)?;
        let center = camera.project(edit.anchor - reference, size)?;
        (reference.length_squared() > 1e-12
            && start.distance(center) >= Self::MIN_SCALE_REFERENCE_PIXELS)
            .then_some(reference)
    }

    pub(super) fn scale_snap(
        edit: &Modeling,
        mut snap: Snap,
        grid_spacing: Option<f64>,
        snap_to_axes: bool,
    ) -> Snap {
        if let Some(spacing) = grid_spacing {
            if snap.kind == Inference::Plane
                && let Some(AxisConstraint::Axis(axis)) = edit.scale_constraint
            {
                snap.kind = Inference::Axis(axis);
            }
            grid::snap_with_geometry(
                snap,
                spacing,
                match edit.scale_constraint {
                    Some(AxisConstraint::Plane(normal)) => Some(normal.vector()),
                    _ => None,
                },
                grid::movement_axis(
                    snap.point,
                    Some(edit.anchor),
                    edit.scale_constraint,
                    snap_to_axes,
                ),
                &edit.base,
                Some(&edit.snap_excluded),
            )
        } else {
            snap
        }
    }

    pub(super) fn scale_value(
        edit: &Modeling,
        camera: &Camera,
        size: DVec2,
        snap_to_grid: bool,
    ) -> f64 {
        if edit.scale_constraint.is_none()
            && let Some(value) = Self::unlocked_scale_value(edit, camera, size, snap_to_grid)
        {
            return value;
        }
        let project = |delta| {
            edit.scale_constraint
                .map_or(delta, |lock| lock.project(delta))
        };
        if let Some(reference) = Self::scale_reference(edit, camera, size) {
            let target = reference + project(edit.direction);
            return if matches!(edit.scale_constraint, Some(AxisConstraint::Axis(_))) {
                target.dot(reference) / reference.length_squared()
            } else {
                target.length() / reference.length()
            };
        }

        // Center clicks, small selections, and foreshortened locked axes use
        // a visible selection extent instead of dividing by a tiny reference.
        // The snapped displacement still supplies the movement and direction.
        let extent = scale_corners(edit).fold(DVec3::ZERO, |extent, point| {
            extent.max(project(point - edit.center).abs())
        });
        let Some(start) = camera.project(edit.anchor, size) else {
            return 1.;
        };
        let free = !snap_to_grid
            && edit.snap.is_some_and(|snap| {
                snap.kind == Inference::Plane
                    || matches!(snap.kind, Inference::Axis(axis)
                    if edit.scale_constraint == Some(AxisConstraint::Axis(axis)))
            });
        let handle = [DVec3::X, DVec3::Y, DVec3::Z]
            .into_iter()
            .filter_map(|axis| {
                let world_handle = axis * extent.dot(axis);
                let endpoint = camera.project(edit.anchor + world_handle, size)?;
                let handle = endpoint - start;
                (handle.length_squared() > 1e-12).then_some((world_handle, handle))
            })
            .max_by(|(_, a), (_, b)| a.length_squared().total_cmp(&b.length_squared()));
        if let Some((world_handle, handle)) = handle {
            if !free {
                // Exact targets and grid steps keep their world-space size.
                1. + project(edit.direction).dot(world_handle) / world_handle.length_squared()
            } else {
                // A snapped face center may differ from the initial click.
                // Measure from the click so free scaling begins at exactly 1x.
                1. + dv(edit.pointer - edit.pixel).dot(handle.normalize())
                    / handle.length().max(Self::MIN_SCALE_REFERENCE_PIXELS)
            }
        } else {
            1.
        }
    }

    fn unlocked_scale_value(
        edit: &Modeling,
        camera: &Camera,
        size: DVec2,
        snap_to_grid: bool,
    ) -> Option<f64> {
        let start = camera.project(edit.anchor, size)?;
        let center = camera.project(edit.center, size)?;
        let mut reference = start - center;
        let plane = edit.snap.is_some_and(|snap| snap.kind == Inference::Plane);
        if !plane && reference.length() >= Self::MIN_SCALE_REFERENCE_PIXELS {
            // Exact geometry and automatic axis targets retain world-space ratios.
            return None;
        }
        let movement = if plane && !snap_to_grid {
            dv(edit.pointer - edit.pixel)
        } else {
            camera.project(edit.anchor + edit.direction, size)? - start
        };
        if reference.length() < Self::MIN_SCALE_REFERENCE_PIXELS {
            // A center click has no radial direction. Use a stable screen-space
            // handle sized to the selection, without choosing a model axis.
            let radius = scale_corners(edit)
                .filter_map(|point| camera.project(point, size))
                .map(|point| point.distance(center))
                .fold(Self::MIN_SCALE_REFERENCE_PIXELS, f64::max);
            reference = DVec2::new(1., -1.).normalize() * radius;
        }
        // Distance responds to movement around the center in every direction;
        // projecting onto the original reference line would create an implicit lock.
        Some((reference + movement).length() / reference.length())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cage_grips_select_opposite_origins_and_axis_plane_or_uniform_scaling() {
        let handles = grips(-DVec3::ONE, DVec3::ONE);
        assert_eq!(handles.len(), 26);
        let mut counts = [0; 3];
        for grip in handles {
            assert_eq!(grip.origin, -grip.point);
            let count = match grip.constraint {
                Some(AxisConstraint::Axis(axis)) => {
                    assert_eq!(grip.point.abs(), axis.vector());
                    0
                }
                Some(AxisConstraint::Plane(axis)) => {
                    assert_eq!(grip.point.dot(axis.vector()), 0.);
                    1
                }
                None => {
                    assert_eq!(grip.point.abs(), DVec3::ONE);
                    2
                }
            };
            counts[count] += 1;
        }
        assert_eq!(counts, [6, 12, 8]);
    }

    #[test]
    fn flat_and_linear_selections_have_distinct_usable_grips() {
        let flat = grips(DVec3::ZERO, DVec3::new(2., 4., 0.));
        assert_eq!(flat.len(), 8);
        assert!(
            flat.iter()
                .all(|grip| grip.point.z == 0. && grip.origin.z == 0.)
        );
        let line = grips(DVec3::ZERO, DVec3::X * 4.);
        assert_eq!(line.len(), 2);
        assert!(
            line.iter()
                .all(|grip| grip.constraint == Some(AxisConstraint::Axis(Axis::X)))
        );
        assert!(grips(DVec3::ONE, DVec3::ONE).is_empty());
    }
}
