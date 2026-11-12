use super::*;
use benchup_core::document::EPS;
use std::f64::consts::TAU;

impl BenchUpApp {
    pub(super) fn arc_points(&self, precise: bool) -> Result<Vec<DVec3>, &'static str> {
        let Some(drawing) = &self.drawing else {
            return Ok(vec![]);
        };
        let snap = self
            .snap
            .ok_or(tr!("app.move_over_the_drawing_plane_to_set_a"))?;
        let start = drawing.origin;
        let length = (precise && !self.length_input.trim().is_empty())
            .then(|| precision::positive_length(&self.length_input, self.doc.display.units.length))
            .transpose()?;
        let Some(end) = drawing.arc_end else {
            let mut end = snap.point;
            if let Some(length) = length {
                let direction = (end - start)
                    .try_normalize()
                    .ok_or(tr!("app.move_the_pointer_to_choose_a_direction_first"))?;
                end = start + direction * length;
            }
            if end.distance(start) < EPS {
                return Err(tr!("app.arc_distinct_endpoints"));
            }
            return Ok(vec![start, end]);
        };
        let through = if let Some(height) = length {
            let chord = (end - start).normalize();
            let across = drawing.normal.cross(chord).normalize();
            let midpoint = start + (end - start) * 0.5;
            let side = if (snap.point - midpoint).dot(across) < 0. {
                -1.
            } else {
                1.
            };
            midpoint + across * (side * height)
        } else {
            snap.point
        };
        circular_arc(start, end, through).ok_or(tr!("app.arc_point_off_chord"))
    }

    pub(super) fn set_arc_end(&mut self, ctx: &egui::Context) {
        let points = match self.arc_points(true) {
            Ok(points) => points,
            Err(message) => {
                self.say(ctx, message);
                return;
            }
        };
        let Some(drawing) = &mut self.drawing else {
            return;
        };
        let end = points[1];
        // Clearing locks must still leave a working plane through both endpoints.
        if (end - drawing.origin).dot(drawing.surface_normal).abs() > 1e-6 {
            drawing.surface_normal = drawing.normal;
            drawing.on_surface = false;
        }
        drawing.arc_end = Some(end);
        self.length_input.clear();
    }
}

fn circular_arc(start: DVec3, end: DVec3, through: DVec3) -> Option<Vec<DVec3>> {
    let chord = end - start;
    let width = chord.length();
    let u = chord / width;
    let offset = through - start;
    let x = offset.dot(u);
    let across = offset - u * x;
    let y = across.length();
    if !y.is_finite() || y < EPS.max(width.max(offset.length()) * 1e-10) {
        return None;
    }
    let v = across / y;
    let center = DVec2::new(width / 2., (x * (x - width) + y * y) / (2. * y));
    let radii = [
        -center,
        DVec2::new(x, y) - center,
        DVec2::new(width, 0.) - center,
    ];
    let mut points = vec![start];
    // Walk clockwise in the chord frame, explicitly retaining the snapped
    // through-point so it can join an existing vertex or split an edge.
    for (i, (from, to)) in [(start, through), (through, end)].into_iter().enumerate() {
        let a = radii[i];
        let b = radii[i + 1];
        let sweep = (-a.perp_dot(b).atan2(a.dot(b))).rem_euclid(TAU);
        let segments = (sweep / (TAU / 48.)).ceil() as usize;
        let radial = u * a.x + v * a.y;
        let tangent = u * a.y - v * a.x;
        for step in 1..segments {
            let angle = sweep * step as f64 / segments as f64;
            // Compute displacement from the exact span endpoint. This avoids
            // subtracting a distant circle center for shallow arcs.
            points.push(from - radial * (2. * (angle / 2.).sin().powi(2)) + tangent * angle.sin());
        }
        points.push(to);
    }
    Some(points)
}
