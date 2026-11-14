//! A PNG preview of the model from a standard view, so an agent can check a
//! result without opening BenchUp (`benchup render`, MCP `render_view`).
//!
//! This is an offline export for command-line agents, not the interactive
//! viewport (which renders on the GPU): a small software rasterizer with a
//! depth buffer, flat shading, visible edges and dimensions, so it runs
//! anywhere the CLI does, without a display or GPU. Joints and router
//! profiles are shown cut, as BenchUp evaluates them.
use crate::scene;
use benchup_core::{DVec3, Model};
use flate2::{Compression, write::ZlibEncoder};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Write;

pub const VIEWS: &[&str] = &["iso", "front", "back", "left", "right", "top", "bottom"];
pub const MAX_SIZE: u32 = 2400;
pub const MIN_SIZE: u32 = 64;
/// Supersampling factor per axis, for smooth edges.
const SAMPLES: usize = 2;
const BACKGROUND: [u8; 3] = [246, 246, 244];
const WOOD: [f64; 3] = [222., 196., 158.];
const EDGE: [f64; 3] = [40., 40., 40.];
const DIMENSION: [f64; 3] = [30., 90., 200.];
const TEXT_MARK: [f64; 3] = [200., 80., 30.];
/// Faces meeting at less than 20° are drawn without the edge between them.
const SMOOTH_COS: f64 = 0.94;
const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;

pub struct Rendered {
    pub png: Vec<u8>,
    /// The view, image size and what the image shows about dimensions.
    pub info: Value,
}

/// Camera axes for a named view: (forward, right, up).
fn view_axes(view: &str) -> Option<(DVec3, DVec3, DVec3)> {
    let forward = match view {
        "iso" => DVec3::new(1., 1., -1.).normalize(),
        "front" => DVec3::Y,
        "back" => -DVec3::Y,
        "left" => DVec3::X,
        "right" => -DVec3::X,
        "top" => -DVec3::Z,
        "bottom" => DVec3::Z,
        _ => return None,
    };
    let up_hint = if forward.z.abs() > 0.99 { DVec3::Y * -forward.z.signum() } else { DVec3::Z };
    let right = forward.cross(up_hint).normalize();
    let up = right.cross(forward).normalize();
    Some((forward, right, up))
}

struct Canvas {
    width: usize,
    height: usize,
    color: Vec<[f64; 3]>,
    depth: Vec<f64>,
}
impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        let background = BACKGROUND.map(f64::from);
        Self { width, height, color: vec![background; width * height], depth: vec![f64::INFINITY; width * height] }
    }
    fn triangle(&mut self, p: [[f64; 3]; 3], color: [f64; 3]) {
        let min_x = p.iter().map(|v| v[0]).fold(f64::INFINITY, f64::min).floor().max(0.) as usize;
        let max_x = p.iter().map(|v| v[0]).fold(f64::NEG_INFINITY, f64::max).ceil().min(self.width as f64 - 1.);
        let min_y = p.iter().map(|v| v[1]).fold(f64::INFINITY, f64::min).floor().max(0.) as usize;
        let max_y = p.iter().map(|v| v[1]).fold(f64::NEG_INFINITY, f64::max).ceil().min(self.height as f64 - 1.);
        if max_x < 0. || max_y < 0. {
            return;
        }
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-12 {
            return;
        }
        for y in min_y..=max_y as usize {
            for x in min_x..=max_x as usize {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((p[1][0] - px) * (p[2][1] - py) - (p[2][0] - px) * (p[1][1] - py)) / area;
                let w1 = ((p[2][0] - px) * (p[0][1] - py) - (p[0][0] - px) * (p[2][1] - py)) / area;
                let w2 = 1. - w0 - w1;
                if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                    continue;
                }
                let z = w0 * p[0][2] + w1 * p[1][2] + w2 * p[2][2];
                let i = y * self.width + x;
                if z < self.depth[i] {
                    self.depth[i] = z;
                    self.color[i] = color;
                }
            }
        }
    }
    /// A line, hidden where something is in front of it by more than `bias`
    /// (`None` draws it on top of everything).
    fn line(&mut self, a: [f64; 3], b: [f64; 3], color: [f64; 3], thickness: f64, bias: Option<f64>) {
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        let steps = (length * 1.5).ceil().max(1.) as usize;
        let radius = (thickness / 2.).max(0.5);
        let reach = radius.ceil() as i64;
        for k in 0..=steps {
            let t = k as f64 / steps as f64;
            let (x, y, z) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t);
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    if ((dx * dx + dy * dy) as f64).sqrt() > radius {
                        continue;
                    }
                    let (px, py) = (x.floor() as i64 + dx, y.floor() as i64 + dy);
                    if px < 0 || py < 0 || px >= self.width as i64 || py >= self.height as i64 {
                        continue;
                    }
                    let i = py as usize * self.width + px as usize;
                    if bias.is_none_or(|bias| z <= self.depth[i] + bias) {
                        self.color[i] = color;
                    }
                }
            }
        }
    }
    fn text(&mut self, text: &str, center: [f64; 2], scale: f64, color: [f64; 3]) {
        let glyphs: Vec<&[u8; GLYPH_H]> = text.chars().filter_map(glyph).collect();
        let advance = (GLYPH_W + 1) as f64 * scale;
        let width = advance * glyphs.len() as f64 - scale;
        let (left, top) = (center[0] - width / 2., center[1] - GLYPH_H as f64 * scale / 2.);
        // A light plate behind the label keeps it readable over the model.
        let pad = scale * 1.5;
        let plate = BACKGROUND.map(f64::from);
        for y in (top - pad).max(0.) as usize..((top + GLYPH_H as f64 * scale + pad).max(0.) as usize).min(self.height) {
            for x in (left - pad).max(0.) as usize..((left + width + pad).max(0.) as usize).min(self.width) {
                self.color[y * self.width + x] = plate;
            }
        }
        for (n, rows) in glyphs.iter().enumerate() {
            for (row, bits) in rows.iter().enumerate() {
                for column in 0..GLYPH_W {
                    if bits & (1 << (GLYPH_W - 1 - column)) == 0 {
                        continue;
                    }
                    let x0 = left + n as f64 * advance + column as f64 * scale;
                    let y0 = top + row as f64 * scale;
                    for y in y0.max(0.) as usize..((y0 + scale).max(0.) as usize).min(self.height) {
                        for x in x0.max(0.) as usize..((x0 + scale).max(0.) as usize).min(self.width) {
                            self.color[y * self.width + x] = color;
                        }
                    }
                }
            }
        }
    }
    /// Average each SAMPLES x SAMPLES block into one pixel.
    fn downsample(&self) -> Vec<u8> {
        let (w, h) = (self.width / SAMPLES, self.height / SAMPLES);
        let mut out = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0.; 3];
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let c = self.color[(y * SAMPLES + sy) * self.width + x * SAMPLES + sx];
                        for k in 0..3 {
                            sum[k] += c[k];
                        }
                    }
                }
                let n = (SAMPLES * SAMPLES) as f64;
                out.extend(sum.map(|v| (v / n).round().clamp(0., 255.) as u8));
            }
        }
        out
    }
}

fn glyph(c: char) -> Option<&'static [u8; GLYPH_H]> {
    const DIGITS: [[u8; GLYPH_H]; 10] = [
        [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
    ];
    const LETTERS: [[u8; GLYPH_H]; 26] = [
        [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001],
        [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010],
        [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
    ];
    const DOT: [u8; GLYPH_H] = [0, 0, 0, 0, 0, 0b01100, 0b01100];
    const MINUS: [u8; GLYPH_H] = [0, 0, 0, 0b11111, 0, 0, 0];
    const SPACE: [u8; GLYPH_H] = [0; GLYPH_H];
    const COMMA: [u8; GLYPH_H] = [0, 0, 0, 0, 0b00110, 0b00100, 0b01000];
    const COLON: [u8; GLYPH_H] = [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0];
    const SLASH: [u8; GLYPH_H] = [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000];
    const OPEN: [u8; GLYPH_H] = [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010];
    const CLOSE: [u8; GLYPH_H] = [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000];
    const PLUS: [u8; GLYPH_H] = [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0];
    const EQUALS: [u8; GLYPH_H] = [0, 0, 0b11111, 0, 0b11111, 0, 0];
    const QUESTION: [u8; GLYPH_H] = [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100];
    const BANG: [u8; GLYPH_H] = [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100];
    const QUOTE: [u8; GLYPH_H] = [0b01010, 0b01010, 0, 0, 0, 0, 0];
    const APOSTROPHE: [u8; GLYPH_H] = [0b00100, 0b00100, 0, 0, 0, 0, 0];
    const DEGREE: [u8; GLYPH_H] = [0b01100, 0b10010, 0b10010, 0b01100, 0, 0, 0];
    const HASH: [u8; GLYPH_H] = [0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010];
    match c {
        '0'..='9' => Some(&DIGITS[c as usize - '0' as usize]),
        'A'..='Z' => Some(&LETTERS[c as usize - 'A' as usize]),
        '.' => Some(&DOT),
        '-' | '_' => Some(&MINUS),
        ' ' => Some(&SPACE),
        ',' | ';' => Some(&COMMA),
        ':' => Some(&COLON),
        '/' => Some(&SLASH),
        '(' | '[' => Some(&OPEN),
        ')' | ']' => Some(&CLOSE),
        '+' => Some(&PLUS),
        '=' => Some(&EQUALS),
        '!' => Some(&BANG),
        '"' => Some(&QUOTE),
        '\'' => Some(&APOSTROPHE),
        '°' => Some(&DEGREE),
        '#' => Some(&HASH),
        _ => Some(&QUESTION),
    }
}

/// Upper case without accents, as the preview font has them: "Mesilla ñ" → "MESILLA N".
fn fold_text(text: &str) -> String {
    text.chars()
        .flat_map(char::to_uppercase)
        .map(|c| match c {
            'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'A',
            'É' | 'È' | 'Ê' | 'Ë' => 'E',
            'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
            'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => 'O',
            'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
            'Ñ' => 'N',
            'Ç' => 'C',
            '×' => 'X',
            '«' | '»' | '“' | '”' => '"',
            '‘' | '’' => '\'',
            '–' | '—' => '-',
            other => other,
        })
        .take(80)
        .collect()
}

fn png(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, String> {
    let mut raw = Vec::with_capacity(rgb.len() + height as usize);
    for row in rgb.chunks(width as usize * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&raw).map_err(|e| e.to_string())?;
    let data = encoder.finish().map_err(|e| e.to_string())?;
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut typed = kind.to_vec();
        typed.extend_from_slice(body);
        out.extend_from_slice(&typed);
        out.extend_from_slice(&crate::files::png_crc(&typed).to_be_bytes());
    };
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &data);
    chunk(b"IEND", &[]);
    Ok(out)
}

fn label(length_mm: f64) -> String {
    let rounded = (length_mm * 10.).round() / 10.;
    if (rounded - rounded.round()).abs() < 1e-9 { format!("{}", rounded.round() as i64) } else { format!("{rounded:.1}") }
}

/// Render `model` from `view` at `width` x `height` pixels.
pub fn render(model: &Model, view: &str, width: u32, height: u32) -> Result<Rendered, String> {
    let (forward, right, up) = view_axes(view).ok_or_else(|| format!("unknown view {view}; use one of {}", VIEWS.join(", ")))?;
    if !(MIN_SIZE..=MAX_SIZE).contains(&width) || !(MIN_SIZE..=MAX_SIZE).contains(&height) {
        return Err(format!("the image must be between {MIN_SIZE} and {MAX_SIZE} pixels on each side"));
    }
    // Show the boards as they will be built: joints and profiles cut.
    let evaluated = if model.has_machining() { model.evaluate_joinery().model } else { model.clone() };
    let shown = |m: &Model, id: u64| !m.is_hidden(id);
    let faces: Vec<(u64, Vec<[DVec3; 3]>)> = evaluated
        .faces
        .values()
        .filter(|f| shown(&evaluated, f.id))
        .map(|f| (f.id, evaluated.face_triangles(f)))
        .collect();
    // An edge between two faces that meet almost flat is a facet of a curve,
    // not a corner: leave it out so curves read smooth.
    let mut normals: std::collections::BTreeMap<(u64, u64), Vec<DVec3>> = std::collections::BTreeMap::new();
    for face in evaluated.faces.values().filter(|f| shown(&evaluated, f.id)) {
        let Some(plane) = benchup_core::faces::plane(&evaluated, &face.vertices) else { continue };
        for ring in std::iter::once(&face.vertices).chain(&face.holes) {
            for (i, a) in ring.iter().enumerate() {
                let b = ring[(i + 1) % ring.len()];
                normals.entry(((*a).min(b), (*a).max(b))).or_default().push(plane.normal);
            }
        }
    }
    let facet = |a: u64, b: u64| matches!(normals.get(&(a.min(b), a.max(b))).map(Vec::as_slice), Some([n, m]) if n.dot(*m).abs() > SMOOTH_COS);
    let edges: Vec<(DVec3, DVec3)> = evaluated
        .edges
        .values()
        .filter(|e| shown(&evaluated, e.id) && !facet(e.a, e.b))
        .map(|e| evaluated.points(e))
        .collect();
    // As BenchUp draws them: extension lines and the dimension line (along
    // its axis when it has one), with the value at the label position.
    struct Shown {
        id: u64,
        segments: Vec<(DVec3, DVec3)>,
        line: (DVec3, DVec3),
        label: DVec3,
        length: f64,
    }
    let dimensions: Vec<Shown> = model
        .dimensions
        .iter()
        .filter(|(id, _)| shown(model, **id))
        .map(|(id, d)| {
            let [_, _, c, e] = d.layout(model);
            Shown { id: *id, segments: d.segments(model), line: (c, e), label: d.label_position(model), length: d.length(model) }
        })
        .collect();
    let texts: Vec<(u64, DVec3, DVec3, String)> = model
        .texts
        .iter()
        .filter(|(id, _)| shown(model, **id))
        .map(|(id, t)| (*id, t.anchor.position(model), t.offset, t.text.clone()))
        .collect();
    // Fit everything drawn, with a margin.
    let mut points: Vec<DVec3> = faces.iter().flat_map(|(_, t)| t.iter().flatten().copied()).collect();
    points.extend(edges.iter().flat_map(|(a, b)| [*a, *b]));
    for d in &dimensions {
        points.extend(d.segments.iter().flat_map(|(a, b)| [*a, *b]));
    }
    for (_, anchor, offset, _) in &texts {
        points.extend([*anchor, *anchor + *offset]);
    }
    let (w, h) = (width as usize * SAMPLES, height as usize * SAMPLES);
    let mut canvas = Canvas::new(w, h);
    let empty = points.is_empty();
    let project_2d = |p: DVec3| (p.dot(right), p.dot(up));
    let (mut lo, mut hi) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
    let mut depth_range = (f64::INFINITY, f64::NEG_INFINITY);
    for p in &points {
        let (x, y) = project_2d(*p);
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
        let d = p.dot(forward);
        depth_range = (depth_range.0.min(d), depth_range.1.max(d));
    }
    if empty {
        lo = (-0.5, -0.5);
        hi = (0.5, 0.5);
        depth_range = (0., 1.);
    }
    let margin = 0.06;
    let span = ((hi.0 - lo.0) / (1. - 2. * margin) / w as f64).max((hi.1 - lo.1) / (1. - 2. * margin) / h as f64).max(1e-9);
    let center = ((lo.0 + hi.0) / 2., (lo.1 + hi.1) / 2.);
    let project = |p: DVec3| -> [f64; 3] {
        let (x, y) = project_2d(p);
        [w as f64 / 2. + (x - center.0) / span, h as f64 / 2. - (y - center.1) / span, p.dot(forward)]
    };
    let light = (-forward + up * 0.6 - right * 0.35).normalize();
    for (id, triangles) in &faces {
        let base = evaluated.face_colors.get(id).map_or(WOOD, |c| c.map(f64::from));
        for t in triangles {
            let mut normal = (t[1] - t[0]).cross(t[2] - t[0]);
            if normal.length() < 1e-18 {
                continue;
            }
            normal = normal.normalize();
            if normal.dot(forward) > 0. {
                normal = -normal;
            }
            let shade = 0.45 + 0.55 * normal.dot(light).max(0.);
            canvas.triangle([project(t[0]), project(t[1]), project(t[2])], base.map(|c| c * shade));
        }
    }
    let bias = (depth_range.1 - depth_range.0).max(1e-3) * 0.004 + span * 2.;
    for (a, b) in &edges {
        canvas.line(project(*a), project(*b), EDGE, SAMPLES as f64, Some(bias));
    }
    // Dimensions on top, with their value; overlapping labels are reported.
    let scale = SAMPLES as f64 * 2.;
    let mut labels: Vec<(u64, [f64; 4], [f64; 4])> = Vec::new();
    let mut end_on = Vec::new();
    for d in &dimensions {
        for (a, b) in &d.segments {
            canvas.line(project(*a), project(*b), DIMENSION, 1., None);
        }
        let (pa, pb) = (project(d.line.0), project(d.line.1));
        canvas.line(pa, pb, DIMENSION, SAMPLES as f64, None);
        let id = &d.id;
        if (pb[0] - pa[0]).hypot(pb[1] - pa[1]) < 6. * SAMPLES as f64 {
            end_on.push(*id);
            continue;
        }
        let text = label(scene::mm(d.length));
        let at = project(d.label);
        let mid = [at[0], at[1]];
        canvas.text(&text, mid, scale, DIMENSION);
        let half_w = ((GLYPH_W + 1) * text.len()) as f64 * scale / 2. + scale;
        let half_h = GLYPH_H as f64 * scale / 2. + scale;
        let rect = [mid[0] - half_w, mid[1] - half_h, mid[0] + half_w, mid[1] + half_h];
        labels.push((*id, rect, [pa[0], pa[1], pb[0], pb[1]]));
    }
    let mut notes = Vec::new();
    for (id, anchor, offset, text) in &texts {
        let (p, q) = (project(*anchor), project(*anchor + *offset));
        canvas.line(p, q, TEXT_MARK, 1., None);
        // One line per line of the note, centred on the label point.
        let lines: Vec<String> = text.lines().map(fold_text).filter(|l| !l.trim().is_empty()).collect();
        let step = (GLYPH_H as f64 + 3.) * scale;
        let first = q[1] - step * (lines.len().max(1) - 1) as f64 / 2.;
        for (n, line) in lines.iter().enumerate() {
            canvas.text(line, [q[0], first + step * n as f64], scale, TEXT_MARK);
        }
        notes.push(json!({"id": id, "text": text, "at_px": [(q[0] / SAMPLES as f64).round(), (q[1] / SAMPLES as f64).round()]}));
    }
    let overlaps_rect = |r: &[f64; 4], s: &[f64; 4]| r[0] < s[2] && s[0] < r[2] && r[1] < s[3] && s[1] < r[3];
    let line_hits_rect = |l: &[f64; 4], r: &[f64; 4]| {
        (0..=32).any(|k| {
            let t = k as f64 / 32.;
            let (x, y) = (l[0] + (l[2] - l[0]) * t, l[1] + (l[3] - l[1]) * t);
            x > r[0] && x < r[2] && y > r[1] && y < r[3]
        })
    };
    let mut overlaps = Vec::new();
    for (i, (id, rect, line)) in labels.iter().enumerate() {
        for (other, other_rect, other_line) in &labels[i + 1..] {
            if overlaps_rect(rect, other_rect) {
                overlaps.push(json!({"dimensions": [id, other], "problem": "labels overlap"}));
            } else if line_hits_rect(line, other_rect) || line_hits_rect(other_line, rect) {
                overlaps.push(json!({"dimensions": [id, other], "problem": "a dimension line crosses the other's label"}));
            }
        }
    }
    let rgb = canvas.downsample();
    let png = png(width, height, &rgb)?;
    let mut info = json!({
        "view": view, "width": width, "height": height,
        "mm_per_pixel": (span * SAMPLES as f64 * 10_000.).round() / 10.,
        "dimensions_shown": labels.len(),
        "dimension_overlaps": overlaps,
        "texts": notes,
    });
    if !end_on.is_empty() {
        info["dimensions_end_on"] = json!(end_on);
        info["note"] = json!("These dimensions point along the view direction and cannot be read in this view.");
    }
    if empty {
        info["note"] = json!("The model is empty.");
    }
    let hidden: BTreeSet<u64> = model.hidden.iter().copied().collect();
    if !hidden.is_empty() {
        info["hidden_entities"] = json!(hidden.len());
    }
    Ok(Rendered { png, info })
}

/// Standard base64, for embedding the image in an MCP result.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Call, ToolEnv, execute};

    fn shelf_with_dimensions() -> Model {
        let mut model = Model::default();
        let mut env = ToolEnv::default();
        for (name, args) in [
            ("create_board", json!({"name": "Side", "origin_mm": [0, 0, 0], "size_mm": [18, 300, 900]})),
            ("create_board", json!({"name": "Shelf", "origin_mm": [9, 0, 400], "size_mm": [500, 290, 18]})),
            ("add_dimension", json!({"start_mm": [0, 0, 0], "end_mm": [0, 0, 900], "offset_mm": [-100, 0, 0], "axis": "z"})),
            ("add_dimension", json!({"start_mm": [0, 0, 0], "end_mm": [0, 0, 900], "offset_mm": [-110, 0, 0], "axis": "z"})),
        ] {
            assert!(execute(&mut model, &Call::new("t", name, args), &mut env).ok);
        }
        model
    }

    #[test]
    fn renders_a_png_of_the_requested_size_for_every_view() {
        let model = shelf_with_dimensions();
        for view in VIEWS {
            let rendered = render(&model, view, 200, 150).unwrap();
            assert!(rendered.png.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(&rendered.png[16..24], &[0, 0, 0, 200, 0, 0, 0, 150]);
            assert_eq!(rendered.info["view"], *view);
        }
        assert!(render(&model, "sideways", 200, 150).is_err());
        assert!(render(&model, "front", 10, 150).is_err());
    }

    #[test]
    fn the_model_is_drawn_and_overlapping_dimensions_are_reported() {
        let model = shelf_with_dimensions();
        let rendered = render(&model, "front", 400, 400).unwrap();
        let overlaps = rendered.info["dimension_overlaps"].as_array().unwrap();
        assert_eq!(overlaps.len(), 1, "{}", rendered.info);
        // Something other than the background was drawn.
        let mut decoder = flate2::read::ZlibDecoder::new(&rendered.png[41..rendered.png.len() - 16]);
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut raw).unwrap();
        assert!(raw.chunks(3).any(|p| p.len() == 3 && p != BACKGROUND), "the image is blank");
        // Seen from the side, the vertical dimensions read; from the top they are end-on.
        let top = render(&model, "top", 200, 200).unwrap();
        assert_eq!(top.info["dimensions_end_on"].as_array().unwrap().len(), 2);
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
    }
}
