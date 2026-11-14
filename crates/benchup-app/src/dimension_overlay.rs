//! Floating dimension inputs drawn on the geometry being sized.
//!
//! Typed characters already flow into `length_input` without focusing the
//! status-bar field. These boxes mirror that buffer (or the live value when
//! nothing is typed) centered on the edge that the value changes, so the user
//! sees where the dimension lands. Rectangles show one box per side; Tab
//! switches which side receives typing. The boxes are display-only: a second
//! focusable text field would compete with the global keyboard routing.
//!
//! Rectangle input keeps the `width,height` buffer format, so the status-bar
//! field, formulas, and commit logic are unchanged. Either side may be left
//! empty to keep the size under the cursor.

use benchup_core::tr;
use super::*;

/// Where a floating box is attached in screen space.
enum OverlayAnchor {
    /// Centered on a projected segment's midpoint.
    Segment(Pos2, Pos2),
    /// Beside a single point, such as the pointer.
    Point(Pos2),
}

struct DimensionBox {
    anchor: OverlayAnchor,
    label: &'static str,
    text: String,
    /// Receives typed characters.
    active: bool,
    /// Holds a typed (locked) value rather than the live cursor value.
    typed: bool,
    /// The whole value is selected; typing replaces it.
    selected: bool,
    /// Secondary line under the value, such as Push/Pull's resulting depth.
    note: Option<String>,
    invalid: bool,
}

/// Gap between a point anchor and the box, in points.
const POINT_GAP: f32 = 24.;

impl BenchUpApp {
    /// Label shared by the status-bar field and the floating box.
    pub(super) fn precision_field_label(&self) -> &'static str {
        match self.tool {
            Tool::Scale => tr!("app.factor"),
            Tool::Rotate | Tool::Protractor => tr!("app.angle"),
            Tool::PushPull => tr!("app.distance"),
            Tool::Offset => tr!("app.offset"),
            Tool::Move if self.last_move_copy.is_some() => tr!("app.copies"),
            Tool::Move => tr!("app.distance"),
            Tool::Rectangle | Tool::Image => tr!("app.width_height"),
            Tool::Circle | Tool::Polygon => tr!("app.radius"),
            Tool::Arc
                if self
                    .drawing
                    .as_ref()
                    .is_some_and(|drawing| drawing.arc_end.is_some()) =>
            {
                tr!("app.arc_height")
            }
            Tool::Arc => tr!("app.arc_chord"),
            _ => tr!("app.length"),
        }
    }

    /// A rectangle is being drawn, so input is split into width and height.
    pub(super) fn rect_fields_active(&self) -> bool {
        self.drawing.is_some() && matches!(self.tool, Tool::Rectangle | Tool::Image)
    }

    fn rect_parts(&self) -> (String, String) {
        match self.length_input.split_once(',') {
            Some((width, height)) => (width.to_owned(), height.to_owned()),
            None => (self.length_input.clone(), String::new()),
        }
    }

    fn set_rect_parts(&mut self, width: String, height: String) {
        self.length_input = if height.is_empty() && !self.rect_height_active {
            width
        } else {
            format!("{width},{height}")
        };
    }

    /// Insert typed text into the active rectangle side. A comma moves to height.
    /// After Tab the active side is selected, so the first character replaces it.
    pub(super) fn rect_input_text(&mut self, text: &str) {
        let (mut width, mut height) = self.rect_parts();
        let (mut height_active, mut selected) = (self.rect_height_active, self.rect_field_selected);
        for c in text.chars() {
            if c == ',' {
                height_active = true;
                selected = true;
                continue;
            }
            let field = if height_active { &mut height } else { &mut width };
            if std::mem::take(&mut selected) {
                field.clear();
            }
            field.push(c);
        }
        // Count the separator, matching the plain `length_input` limit.
        if width.len() + 1 + height.len() <= precision::MAX_INPUT_LEN {
            self.rect_height_active = height_active;
            self.rect_field_selected = selected;
            self.set_rect_parts(width, height);
        }
    }

    /// Delete from the active rectangle side.
    pub(super) fn rect_input_backspace(&mut self) {
        let (mut width, mut height) = self.rect_parts();
        let field = if self.rect_height_active { &mut height } else { &mut width };
        if std::mem::take(&mut self.rect_field_selected) {
            field.clear();
        } else {
            field.pop();
        }
        self.set_rect_parts(width, height);
    }

    /// Tab: switch typing between the rectangle's width and height.
    pub(super) fn toggle_rect_field(&mut self) {
        let (width, height) = self.rect_parts();
        self.rect_height_active = !self.rect_height_active;
        self.rect_field_selected = true;
        self.set_rect_parts(width, height);
    }

    fn rect_boxes(&self, screen: &impl Fn(DVec3) -> Option<Pos2>) -> Option<Vec<DimensionBox>> {
        // Same points as the drawn preview, including the Image tool's offset.
        let points = self.drawing_points();
        let [corner, width_end, far, ..] = points.as_slice() else {
            return None;
        };
        let units = self.doc.display.units;
        let (width, height) = self.rect_parts();
        let field = |typed: &str, active: bool, label, a: DVec3, b: DVec3| -> Option<DimensionBox> {
            // Trim only to detect an empty side; show the text as typed so a
            // trailing space is visible while entering "2 cm".
            let empty = typed.trim().is_empty();
            Some(DimensionBox {
                anchor: OverlayAnchor::Segment(screen(a)?, screen(b)?),
                label,
                text: if empty {
                    units.format_length(a.distance(b))
                } else {
                    typed.to_owned()
                },
                active,
                typed: !empty,
                selected: active && self.rect_field_selected && !empty,
                note: None,
                invalid: !empty && precision::positive_length(typed, units.length).is_err(),
            })
        };
        Some(vec![
            field(&width, !self.rect_height_active, tr!("app.width"), *corner, *width_end)?,
            field(&height, self.rect_height_active, tr!("app.height"), *width_end, *far)?,
        ])
    }

    /// Anchor and live value for the operation being sized.
    fn dimension_target(&self, screen: &impl Fn(DVec3) -> Option<Pos2>) -> Option<(OverlayAnchor, String)> {
        let units = self.doc.display.units;
        if let Some(edit) = &self.modeling {
            let value = precision::modeling_value_with_units(self.tool, &self.length_input, units)
                .unwrap_or(edit.value);
            let text = match self.tool {
                Tool::Scale => format!("{value:.3}"),
                Tool::Rotate => units.format_angle(value),
                _ => units.format_length(value),
            };
            let anchor = match self.tool {
                Tool::Move => match (screen(edit.anchor), screen(edit.anchor + edit.direction)) {
                    (Some(a), Some(b)) if a.distance(b) > 1. => OverlayAnchor::Segment(a, b),
                    _ => OverlayAnchor::Point(edit.pointer),
                },
                _ => OverlayAnchor::Point(edit.pointer),
            };
            return Some((anchor, text));
        }
        if self.last_move_copy.is_some() {
            let point = self.viewport_pointer?;
            return Some((OverlayAnchor::Point(point), "/3 · 3x".into()));
        }
        let snap = self.snap?;
        if self.tool == Tool::Line {
            let start = self.line_start?;
            let anchor = OverlayAnchor::Segment(screen(start)?, screen(snap.point)?);
            return Some((anchor, units.format_length(start.distance(snap.point))));
        }
        let drawing = self.drawing.as_ref()?;
        let origin = drawing.origin;
        let delta = snap.point - origin;
        match self.tool {
            Tool::Freehand => {
                let points = self.raw_drawing_points();
                let total: f64 = points.windows(2).map(|p| p[0].distance(p[1])).sum();
                Some((OverlayAnchor::Point(screen(snap.point)?), units.format_length(total)))
            }
            Tool::Circle | Tool::Polygon => Some((
                OverlayAnchor::Segment(screen(origin)?, screen(snap.point)?),
                units.format_length(delta.length()),
            )),
            Tool::Arc if drawing.arc_end.is_none() => Some((
                OverlayAnchor::Segment(screen(origin)?, screen(snap.point)?),
                units.format_length(delta.length()),
            )),
            Tool::Arc => {
                let (midpoint, apex, height) = self.arc_height_segment()?;
                Some((
                    OverlayAnchor::Segment(screen(midpoint)?, screen(apex)?),
                    units.format_length(height),
                ))
            }
            _ => None,
        }
    }

    /// The arc's height line: chord midpoint, apex, and height (sagitta).
    ///
    /// Matches `arc_points`: a typed height places the apex on the
    /// perpendicular through the chord midpoint, on the pointer's side;
    /// otherwise the arc passes through the pointer, and its apex is the
    /// circle point on that side above the chord midpoint.
    pub(super) fn arc_height_segment(&self) -> Option<(DVec3, DVec3, f64)> {
        let drawing = self.drawing.as_ref()?;
        let (start, end, pointer) = (drawing.origin, drawing.arc_end?, self.snap?.point);
        let chord = end - start;
        let width = chord.length();
        if width < 1e-9 {
            return None;
        }
        let u = chord / width;
        let midpoint = start + chord * 0.5;
        let offset = pointer - start;
        let x = offset.dot(u);
        let across = offset - u * x;
        let y = across.length();
        let side = across
            .try_normalize()
            .or_else(|| drawing.normal.cross(u).try_normalize())?;
        let typed = precision::positive_length(&self.length_input, self.doc.display.units.length).ok();
        let height = typed.unwrap_or_else(|| {
            if y < 1e-9 {
                return 0.;
            }
            // Circle center in the chord frame is (width / 2, cy).
            let cy = (x * (x - width) + y * y) / (2. * y);
            cy + (width * width / 4. + cy * cy).sqrt()
        });
        height.is_finite().then(|| (midpoint, midpoint + side * height, height))
    }

    /// Push/Pull on an already extruded face: its resulting depth (see `push_pull_total`).
    pub(super) fn dimension_note(&self) -> Option<String> {
        let edit = self.modeling.as_ref().filter(|_| self.tool == Tool::PushPull)?;
        let units = self.doc.display.units;
        let value = precision::modeling_value_with_units(self.tool, &self.length_input, units)
            .unwrap_or(edit.value);
        let total = self.push_pull_total(edit, value)?;
        Some(tr!("app.push_pull_total", total = units.format_length(total)))
    }

    /// Whether the typed text cannot be applied to the current operation.
    fn dimension_input_invalid(&self) -> bool {
        if self.length_input.trim().is_empty() || self.last_move_copy.is_some() {
            return false;
        }
        let units = self.doc.display.units;
        if self.modeling.is_some() {
            precision::modeling_value_with_units(self.tool, &self.length_input, units).is_err()
        } else if self.drawing.is_some() {
            self.precise_drawing_points().is_err()
        } else {
            precision::positive_length(&self.length_input, units.length).is_err()
        }
    }

    pub(super) fn paint_dimension_overlay(&self, p: &egui::Painter, rect: Rect) {
        if !self.doc.display.dimension_input_overlay
            || self.history_preview.is_some()
            || self.tool.is_measuring()
            || !self.precision_available()
        {
            return;
        }
        let size = dv(rect.size());
        let screen = |point: DVec3| self.doc.camera.project(point, size).map(|v| rect.min + pv(v));
        let boxes = if self.rect_fields_active() {
            self.rect_boxes(&screen)
        } else {
            self.dimension_target(&screen).map(|(anchor, live)| {
                let typed = !self.length_input.is_empty();
                vec![DimensionBox {
                    anchor,
                    label: self.precision_field_label(),
                    text: if typed { self.length_input.clone() } else { live },
                    active: true,
                    typed,
                    selected: false,
                    note: self.dimension_note(),
                    invalid: self.dimension_input_invalid(),
                }]
            })
        };
        // Draw the active box last so it stays on top where boxes overlap.
        let mut boxes = boxes.unwrap_or_default();
        boxes.sort_by_key(|b| b.active);
        for dimension in boxes {
            self.paint_dimension_box(p, rect, dimension);
        }
    }

    fn paint_dimension_box(&self, p: &egui::Painter, rect: Rect, dimension: DimensionBox) {
        let colors = Palette::for_painter(p);
        let show_caret = dimension.active && dimension.typed && !dimension.selected;
        let label = p.layout_no_wrap(dimension.label.to_owned(), FontId::proportional(10.), colors.muted);
        let value = p.layout_no_wrap(
            if dimension.text.is_empty() { "—".to_owned() } else { dimension.text },
            FontId::monospace(14.),
            if dimension.typed || dimension.active { colors.ink } else { colors.muted },
        );
        let padding = egui::vec2(9., 5.);
        let note = dimension
            .note
            .map(|note| p.layout_no_wrap(note, FontId::proportional(10.), colors.muted));
        let note_size = note.as_ref().map_or(Vec2::ZERO, |note| note.size() + egui::vec2(0., 1.));
        let caret = if show_caret { 5. } else { 0. };
        let inner = egui::vec2(
            label.size().x.max(value.size().x + caret).max(note_size.x),
            label.size().y + 1. + value.size().y + note_size.y,
        );
        let box_size = inner + padding * 2.;
        let center = match dimension.anchor {
            // Sit on the edge itself, where the value takes effect.
            OverlayAnchor::Segment(a, b) => a.lerp(b, 0.5),
            OverlayAnchor::Point(point) => {
                point + egui::vec2(POINT_GAP, -POINT_GAP) + box_size * egui::vec2(0.5, -0.5)
            }
        };
        let bounds = rect.shrink(8.);
        if bounds.width() < box_size.x || bounds.height() < box_size.y {
            return;
        }
        let min = (center - box_size * 0.5).clamp(bounds.min, bounds.max - box_size);
        let frame = Rect::from_min_size(min, box_size);
        // Use the theme's text color (white in dark mode) rather than the
        // inference green, so the active field stands apart from snapping cues.
        let border = if dimension.invalid {
            ACCENT
        } else if dimension.active {
            colors.ink
        } else {
            colors.border
        };
        if dimension.active {
            p.rect_filled(frame.translate(egui::vec2(0., 2.)), 6, Color32::from_black_alpha(40));
        }
        // Inactive fields recede so the one receiving typing is obvious.
        p.rect_filled(frame, 6, if dimension.active { colors.paper } else { colors.paper.gamma_multiply(0.75) });
        p.rect_stroke(
            frame,
            6,
            Stroke::new(if dimension.active { 2. } else { 1. }, border),
            egui::StrokeKind::Inside,
        );
        let label_at = frame.min + padding;
        let value_at = label_at + egui::vec2(0., label.size().y + 1.);
        let value_size = value.size();
        p.galley(label_at, label, colors.muted);
        if dimension.selected {
            let selection = p.ctx().global_style().visuals.selection.bg_fill;
            p.rect_filled(Rect::from_min_size(value_at, value_size).expand2(egui::vec2(2., 1.)), 2, selection);
        }
        p.galley(value_at, value, colors.ink);
        if let Some(note) = note {
            p.galley(value_at + egui::vec2(0., value_size.y + 1.), note, colors.muted);
        }
        if show_caret {
            let x = value_at.x + value_size.x + 2.;
            p.line_segment(
                [egui::pos2(x, value_at.y + 1.), egui::pos2(x, value_at.y + value_size.y - 1.)],
                Stroke::new(1.5, border),
            );
        }
    }
}
