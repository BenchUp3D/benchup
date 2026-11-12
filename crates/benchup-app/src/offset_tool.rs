//! Resolve the cursor against the face's mitered offset boundary.
use super::*;
use benchup_core::faces::{Plane, in_polygon};

struct Boundary {
    a: DVec2,
    b: DVec2,
    inward: DVec2,
    a_miter: DVec2,
    b_miter: DVec2,
}

impl Boundary {
    fn shifted(&self, distance: f64) -> (DVec2, DVec2) {
        (
            self.a + self.a_miter * distance,
            self.b + self.b_miter * distance,
        )
    }

    fn contains_offset_point(&self, point: DVec2, distance: f64) -> bool {
        let (a, b) = self.shifted(distance);
        let edge = b - a;
        if edge.dot(self.b - self.a) <= 1e-12 {
            return false;
        }
        let along = (point - a).dot(edge);
        let tolerance = edge.length() * 1e-6;
        along >= -tolerance && along <= edge.length_squared() + tolerance
    }
}

pub(super) struct Offset {
    plane: Plane,
    rings: Vec<Vec<DVec2>>,
    boundaries: Vec<Boundary>,
    active: usize,
    point: DVec2,
}

impl Offset {
    pub(super) fn new(model: &Model, face: u64) -> Option<Self> {
        let face = &model.faces[&face];
        let plane = benchup_core::faces::plane(model, &face.vertices)?;
        let mut rings = Vec::new();
        let mut boundaries = Vec::new();
        for (hole, ring) in std::iter::once(&face.vertices)
            .chain(face.holes.iter())
            .enumerate()
        {
            let points: Vec<_> = ring
                .iter()
                .map(|id| plane.project(model.vertices[id].position))
                .collect();
            let count = points.len();
            let area: f64 = (0..count)
                .map(|i| points[i].perp_dot(points[(i + 1) % count]))
                .sum();
            let sign = area.signum() * if hole == 0 { 1. } else { -1. };
            let normals: Vec<_> = (0..count)
                .map(|i| {
                    let edge = (points[(i + 1) % count] - points[i]).normalize();
                    DVec2::new(-edge.y, edge.x) * sign
                })
                .collect();
            let miters = (0..count)
                .map(|i| {
                    let (a, b) = (normals[(i + count - 1) % count], normals[i]);
                    let denominator = 1. + a.dot(b);
                    (denominator > 1e-12).then(|| (a + b) / denominator)
                })
                .collect::<Option<Vec<_>>>()?;
            boundaries.extend((0..count).map(|i| Boundary {
                a: points[i],
                b: points[(i + 1) % count],
                inward: normals[i],
                a_miter: miters[i],
                b_miter: miters[(i + 1) % count],
            }));
            rings.push(points);
        }
        Some(Self {
            plane,
            rings,
            boundaries,
            active: 0,
            point: DVec2::ZERO,
        })
    }

    pub(super) fn update(&mut self, point: DVec3) -> (DVec3, f64) {
        self.point = self.plane.project(point);
        let inside = in_polygon(self.point, &self.rings[0], true)
            && !self
                .rings
                .iter()
                .skip(1)
                .any(|ring| in_polygon(self.point, ring, true));
        // Test the shifted segment, not its infinite supporting line. This
        // switches edges at corners and keeps outward miters under the cursor.
        let target = self
            .boundaries
            .iter()
            .enumerate()
            .filter_map(|(i, boundary)| {
                let distance = (self.point - boundary.a).dot(boundary.inward);
                let correct_side = distance.abs() < 1e-7 || (distance > 0.) == inside;
                (correct_side && boundary.contains_offset_point(self.point, distance))
                    .then_some((i, distance))
            })
            .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
        let (active, distance) = target.unwrap_or_else(|| {
            // Collapsed/invalid offsets still reach the model's validation so
            // it can display the existing error instead of freezing a preview.
            let (i, boundary) = self
                .boundaries
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    selection::distance_to_segment(self.point, a.a, a.b)
                        .total_cmp(&selection::distance_to_segment(self.point, b.a, b.b))
                })
                .unwrap();
            (i, (self.point - boundary.a).dot(boundary.inward))
        });
        self.active = active;
        let inward = self.boundaries[active].inward;
        (self.plane.u * inward.x + self.plane.v * inward.y, distance)
    }

    pub(super) fn marker(&self, distance: f64) -> DVec3 {
        let (a, b) = self.boundaries[self.active].shifted(distance);
        let edge = b - a;
        let t = if edge.length_squared() > 1e-12 {
            ((self.point - a).dot(edge) / edge.length_squared()).clamp(0., 1.)
        } else {
            0.
        };
        let point = a + edge * t;
        self.plane.origin + self.plane.u * point.x + self.plane.v * point.y
    }
}
