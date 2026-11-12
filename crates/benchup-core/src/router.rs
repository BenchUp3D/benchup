//! Non-destructive edge machining. The saved model retains the uncut stock.
use crate::{
    DVec2, DVec3, Model, ObjectKind,
    csg::{self, Boolean, Polygon},
    solids::{self, Frame},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Original editable path, retained separately from the machined solid mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeControl {
    pub points: [DVec3; 2],
    pub parent: Option<u64>,
    pub hidden: bool,
    pub locked: bool,
}

/// Derived presentation only: recomputed from saved routes, never serialized
/// into the authoritative stock or undo history.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Appearance {
    pub faces: BTreeSet<u64>,
    pub edges: BTreeSet<u64>,
    pub smooth_edges: BTreeSet<u64>,
    pub normals: BTreeMap<(u64, u64), DVec3>,
    /// Evaluated primitives resolve back to editable stock, in either direction.
    pub sources: BTreeMap<u64, u64>,
    pub by_source: BTreeMap<u64, BTreeSet<u64>>,
    pub edge_controls: BTreeMap<u64, EdgeControl>,
    /// Unrouted editable stock edges. Their original endpoints, rather than
    /// CSG subdivision vertices, supply selection handles on machined stock.
    pub stock_edge_controls: BTreeMap<u64, EdgeControl>,
}

impl Appearance {
    pub(crate) fn subset(&self, ids: &BTreeSet<u64>) -> Self {
        Self {
            faces: self.faces.intersection(ids).copied().collect(),
            edges: self.edges.intersection(ids).copied().collect(),
            smooth_edges: self.smooth_edges.intersection(ids).copied().collect(),
            sources: self
                .sources
                .iter()
                .filter(|(id, _)| ids.contains(id))
                .map(|(&id, &source)| (id, source))
                .collect(),
            by_source: self
                .by_source
                .iter()
                .filter_map(|(&source, derived)| {
                    let derived: BTreeSet<_> = derived.intersection(ids).copied().collect();
                    (!derived.is_empty()).then_some((source, derived))
                })
                .collect(),
            edge_controls: self
                .edge_controls
                .iter()
                .filter(|(source, _)| {
                    ids.contains(*source)
                        || self
                            .by_source
                            .get(*source)
                            .is_some_and(|derived| !derived.is_disjoint(ids))
                })
                .map(|(&id, control)| (id, control.clone()))
                .collect(),
            stock_edge_controls: self
                .stock_edge_controls
                .iter()
                .filter(|(source, _)| {
                    ids.contains(*source)
                        || self
                            .by_source
                            .get(*source)
                            .is_some_and(|derived| !derived.is_disjoint(ids))
                })
                .map(|(&id, control)| (id, control.clone()))
                .collect(),
            normals: self
                .normals
                .iter()
                .filter(|((face, _), _)| ids.contains(face))
                .map(|(&key, &normal)| (key, normal))
                .collect(),
        }
    }

    pub fn endpoint_controls(&self) -> impl Iterator<Item = (&u64, &EdgeControl)> {
        self.edge_controls.iter().chain(&self.stock_edge_controls)
    }

    pub fn endpoint_control(&self, id: u64) -> Option<&EdgeControl> {
        self.edge_controls
            .get(&id)
            .or_else(|| self.stock_edge_controls.get(&id))
    }

    pub fn render_entities(&self, source: &BTreeSet<u64>) -> BTreeSet<u64> {
        let mut result = source.clone();
        for id in source {
            if let Some(derived) = self.by_source.get(id) {
                result.extend(derived);
            }
        }
        result
    }

    /// A routed edge is selected through its original path, not its generated
    /// facets. Face/object selections still resolve onto the machined solid.
    pub fn selection_entities(&self, source: &BTreeSet<u64>) -> BTreeSet<u64> {
        let mut result = source.clone();
        for id in source
            .iter()
            .filter(|id| !self.edge_controls.contains_key(*id))
        {
            if let Some(derived) = self.by_source.get(id) {
                result.extend(derived);
            }
        }
        result
    }

    pub fn normal_at(&self, model: &Model, face: u64, point: DVec3, fallback: DVec3) -> DVec3 {
        if !self.faces.contains(&face) {
            return fallback;
        }
        let mut normal = DVec3::ZERO;
        for &vertex in model.faces[&face]
            .vertices
            .iter()
            .chain(model.faces[&face].holes.iter().flatten())
        {
            let Some(&value) = self.normals.get(&(face, vertex)) else {
                continue;
            };
            let distance = model.vertices[&vertex].position.distance_squared(point);
            if distance < 1e-18 {
                return value;
            }
            normal += value / distance;
        }
        normal.try_normalize().unwrap_or(fallback)
    }

    pub(crate) fn remove(&mut self, ids: &BTreeSet<u64>) {
        self.faces.retain(|id| !ids.contains(id));
        self.edges.retain(|id| !ids.contains(id));
        self.smooth_edges.retain(|id| !ids.contains(id));
        self.normals.retain(|(face, _), _| !ids.contains(face));
        self.sources.retain(|id, _| !ids.contains(id));
        self.by_source.retain(|_, derived| {
            derived.retain(|id| !ids.contains(id));
            !derived.is_empty()
        });
        self.edge_controls
            .retain(|source, _| self.by_source.contains_key(source));
        self.stock_edge_controls
            .retain(|source, _| self.by_source.contains_key(source));
    }

    pub(crate) fn extend(&mut self, other: &Self) {
        self.faces.extend(&other.faces);
        self.edges.extend(&other.edges);
        self.smooth_edges.extend(&other.smooth_edges);
        self.normals.extend(&other.normals);
        self.sources.extend(&other.sources);
        self.edge_controls.extend(
            other
                .edge_controls
                .iter()
                .map(|(&id, control)| (id, control.clone())),
        );
        self.stock_edge_controls.extend(
            other
                .stock_edge_controls
                .iter()
                .map(|(&id, control)| (id, control.clone())),
        );
        for (&source, derived) in &other.by_source {
            self.by_source.entry(source).or_default().extend(derived);
        }
    }
}

pub const CUT_COLOR: [u8; 3] = [218, 144, 72];
pub const EDGE_COLOR: [u8; 3] = [190, 112, 40];

fn finish_appearance(current: &Model, stock: &Model, applied: &[u64], contents: &mut Model) {
    let mut appearance = Appearance::default();
    let normals: BTreeMap<_, _> = contents
        .faces
        .values()
        .filter_map(|face| {
            crate::faces::plane(contents, &face.vertices).map(|plane| (face.id, plane.normal))
        })
        .collect();
    let route_at = |point: DVec3| {
        applied
            .iter()
            .copied()
            .min_by(|a, b| {
                let distance = |id| {
                    let (a, b) = stock.points(&stock.edges[&id]);
                    let t = ((point - a).dot(b - a) / a.distance_squared(b)).clamp(0., 1.);
                    point.distance_squared(a + (b - a) * t)
                };
                distance(*a).total_cmp(&distance(*b))
            })
            .unwrap()
    };
    let mut incident = BTreeMap::<u64, Vec<u64>>::new();
    let mut boundaries = BTreeMap::<(u64, u64), Vec<u64>>::new();
    for face in contents.faces.values() {
        let triangle = contents.face_triangles(face)[0];
        let center = triangle.into_iter().sum::<DVec3>() / 3.;
        let source = stock
            .faces
            .values()
            .find(|source| stock.face_contains(source, center, true))
            .map_or_else(|| route_at(center), |face| face.id);
        appearance.sources.insert(face.id, source);
        if !current
            .faces
            .values()
            .any(|source| current.face_contains(source, center, true))
        {
            appearance.faces.insert(face.id);
            contents.face_colors.insert(face.id, CUT_COLOR);
        }
        for ring in std::iter::once(&face.vertices).chain(&face.holes) {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                incident.entry(a).or_default().push(face.id);
                boundaries
                    .entry((a.min(b), a.max(b)))
                    .or_default()
                    .push(face.id);
            }
        }
    }
    // Hide only shallow seams around a routed surface; keep sharp shoulders,
    // rebates, grooves and the stock's unrelated creases intact.
    let threshold = 20_f64.to_radians().cos();
    for edge in contents.edges.values() {
        let Some(neighbors) = boundaries.get(&(edge.a.min(edge.b), edge.a.max(edge.b))) else {
            continue;
        };
        let (a, b) = contents.points(edge);
        let source = stock
            .edges
            .values()
            .find(|source| {
                let (start, end) = stock.points(source);
                [a, b].into_iter().all(|point| {
                    let t = (point - start).dot(end - start) / start.distance_squared(end);
                    (-1e-7..=1. + 1e-7).contains(&t)
                        && point.distance(start + (end - start) * t) < 1e-7
                })
            })
            .map(|edge| edge.id)
            .unwrap_or_else(|| {
                neighbors
                    .iter()
                    .filter_map(|id| appearance.sources.get(id))
                    .find(|id| applied.contains(id))
                    .copied()
                    .unwrap_or_else(|| route_at((a + b) * 0.5))
            });
        appearance.sources.insert(edge.id, source);
        if !neighbors.iter().any(|id| appearance.faces.contains(id)) {
            continue;
        }
        appearance.edges.insert(edge.id);
        if let [a, b] = neighbors.as_slice()
            && normals[a].dot(normals[b]) > threshold
        {
            appearance.smooth_edges.insert(edge.id);
        }
    }
    for &face in &appearance.faces {
        for &vertex in contents.faces[&face]
            .vertices
            .iter()
            .chain(contents.faces[&face].holes.iter().flatten())
        {
            let normal = incident[&vertex]
                .iter()
                .filter(|id| normals[&face].dot(normals[id]) > threshold)
                .map(|id| normals[id])
                .sum::<DVec3>()
                .normalize_or_zero();
            appearance.normals.insert((face, vertex), normal);
        }
    }
    for (&derived, &source) in &appearance.sources {
        appearance
            .by_source
            .entry(source)
            .or_default()
            .insert(derived);
    }
    for &id in applied {
        let (a, b) = stock.points(&stock.edges[&id]);
        appearance.edge_controls.insert(
            id,
            EdgeControl {
                points: [a, b],
                parent: None,
                hidden: stock.is_hidden(id),
                locked: stock.is_locked(id),
            },
        );
    }
    for edge in stock
        .edges
        .values()
        .filter(|edge| !applied.contains(&edge.id))
    {
        let (a, b) = stock.points(edge);
        appearance.stock_edge_controls.insert(
            edge.id,
            EdgeControl {
                points: [a, b],
                parent: None,
                hidden: stock.is_hidden(edge.id),
                locked: stock.is_locked(edge.id),
            },
        );
    }
    contents.routing_appearance = appearance;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    #[default]
    Chamfer,
    Bevel,
    Roundover,
    Cove,
    Rabbet,
    Groove,
    Ogee,
    Bead,
    None,
}

impl Profile {
    pub const ALL: [Self; 9] = [
        Self::None,
        Self::Chamfer,
        Self::Bevel,
        Self::Roundover,
        Self::Cove,
        Self::Rabbet,
        Self::Groove,
        Self::Ogee,
        Self::Bead,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => tr!("router.none"),
            Self::Chamfer => tr!("router.chamfer"),
            Self::Bevel => tr!("router.bevel"),
            Self::Roundover => tr!("router.roundover"),
            Self::Cove => tr!("router.cove"),
            Self::Rabbet => tr!("router.rabbet"),
            Self::Groove => tr!("router.groove"),
            Self::Ogee => tr!("router.ogee"),
            Self::Bead => tr!("router.bead"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub profile: Profile,
    /// In meters. Also the radius for circular profiles.
    pub width: f64,
    pub depth: f64,
    /// Groove setback or bead shoulder, in meters.
    pub offset: f64,
    pub angle_degrees: f64,
    pub reverse: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            profile: Profile::Chamfer,
            width: 0.005,
            depth: 0.003,
            offset: 0.002,
            angle_degrees: 30.,
            reverse: false,
        }
    }
}

impl Settings {
    pub const SIZE: std::ops::RangeInclusive<f64> = 0.0001..=100.;
    pub const ANGLE: std::ops::RangeInclusive<f64> = 1. ..=89.;

    pub fn valid(self) -> bool {
        [self.width, self.depth, self.offset]
            .into_iter()
            .all(|v| Self::SIZE.contains(&v))
            && Self::ANGLE.contains(&self.angle_degrees)
    }

    /// Cutting boundary from the second face to the first face in an edge cross section.
    fn boundary(self, opening: f64) -> Result<Vec<DVec2>, &'static str> {
        use Profile::*;
        let w = self.width;
        let d = self.depth;
        let s = self.offset;
        let p = DVec2::new;
        let quarter = |radius: f64, origin: DVec2, convex: bool| {
            (0..=24).map(move |i| {
                let angle = std::f64::consts::FRAC_PI_2 * f64::from(i) / 24.;
                origin
                    + radius
                        * if convex {
                            p(1. - angle.cos(), 1. - angle.sin())
                        } else {
                            p(angle.sin(), angle.cos())
                        }
            })
        };
        if !matches!(self.profile, Chamfer | Bevel) && opening.cos().abs() > 1e-5 {
            return Err(tr!("router.square_edge_required"));
        }
        Ok(match self.profile {
            None => return Err(tr!("router.invalid_settings")),
            Chamfer => vec![p(0., w), p(w, 0.)],
            Bevel => {
                let angle = self.angle_degrees.to_radians();
                if opening + angle >= std::f64::consts::PI - 0.01 {
                    return Err(tr!("router.angle_does_not_fit"));
                }
                let depth = w * angle.sin() / (opening + angle).sin();
                vec![p(0., depth), p(w, 0.)]
            }
            Rabbet => vec![p(0., d), p(w, d), p(w, 0.)],
            Groove => vec![p(s, d), p(s + w, d), p(s + w, 0.)],
            Roundover => quarter(w, DVec2::ZERO, true).collect(),
            Cove => quarter(w, DVec2::ZERO, false).collect(),
            Ogee => quarter(w * 0.5, p(0., w * 0.5), true)
                .chain(quarter(w * 0.5, p(w * 0.5, 0.), false).skip(1))
                .collect(),
            Bead => std::iter::once(p(0., w + s))
                .chain(quarter(w, p(s, s), true))
                .chain(std::iter::once(p(w + s, 0.)))
                .collect(),
        })
    }
}

/// Match a point on an original face against the oriented solid polygons.
fn on_polygon(polygon: &Polygon, point: DVec3, epsilon: f64) -> bool {
    (polygon.normal.dot(point) - polygon.w).abs() <= epsilon
        && polygon
            .vertices
            .iter()
            .zip(polygon.vertices.iter().cycle().skip(1))
            .all(|(&a, &b)| {
                (b - a).cross(point - a).dot(polygon.normal) >= -epsilon * a.distance(b)
            })
}

/// A complete segment must lie on a coplanar union of stock polygons. Interval
/// clipping handles triangulation seams, concave outlines and face holes.
fn surface_support<'a>(
    surfaces: &'a [Polygon],
    a: DVec3,
    b: DVec3,
    eps: f64,
) -> Option<&'a Polygon> {
    surfaces.iter().find(|surface| {
        if [a, b]
            .into_iter()
            .any(|p| (surface.normal.dot(p) - surface.w).abs() > eps)
        {
            return false;
        }
        let mut intervals = Vec::new();
        for polygon in surfaces
            .iter()
            .filter(|p| p.normal.dot(surface.normal) > 1. - 1e-7 && (p.w - surface.w).abs() < eps)
        {
            let (mut lo, mut hi) = (0_f64, 1_f64);
            for (&p, &q) in polygon
                .vertices
                .iter()
                .zip(polygon.vertices.iter().cycle().skip(1))
            {
                let inward = polygon.normal.cross(q - p).normalize();
                let start = (a - p).dot(inward);
                let delta = (b - a).dot(inward);
                if delta.abs() < 1e-14 {
                    if start < -eps {
                        hi = -1.;
                        break;
                    }
                } else {
                    let t = (-eps - start) / delta;
                    if delta > 0. {
                        lo = lo.max(t);
                    } else {
                        hi = hi.min(t);
                    }
                }
            }
            if lo <= hi {
                intervals.push((lo, hi));
            }
        }
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut covered = 0_f64;
        for (lo, hi) in intervals {
            if lo > covered + 1e-9 {
                return false;
            }
            covered = covered.max(hi);
        }
        covered >= 1. - 1e-9
    })
}

fn surface_cutter(
    surfaces: &[Polygon],
    frame: Frame,
    a: DVec3,
    b: DVec3,
    settings: Settings,
) -> Result<Vec<Polygon>, &'static str> {
    let eps = frame.epsilon;
    let pad = eps * 32.;
    let surface =
        surface_support(surfaces, a, b, eps * 4.).ok_or(tr!("router.surface_path_required"))?;
    let tangent = (b - a).normalize();
    let across = surface.normal.cross(tangent);
    let inward = -surface.normal;
    let scale = frame.local(DVec3::X).distance(frame.local(DVec3::ZERO));
    let half_width = settings.width * 0.5;
    let depth = if settings.profile == Profile::Bevel {
        half_width * settings.angle_degrees.to_radians().tan()
    } else {
        settings.depth
    };
    for side in [-1., 1.] {
        let offset = across * (half_width * scale * side);
        if surface_support(surfaces, a + offset, b + offset, eps * 4.).is_none() {
            return Err(tr!("router.profile_does_not_fit"));
        }
    }
    let half = if matches!(settings.profile, Profile::Groove | Profile::Rabbet) {
        vec![
            DVec2::new(0., depth),
            DVec2::new(half_width, depth),
            DVec2::new(half_width, 0.),
        ]
    } else {
        let boundary = settings.boundary(std::f64::consts::FRAC_PI_2)?;
        let width = boundary.last().unwrap().x;
        let height = boundary[0].y;
        boundary
            .into_iter()
            .map(|p| DVec2::new(p.x * half_width / width, p.y * depth / height))
            .collect()
    };
    let mut bottom: Vec<_> = half.iter().rev().map(|p| DVec2::new(-p.x, p.y)).collect();
    bottom.extend(half.into_iter().skip(1));
    let bottom: Vec<_> = bottom.into_iter().map(|p| p * scale).collect();
    let mut ring = vec![
        DVec2::new(-half_width * scale, -pad),
        DVec2::new(half_width * scale, -pad),
    ];
    ring.extend(bottom.iter().rev());
    let (a, b) = (a - tangent * pad, b + tangent * pad);
    let at = |origin: DVec3, p: DVec2| origin + across * p.x + inward * p.y;
    let flip = across.cross(inward).dot(tangent) < 0.;
    let mut result = Vec::new();
    let mut add = |mut ring: Vec<DVec3>| {
        if flip {
            ring.reverse();
        }
        result.push(Polygon::new(ring, surface.color, eps).ok_or(tr!("router.profile_too_small"))?);
        Ok::<_, &'static str>(())
    };
    // Vertical strips cap even concave bit profiles without fan triangles
    // crossing outside the cutter. Zero-width shoulder spans need only walls.
    for pair in bottom.windows(2).filter(|p| p[1].x - p[0].x > eps) {
        let cap = [
            DVec2::new(pair[0].x, -pad),
            DVec2::new(pair[1].x, -pad),
            pair[1],
            pair[0],
        ];
        add(cap.iter().rev().map(|p| at(a, *p)).collect())?;
        add(cap.iter().map(|p| at(b, *p)).collect())?;
    }
    for (&p, &q) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        if p.distance(q) <= eps {
            continue;
        }
        add(vec![at(a, p), at(a, q), at(b, q), at(b, p)])?;
    }
    Ok(result)
}

fn cutter(
    stock: &Model,
    surfaces: &[Polygon],
    frame: Frame,
    edge: u64,
    settings: Settings,
) -> Result<Vec<Polygon>, &'static str> {
    let edge = &stock.edges[&edge];
    let (a, b) = stock.points(edge);
    let (a, b) = (frame.local(a), frame.local(b));
    let mid = (a + b) * 0.5;
    let tangent = (b - a).normalize();
    let eps = frame.epsilon;
    let pad = eps * 32.;
    let boundary_error = tr!("router.convex_edge_required");
    let adjacent: Vec<_> = stock
        .faces
        .values()
        .filter(|face| {
            std::iter::once(&face.vertices)
                .chain(&face.holes)
                .any(|ring| {
                    ring.iter()
                        .zip(ring.iter().cycle().skip(1))
                        .any(|(&a, &b)| {
                            (a == edge.a && b == edge.b) || (b == edge.a && a == edge.b)
                        })
                })
        })
        .collect();
    let surface_path = match adjacent.as_slice() {
        [first, second] => {
            let a = crate::faces::plane(stock, &first.vertices).ok_or(boundary_error)?;
            let b = crate::faces::plane(stock, &second.vertices).ok_or(boundary_error)?;
            a.normal.dot(b.normal).abs() > 1. - 1e-7
        }
        _ => true,
    };
    if surface_path {
        return surface_cutter(surfaces, frame, a, b, settings);
    }
    let [first, second] = adjacent.as_slice() else {
        return Err(boundary_error);
    };
    let face_normal = |face: &crate::Face| {
        let normal = crate::faces::plane(stock, &face.vertices)?.normal;
        surfaces.iter().find(|poly| {
            poly.normal.dot(normal).abs() > 1. - 1e-7 && on_polygon(poly, mid, eps * 4.)
        })
    };
    let first = face_normal(first).ok_or(boundary_error)?;
    let second = face_normal(second).ok_or(boundary_error)?;
    let (first, second) = if settings.reverse {
        (second, first)
    } else {
        (first, second)
    };
    let inward = |poly: &Polygon, other: &Polygon| {
        let center = poly.vertices.iter().copied().sum::<DVec3>() / poly.vertices.len() as f64;
        let side = poly.normal.cross(tangent);
        side * (center - mid).dot(side).signum()
            * if other.normal.dot(center - mid) < -eps {
                1.
            } else {
                0.
            }
    };
    let u = inward(first, second);
    let v = inward(second, first);
    if u.cross(v).length() < 1e-4 {
        return Err(boundary_error);
    }
    let opening = u.dot(v).clamp(-1., 1.).acos();
    let boundary = settings.boundary(opening)?;
    let scale = frame.local(DVec3::X).distance(frame.local(DVec3::ZERO));
    let start = boundary[0];
    let end = *boundary.last().unwrap();
    let fits_face = |origin: &Polygon, direction: DVec3, distance: f64| {
        surfaces.iter().any(|poly| {
            poly.normal.dot(origin.normal) > 1. - 1e-7
                && on_polygon(poly, mid + direction * (distance * scale + pad), eps)
        })
    };
    if !fits_face(first, u, end.x) || !fits_face(second, v, start.y) {
        return Err(tr!("router.profile_does_not_fit"));
    }
    let mut ring = Vec::with_capacity(boundary.len() + 3);
    let left = if settings.profile == Profile::Groove {
        start.x * scale
    } else {
        -pad
    };
    ring.push(DVec2::new(left, -pad));
    ring.push(DVec2::new(end.x * scale, -pad));
    ring.extend(boundary.iter().rev().map(|p| *p * scale));
    if settings.profile != Profile::Groove {
        ring.push(DVec2::new(left, start.y * scale));
    }
    // Profile coordinates are monotone, so both caps can be triangulated
    // from the lower left. Extend just beyond the source ends to avoid
    // coplanar cutter caps on square stock.
    let (a, b) = (a - tangent * pad, b + tangent * pad);
    let near: Vec<_> = ring.iter().map(|p| a + u * p.x + v * p.y).collect();
    let far: Vec<_> = ring.iter().map(|p| b + u * p.x + v * p.y).collect();
    let flip = u.cross(v).dot(tangent) < 0.;
    let mut result = Vec::new();
    let mut add = |mut ring: Vec<_>| {
        if flip {
            ring.reverse();
        }
        result.push(Polygon::new(ring, first.color, eps).ok_or(tr!("router.profile_too_small"))?);
        Ok::<_, &'static str>(())
    };
    for i in 1..near.len() - 1 {
        add(vec![near[0], near[i + 1], near[i]])?;
        add(vec![far[0], far[i], far[i + 1]])?;
    }
    for i in 0..near.len() {
        let j = (i + 1) % near.len();
        add(vec![near[i], near[j], far[j], far[i]])?;
    }
    Ok(result)
}

impl Model {
    /// Validate the saved definitions independently of the rest of the model.
    pub fn validate_routes(&self) -> Result<(), &'static str> {
        if self.routes.len() > 10_000 {
            return Err(tr!("core.document.model_is_too_large"));
        }
        if self
            .routes
            .iter()
            .any(|(edge, settings)| !self.edges.contains_key(edge) || !settings.valid())
        {
            return Err(tr!("router.invalid_settings"));
        }
        Ok(())
    }

    pub fn has_machining(&self) -> bool {
        !self.routes.is_empty()
            || self.connections.values().any(|connection| {
                matches!(
                    connection.operation,
                    crate::connections::ConnectionOperation::Joinery(_)
                )
            })
    }

    /// Whether this is a drawn surface path rather than a two-plane boundary.
    pub fn route_on_surface(&self, edge: u64) -> bool {
        let Some(edge) = self.edges.get(&edge) else {
            return false;
        };
        let normals: Vec<_> = self
            .faces
            .values()
            .filter(|face| {
                std::iter::once(&face.vertices)
                    .chain(&face.holes)
                    .any(|ring| {
                        ring.iter()
                            .zip(ring.iter().cycle().skip(1))
                            .any(|(&a, &b)| {
                                (a == edge.a && b == edge.b) || (a == edge.b && b == edge.a)
                            })
                    })
            })
            .filter_map(|face| crate::faces::plane(self, &face.vertices).map(|p| p.normal))
            .collect();
        !matches!(normals.as_slice(), [a, b] if a.dot(*b).abs() < 1. - 1e-7)
    }

    /// Resolve detached lines drawn on a face to the same loose stock body.
    /// Parent boundaries remain authoritative: a line in another group cannot
    /// silently machine a neighboring object's geometry.
    pub fn route_entities(&self, edge: u64) -> Result<BTreeSet<u64>, &'static str> {
        let mut ids = self.solid_entities(edge)?;
        if ids.iter().any(|id| self.objects.contains_key(id)) {
            return Ok(ids);
        }
        if !ids.iter().any(|id| self.faces.contains_key(id)) {
            let line = self
                .edges
                .get(&edge)
                .ok_or(tr!("router.surface_path_required"))?;
            let (a, b) = self.points(line);
            let face = self
                .faces
                .values()
                .find(|face| {
                    self.parent(face.id) == self.edit_context
                        && [a, (a + b) * 0.5, b]
                            .into_iter()
                            .all(|p| self.face_contains(face, p, true))
                })
                .ok_or(tr!("router.surface_path_required"))?;
            ids.extend(self.solid_entities(face.id)?);
        }
        let faces: Vec<_> = ids.iter().filter_map(|id| self.faces.get(id)).collect();
        let children: BTreeSet<_> = self
            .objects
            .values()
            .flat_map(|o| o.entities.iter().copied())
            .collect();
        for line in self.edges.values().filter(|line| match self.edit_context {
            Some(scope) => self.objects[&scope].entities.contains(&line.id),
            None => !children.contains(&line.id),
        }) {
            if ids.contains(&line.id) {
                continue;
            }
            let (a, b) = self.points(line);
            if faces.iter().any(|face| {
                [a, (a + b) * 0.5, b]
                    .into_iter()
                    .all(|p| self.face_contains(face, p, true))
            }) {
                ids.insert(line.id);
            }
        }
        Ok(ids)
    }

    /// Owning piece for an edge that can be edited in Router.
    pub fn route_board(&self, edge: u64) -> Option<u64> {
        self.edges
            .contains_key(&edge)
            .then(|| self.parent(edge))
            .flatten()
    }
}

pub(crate) fn evaluate(source: &Model, model: &mut Model, errors: &mut Vec<(u64, String)>) {
    let mut boards = BTreeMap::<_, Vec<_>>::new();
    let mut loose = BTreeMap::new();
    for (&edge, &settings) in &source.routes {
        if settings.profile == Profile::None {
            continue;
        }
        if let Some(board) = source.route_board(edge) {
            boards.entry(board).or_default().push((edge, settings));
        } else if source.edges.contains_key(&edge) {
            loose.insert(edge, settings);
        } else {
            errors.push((edge, tr!("router.solid_piece_required").into()));
        }
    }
    model.routes.clear();
    for (board, routes) in boards {
        let mut applied = Vec::new();
        let result = cut_board(source, model, board, &routes, &mut applied, errors);
        if let Err(error) = result {
            if applied.is_empty() {
                applied.extend(routes.iter().map(|(edge, _)| *edge));
            }
            errors.extend(applied.into_iter().map(|edge| (edge, error.into())));
        }
    }
    // Resolve only bodies touched by a route. Unrelated loose lines, open
    // surfaces and solids must not prevent machining a closed body.
    while let Some((&edge, _)) = loose.first_key_value() {
        let ids = match model.route_entities(edge) {
            Ok(ids) => ids,
            Err(error) => {
                loose.remove(&edge);
                errors.push((edge, error.into()));
                continue;
            }
        };
        let routes: Vec<_> = loose
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .map(|(&id, &settings)| (id, settings))
            .collect();
        for (id, _) in &routes {
            loose.remove(id);
        }
        let stock = source.copy_entities(&ids);
        let mut applied = Vec::new();
        match cut_stock(model.next_id, &stock, &stock, &routes, &mut applied, errors) {
            Ok(Some((mut container, mut contents))) => {
                inherit_loose_state(&stock, &mut contents, edge);
                // A temporary derived container reuses the shared replacement
                // path (including vertex/glue cleanup), then disappears. Saved
                // stock and evaluated loose geometry remain ungrouped.
                let id = container.id;
                container.entities = ids;
                model.objects.insert(id, container);
                if let Err(error) = model.replace_contents(id, &contents) {
                    errors.extend(applied.into_iter().map(|edge| (edge, error.into())));
                }
                model.objects.remove(&id);
            }
            Ok(None) => {}
            Err(error) => {
                if applied.is_empty() {
                    applied.extend(routes.iter().map(|(edge, _)| *edge));
                }
                errors.extend(applied.into_iter().map(|edge| (edge, error.into())));
            }
        }
    }
}

/// Loose results have no parent from which to inherit visibility or tags.
/// Retained surfaces/edges keep their own state; new cut surfaces use the
/// clicked edge as their reference, without changing any saved assignments.
fn inherit_loose_state(stock: &Model, contents: &mut Model, reference: u64) {
    if stock.entity_tags.is_empty() && stock.hidden.is_empty() && stock.locked.is_empty() {
        return;
    }
    let inherit = |contents: &mut Model, id: u64, original: u64| {
        if let Some(&tag) = stock.entity_tags.get(&original) {
            contents.entity_tags.insert(id, tag);
        }
        if stock.hidden.contains(&original) {
            contents.hidden.insert(id);
        }
        if stock.locked.contains(&original) {
            contents.locked.insert(id);
        }
    };
    let faces: Vec<_> = contents
        .faces
        .values()
        .map(|face| {
            let triangle = contents.face_triangles(face)[0];
            let point = triangle.into_iter().sum::<DVec3>() / 3.;
            let original = stock
                .faces
                .values()
                .find(|face| stock.face_contains(face, point, true))
                .map_or(reference, |face| face.id);
            (face.id, original)
        })
        .collect();
    for (id, original) in faces {
        inherit(contents, id, original);
    }
    let edges: Vec<_> = contents
        .edges
        .values()
        .map(|edge| {
            let (a, b) = contents.points(edge);
            let original = stock
                .edges
                .values()
                .find(|source| {
                    let (start, end) = stock.points(source);
                    [a, b].into_iter().all(|point| {
                        let t = (point - start).dot(end - start) / start.distance_squared(end);
                        (-1e-7..=1. + 1e-7).contains(&t)
                            && (start + (end - start) * t).distance(point) < 1e-7
                    })
                })
                .map_or(reference, |edge| edge.id);
            (edge.id, original)
        })
        .collect();
    for (id, original) in edges {
        inherit(contents, id, original);
    }
}

fn cut_board(
    source: &Model,
    model: &mut Model,
    board: u64,
    routes: &[(u64, Settings)],
    applied: &mut Vec<u64>,
    errors: &mut Vec<(u64, String)>,
) -> Result<(), &'static str> {
    let piece = &source.objects[&board];
    if piece
        .entities
        .iter()
        .any(|id| !source.edges.contains_key(id) && !source.faces.contains_key(id))
    {
        return Err(tr!("router.solid_piece_required"));
    }
    let stock = source
        .object_contents(board)
        .ok_or(tr!("router.solid_piece_required"))?;
    let current = if source.objects[&board].entities == model.objects[&board].entities {
        None
    } else {
        Some(
            model
                .object_contents(board)
                .ok_or(tr!("router.solid_piece_required"))?,
        )
    };
    let Some((_, mut contents)) = cut_stock(
        model.next_id,
        &stock,
        current.as_ref().unwrap_or(&stock),
        routes,
        applied,
        errors,
    )?
    else {
        return Ok(());
    };
    for control in contents.routing_appearance.edge_controls.values_mut()
        .chain(contents.routing_appearance.stock_edge_controls.values_mut())
    {
        control.parent = Some(board);
    }
    model.replace_contents(board, &contents)?;
    // Preserve instance IDs and metadata. Derived component definitions are
    // independent so an instance can have different routing from its siblings.
    let mut current = Some(board);
    while let Some(id) = current {
        if matches!(model.objects[&id].kind, ObjectKind::Component { .. }) {
            let geometry = model
                .expand_entities(&model.objects[&id].entities)
                .into_iter()
                .filter(|id| !model.objects.contains_key(id));
            let mapping: BTreeMap<_, _> = geometry.map(|id| (id, id)).collect();
            let object = model.objects.get_mut(&id).unwrap();
            object.kind = ObjectKind::Component {
                definition: uuid::Uuid::new_v4(),
            };
            object.definition_entities = mapping;
        }
        current = model.parent(id);
    }
    for connection in model
        .connections
        .values_mut()
        .filter(|c| c.members.contains(&board))
    {
        connection.entry_face = None;
    }
    Ok(())
}

/// Router accepts drawing paths in addition to a closed stock shell. Feed
/// only face boundaries to the strict solid importer; keep the full model for
/// path lookup, picking provenance and preserving uncut drawing edges.
fn routing_surfaces(model: &Model, frame: Frame) -> Result<Vec<Polygon>, &'static str> {
    let faces = model.faces.keys().copied().collect();
    let shell = model.copy_entities(&faces);
    solids::import(&shell, frame)
}

/// Machine original stock while retaining any incoming joinery cuts.
fn cut_stock(
    next_id: u64,
    stock: &Model,
    current: &Model,
    routes: &[(u64, Settings)],
    applied: &mut Vec<u64>,
    errors: &mut Vec<(u64, String)>,
) -> Result<Option<(crate::Object, Model)>, &'static str> {
    let frame = Frame::new(&[stock])?;
    let surfaces = routing_surfaces(stock, frame)?;
    let mut output = if std::ptr::eq(stock, current) {
        surfaces.clone()
    } else {
        routing_surfaces(current, frame)?
    };
    for &(edge, settings) in routes {
        let cut = (|| {
            if !settings.valid() {
                return Err(tr!("router.invalid_settings"));
            }
            let tool = cutter(stock, &surfaces, frame, edge, settings)?;
            let next = csg::apply(output.clone(), tool, Boolean::Difference, frame.epsilon)?;
            let volume = csg::volume(&next);
            let before = csg::volume(&output);
            if volume <= frame.epsilon.powi(3) * 32. || before - volume <= before * 1e-10 {
                return Err(tr!("router.profile_does_not_fit"));
            }
            Ok(next)
        })();
        match cut {
            Ok(next) => {
                output = next;
                applied.push(edge);
            }
            Err(error) => errors.push((edge, error.into())),
        }
    }
    if applied.is_empty() {
        return Ok(None);
    }

    let mut mesh = Model {
        next_id,
        ..Default::default()
    };
    let shell = solids::output(&mut mesh, output, frame, "", false, true)?
        .ok_or(tr!("router.profile_does_not_fit"))?;
    mesh.solid_volume(shell)?;
    let mut contents = mesh
        .object_contents(shell)
        .ok_or(tr!("router.profile_does_not_fit"))?;
    // Clipping the curved cutter also subdivides straight, unrelated stock
    // boundaries. Clean the evaluated mesh before recording its provenance;
    // these generated vertices never enter the saved drawing selection.
    contents.clean_collinear_vertices(&contents.root_entities())?;
    finish_appearance(current, stock, applied, &mut contents);
    // Keep other drawn paths visible/selectable when they still lie on the
    // machined surface. They are not solid boundaries and CSG omits them.
    let finished = routing_surfaces(&contents, frame)?;
    let mut first_normal = BTreeMap::<(u64, u64), DVec3>::new();
    let mut hard = BTreeSet::new();
    for face in stock.faces.values() {
        let normal = crate::faces::plane(stock, &face.vertices).unwrap().normal;
        for ring in std::iter::once(&face.vertices).chain(&face.holes) {
            for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let pair = (a.min(b), a.max(b));
                if let Some(previous) = first_normal.insert(pair, normal)
                    && previous.dot(normal).abs() < 1. - 1e-7
                {
                    hard.insert(pair);
                }
            }
        }
    }
    for edge in stock.edges.values().filter(|edge| {
        !applied.contains(&edge.id) && !hard.contains(&(edge.a.min(edge.b), edge.a.max(edge.b)))
    }) {
        let (a, b) = stock.points(edge);
        if surface_support(
            &finished,
            frame.local(a),
            frame.local(b),
            frame.epsilon * 4.,
        )
        .is_some()
        {
            contents
                .vertices
                .insert(edge.a, stock.vertices[&edge.a].clone());
            contents
                .vertices
                .insert(edge.b, stock.vertices[&edge.b].clone());
            contents.edges.insert(edge.id, edge.clone());
            contents.routing_appearance.sources.insert(edge.id, edge.id);
            contents
                .routing_appearance
                .by_source
                .entry(edge.id)
                .or_default()
                .insert(edge.id);
        }
    }

    if !stock.wood_glue.is_empty() {
        let result_faces: Vec<_> = contents.faces.keys().copied().collect();
        contents.transfer_glue_from(stock, &stock.faces.keys().copied().collect(), result_faces);
    }
    Ok(Some((mesh.objects[&shell].clone(), contents)))
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
