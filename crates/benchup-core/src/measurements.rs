//! Measurements and persistent, geometry-attached linear and angular dimensions.
use crate::{Axis, DVec3, Model, document::EPS};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DimensionAnchor {
    pub point: DVec3,
    /// Barycentric attachment to a vertex, edge, or surface triangle.
    #[serde(default)]
    pub weights: Vec<(u64, f64)>,
}
impl DimensionAnchor {
    pub(crate) fn validate(&self, model: &Model) -> Result<(), &'static str> {
        if !valid_point(self.point)
            || self.weights.len() > 3
            || self
                .weights
                .iter()
                .map(|(id, _)| *id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.weights.len()
            || self.weights.iter().any(|(id, w)| {
                !model.vertices.contains_key(id) || !w.is_finite() || *w < -EPS || *w > 1. + EPS
            })
        {
            return Err(tr!("core.measurements.invalid_dimension_attachment"));
        }
        if !self.weights.is_empty()
            && (self.weights.iter().map(|(_, w)| w).sum::<f64>() - 1.).abs() > EPS
        {
            return Err(tr!(
                "core.measurements.invalid_dimension_attachment_weights"
            ));
        }
        if !valid_point(self.position(model)) {
            return Err(tr!("core.measurements.invalid_dimension_position"));
        }
        Ok(())
    }
    pub fn position(&self, model: &Model) -> DVec3 {
        if self.weights.is_empty()
            || self
                .weights
                .iter()
                .any(|(id, _)| !model.vertices.contains_key(id))
        {
            return self.point;
        }
        self.weights
            .iter()
            .map(|(id, w)| model.vertices[id].position * *w)
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    pub id: u64,
    pub start: DimensionAnchor,
    pub end: DimensionAnchor,
    pub offset: DVec3,
    /// An angular dimension measures start/end rays around this attached vertex.
    /// Absent in older documents and in linear dimensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex: Option<DimensionAnchor>,
    /// When set, measure the separation along this world axis. A perpendicular
    /// axis keeps the full span instead of collapsing it to a zero-length line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<Axis>,
    /// Measure the separation projected onto this world plane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plane: Option<Axis>,
}
impl Dimension {
    pub fn anchors(&self) -> impl Iterator<Item = &DimensionAnchor> {
        [&self.start, &self.end]
            .into_iter()
            .chain(self.vertex.iter())
    }
    pub fn anchors_mut(&mut self) -> impl Iterator<Item = &mut DimensionAnchor> {
        [&mut self.start, &mut self.end]
            .into_iter()
            .chain(self.vertex.iter_mut())
    }
    pub fn angle(&self, model: &Model) -> Option<f64> {
        let center = self.vertex.as_ref()?.position(model);
        let (a, b) = self.points(model);
        angle_degrees(center, a, b)
    }
    pub fn arc(&self, model: &Model) -> Vec<DVec3> {
        let Some(vertex) = &self.vertex else {
            return vec![];
        };
        let center = vertex.position(model);
        let (a, b) = self.points(model);
        let radius = self.offset.length();
        angle_arc(center, a, b)
            .into_iter()
            .map(|p| center + (p - center).normalize_or_zero() * radius)
            .collect()
    }
    pub fn label_position(&self, model: &Model) -> DVec3 {
        if let Some(vertex) = &self.vertex {
            let center = vertex.position(model);
            let (a, b) = self.points(model);
            let Some((u, v, angle)) = angle_frame(center, a, b) else {
                return center;
            };
            center + self.offset.length() * (u * (angle * 0.5).cos() + v * (angle * 0.5).sin())
        } else {
            let [_, _, a, b] = self.layout(model);
            (a + b) * 0.5
        }
    }
    pub fn annotation_points(&self, model: &Model) -> Vec<DVec3> {
        let mut points = self.layout(model).to_vec();
        if let Some(vertex) = &self.vertex {
            points.push(vertex.position(model));
            points.extend(self.arc(model));
        }
        points
    }
    pub fn segments(&self, model: &Model) -> Vec<(DVec3, DVec3)> {
        if let Some(vertex) = &self.vertex {
            let center = vertex.position(model);
            let (a, b) = self.points(model);
            let mut lines = vec![(center, a), (center, b)];
            lines.extend(self.arc(model).windows(2).map(|p| (p[0], p[1])));
            lines
        } else {
            let [a, b, c, d] = self.layout(model);
            vec![(a, c), (b, d), (c, d)]
        }
    }
    pub fn points(&self, model: &Model) -> (DVec3, DVec3) {
        (self.start.position(model), self.end.position(model))
    }
    fn projected_axis(&self, a: DVec3, b: DVec3) -> Option<Axis> {
        self.axis
            .filter(|axis| (b - a).dot(axis.vector()).abs() >= EPS)
    }
    /// Direction of the dimension line, preserving the span when the requested
    /// axis is perpendicular to it. Placement can then use that axis as an offset.
    pub fn line_direction(&self, model: &Model) -> DVec3 {
        let (a, b) = self.points(model);
        if let Some(delta) = self.projected_plane(a, b) {
            return delta.normalize();
        }
        self.projected_axis(a, b)
            .map(Axis::vector)
            .unwrap_or_else(|| (b - a).try_normalize().unwrap_or(DVec3::X))
    }
    pub fn length(&self, model: &Model) -> f64 {
        let (a, b) = self.points(model);
        if let Some(delta) = self.projected_plane(a, b) {
            return delta.length();
        }
        self.projected_axis(a, b)
            .map_or_else(|| a.distance(b), |axis| (b - a).dot(axis.vector()).abs())
    }
    pub fn layout(&self, model: &Model) -> [DVec3; 4] {
        let (a, b) = self.points(model);
        if self.vertex.is_some() {
            let arc = self.arc(model);
            return [
                a,
                b,
                arc.first().copied().unwrap_or(a),
                arc.last().copied().unwrap_or(b),
            ];
        }
        let direction = self.line_direction(model);
        let offset = self.offset - direction * self.offset.dot(direction);
        if self.projected_axis(a, b).is_some() || self.projected_plane(a, b).is_some() {
            let center = (a + b) * 0.5;
            let line = center + offset;
            [
                a,
                b,
                line + direction * (a - center).dot(direction),
                line + direction * (b - center).dot(direction),
            ]
        } else {
            [a, b, a + offset, b + offset]
        }
    }

    fn projected_plane(&self, a: DVec3, b: DVec3) -> Option<DVec3> {
        let normal = self.plane?.vector();
        let delta = b - a;
        let projected = delta - normal * delta.dot(normal);
        (projected.length() >= EPS).then_some(projected)
    }
}

pub fn angle_degrees(center: DVec3, baseline: DVec3, target: DVec3) -> Option<f64> {
    if !center.is_finite() || !baseline.is_finite() || !target.is_finite() {
        return None;
    }
    let a = (baseline - center).try_normalize()?;
    let b = (target - center).try_normalize()?;
    Some(a.cross(b).length().atan2(a.dot(b)).to_degrees())
}

fn angle_frame(center: DVec3, baseline: DVec3, target: DVec3) -> Option<(DVec3, DVec3, f64)> {
    let u = (baseline - center).try_normalize()?;
    let angle = angle_degrees(center, baseline, target)?.to_radians();
    let normal = u.cross(target - center).try_normalize().unwrap_or_else(|| {
        u.cross(if u.z.abs() < 0.9 { DVec3::Z } else { DVec3::X })
            .normalize()
    });
    Some((u, normal.cross(u), angle))
}

pub fn angle_arc(center: DVec3, baseline: DVec3, target: DVec3) -> Vec<DVec3> {
    let Some((u, v, angle)) = angle_frame(center, baseline, target) else {
        return vec![];
    };
    let radius = baseline.distance(center).min(target.distance(center)) * 0.3;
    (0..=32)
        .map(|i| {
            let t = angle * i as f64 / 32.;
            center + radius * (u * t.cos() + v * t.sin())
        })
        .collect()
}

impl Model {
    pub fn dimension_anchor(&self, point: DVec3) -> DimensionAnchor {
        let mut anchor = DimensionAnchor {
            point,
            weights: vec![],
        };
        for edge in self.edges.values().filter(|e| {
            !self.is_hidden(e.id)
                && (self.edit_context.is_none() || self.entity_in_context(e.id).is_some())
        }) {
            let (a, b) = self.points(edge);
            for (id, p) in [(edge.a, a), (edge.b, b)] {
                if p.distance(point) < EPS {
                    anchor.weights = vec![(id, 1.)];
                    return anchor;
                }
            }
        }
        for edge in self.edges.values().filter(|e| {
            !self.is_hidden(e.id)
                && (self.edit_context.is_none() || self.entity_in_context(e.id).is_some())
        }) {
            let (a, b) = self.points(edge);
            let t = (point - a).dot(b - a) / (b - a).length_squared();
            if (0. ..=1.).contains(&t) && (a + (b - a) * t).distance(point) < EPS {
                anchor.weights = vec![(edge.a, 1. - t), (edge.b, t)];
                return anchor;
            }
        }
        // Use the same triangulation as rendering, so holes never acquire attachments.
        for face in self.faces.values().filter(|f| {
            !self.is_hidden(f.id)
                && (self.edit_context.is_none() || self.entity_in_context(f.id).is_some())
                && self.face_contains(f, point, true)
        }) {
            for [a, b, c] in self.face_triangles(face) {
                let (u, v, w) = (b - a, c - a, point - a);
                let denominator = u.dot(u) * v.dot(v) - u.dot(v).powi(2);
                if denominator.abs() < 1e-16 {
                    continue;
                }
                let y = (v.dot(v) * w.dot(u) - u.dot(v) * w.dot(v)) / denominator;
                let z = (u.dot(u) * w.dot(v) - u.dot(v) * w.dot(u)) / denominator;
                let weights = [1. - y - z, y, z];
                if weights.iter().any(|w| *w < -EPS || *w > 1. + EPS) {
                    continue;
                }
                let ids: Option<Vec<_>> = [a, b, c]
                    .into_iter()
                    .map(|p| {
                        face.vertices
                            .iter()
                            .chain(face.holes.iter().flatten())
                            .find(|id| self.vertices[id].position.distance(p) < EPS)
                            .copied()
                    })
                    .collect();
                if let Some(ids) = ids {
                    anchor.weights = ids.into_iter().zip(weights).collect();
                    return anchor;
                }
            }
        }
        anchor
    }
    pub fn add_dimension(
        &mut self,
        start: DVec3,
        end: DVec3,
        offset: DVec3,
    ) -> Result<u64, &'static str> {
        self.add_dimension_with_axis(start, end, offset, None)
    }
    pub fn add_dimension_with_axis(
        &mut self,
        start: DVec3,
        end: DVec3,
        offset: DVec3,
        axis: Option<Axis>,
    ) -> Result<u64, &'static str> {
        self.add_dimension_with_constraint(start, end, offset, axis, None)
    }

    pub fn add_dimension_with_constraint(
        &mut self,
        start: DVec3,
        end: DVec3,
        offset: DVec3,
        axis: Option<Axis>,
        plane: Option<Axis>,
    ) -> Result<u64, &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| {
                model.add_dimension_with_constraint(start, end, offset, axis, plane)
            });
        }
        if !valid_point(start) || !valid_point(end) || !valid_point(offset) {
            return Err(tr!(
                "core.measurements.dimension_coordinates_must_be_finite_and_within_the"
            ));
        }
        if start.distance(end) < EPS {
            return Err(tr!(
                "core.measurements.choose_two_different_dimension_endpoints"
            ));
        }
        let (start, end) = (self.dimension_anchor(start), self.dimension_anchor(end));
        let mut dimension = Dimension {
            id: 0,
            start,
            end,
            offset,
            axis,
            plane,
            vertex: None,
        };
        if dimension.layout(self).iter().any(|p| !valid_point(*p)) {
            return Err(tr!(
                "core.measurements.dimension_is_outside_the_model_limits"
            ));
        }
        let id = self.id();
        dimension.id = id;
        self.dimensions.insert(id, dimension);
        Ok(id)
    }
    /// Save a spatial angle without introducing model edges or faces.
    pub fn add_angular_dimension(
        &mut self,
        vertex: DVec3,
        baseline: DVec3,
        target: DVec3,
    ) -> Result<u64, &'static str> {
        if self.edit_context.is_some() {
            return self
                .edit_in_context(|model| model.add_angular_dimension(vertex, baseline, target));
        }
        let mut dimension = self.angular_dimension(vertex, baseline, target)?;
        let id = self.id();
        dimension.id = id;
        self.dimensions.insert(id, dimension);
        Ok(id)
    }

    /// Adjust a completed angle without changing its ID or entity assignments.
    pub fn update_angular_dimension(
        &mut self,
        id: u64,
        vertex: DVec3,
        baseline: DVec3,
        target: DVec3,
    ) -> Result<(), &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| {
                model.update_angular_dimension(id, vertex, baseline, target)
            });
        }
        let previous = self
            .dimensions
            .get(&id)
            .filter(|d| d.vertex.is_some())
            .ok_or(tr!("core.measurements.invalid_dimension"))?;
        if self.is_locked(id) {
            return Err(tr!("core.modeling.unlock_the_selection_before_editing_it"));
        }
        let mut dimension = self.angular_dimension(vertex, baseline, target)?;
        dimension.id = id;
        dimension.offset = previous.offset;
        if dimension
            .annotation_points(self)
            .iter()
            .any(|p| !valid_point(*p))
        {
            return Err(tr!(
                "core.measurements.dimension_is_outside_the_model_limits"
            ));
        }
        self.dimensions.insert(id, dimension);
        Ok(())
    }

    fn angular_dimension(
        &self,
        vertex: DVec3,
        baseline: DVec3,
        target: DVec3,
    ) -> Result<Dimension, &'static str> {
        if [vertex, baseline, target]
            .into_iter()
            .any(|p| !valid_point(p))
        {
            return Err(tr!(
                "core.measurements.dimension_coordinates_must_be_finite_and_within_the"
            ));
        }
        if vertex.distance(baseline) < EPS || vertex.distance(target) < EPS {
            return Err(tr!(
                "core.measurements.choose_angle_points_away_from_vertex"
            ));
        }
        let dimension = Dimension {
            id: 0,
            start: self.dimension_anchor(baseline),
            end: self.dimension_anchor(target),
            vertex: Some(self.dimension_anchor(vertex)),
            offset: (baseline - vertex).normalize()
                * vertex.distance(baseline).min(vertex.distance(target))
                * 0.3,
            axis: None,
            plane: None,
        };
        if dimension
            .annotation_points(self)
            .into_iter()
            .any(|p| !valid_point(p))
        {
            return Err(tr!(
                "core.measurements.dimension_is_outside_the_model_limits"
            ));
        }
        Ok(dimension)
    }
    /// Snapshot attachments before topology is removed; detached annotations retain their last positions.
    pub(crate) fn sync_dimensions(&mut self) {
        self.sync_texts();
        let updates: Vec<_> = self
            .dimensions
            .values()
            .map(|d| {
                (
                    d.id,
                    d.anchors()
                        .map(|anchor| anchor.position(self))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        for (id, points) in updates {
            let d = self.dimensions.get_mut(&id).unwrap();
            for (anchor, point) in d.anchors_mut().zip(points) {
                anchor.point = point;
                if anchor
                    .weights
                    .iter()
                    .any(|(id, _)| !self.vertices.contains_key(id))
                {
                    anchor.weights.clear();
                }
            }
        }
    }
    pub(crate) fn validate_dimensions(&self) -> Result<(), &'static str> {
        for (id, d) in &self.dimensions {
            if *id != d.id || !valid_point(d.offset) {
                return Err(tr!("core.measurements.invalid_dimension"));
            }
            for anchor in d.anchors() {
                anchor.validate(self)?;
            }
            if d.annotation_points(self).iter().any(|p| !valid_point(*p)) {
                return Err(tr!(
                    "core.measurements.dimension_is_outside_the_model_limits"
                ));
            }
        }
        Ok(())
    }
}
pub(crate) fn valid_point(p: DVec3) -> bool {
    p.is_finite() && p.abs().max_element() <= 1e10
}
