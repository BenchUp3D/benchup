//! Wood glue is instance-specific surface coverage, independent of face color.
use crate::{DVec2, DVec3, Face, Model, faces};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// Numerical tolerance for contact in world metres. Do not use the much
// larger snapping or joinery proximity tolerances to bridge an air gap.
const CONTACT: f64 = 1e-8;
const AREA: f64 = 1e-14;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Coverage {
    Whole,
    Partial {
        anchors: [u64; 3],
        polygons: Vec<Vec<DVec2>>,
    },
}

impl Coverage {
    pub(crate) fn remap(&self, mut id: impl FnMut(u64) -> Option<u64>) -> Option<Self> {
        Some(match self {
            Self::Whole => Self::Whole,
            Self::Partial {
                anchors: [a, b, c],
                polygons,
            } => Self::Partial {
                anchors: [id(*a)?, id(*b)?, id(*c)?],
                polygons: polygons.clone(),
            },
        })
    }

    pub(crate) fn valid(&self, face: &Face) -> bool {
        match self {
            Self::Whole => true,
            Self::Partial { anchors, polygons } => {
                anchors[0] != anchors[1]
                    && anchors[0] != anchors[2]
                    && anchors[1] != anchors[2]
                    && anchors.iter().all(|id| face.vertices.contains(id))
                    && !polygons.is_empty()
                    && polygons
                        .iter()
                        .all(|p| p.len() >= 3 && p.iter().all(|point| point.is_finite()))
            }
        }
    }

    pub(crate) fn polygons(&self, model: &Model, face: &Face) -> Vec<Vec<DVec3>> {
        match self {
            Self::Whole => model
                .face_triangles(face)
                .into_iter()
                .map(Vec::from)
                .collect(),
            Self::Partial { anchors, polygons } => {
                let Some([o, u, v]) = anchor_points(model, *anchors) else {
                    return vec![];
                };
                polygons
                    .iter()
                    .map(|p| {
                        p.iter()
                            .map(|p| o + (u - o) * p.x + (v - o) * p.y)
                            .collect()
                    })
                    .collect()
            }
        }
    }
}

fn anchor_points(model: &Model, [a, b, c]: [u64; 3]) -> Option<[DVec3; 3]> {
    Some([
        model.vertices.get(&a)?.position,
        model.vertices.get(&b)?.position,
        model.vertices.get(&c)?.position,
    ])
}

fn anchor_ids(model: &Model, face: &Face) -> Option<[u64; 3]> {
    let (&a, rest) = face.vertices.split_first()?;
    let o = model.vertices[&a].position;
    let &b = rest.iter().max_by(|a, b| {
        let distance = |id: &&u64| model.vertices[id].position.distance_squared(o);
        distance(a).total_cmp(&distance(b))
    })?;
    let u = model.vertices[&b].position - o;
    let altitude = |id: &&u64| u.cross(model.vertices[id].position - o).length_squared();
    let &c = rest
        .iter()
        .max_by(|a, b| altitude(a).total_cmp(&altitude(b)))?;
    (altitude(&&c) > 0.).then_some([a, b, c])
}

fn signed_area(p: &[DVec2]) -> f64 {
    let Some(&origin) = p.first() else { return 0. };
    p[1..]
        .windows(2)
        .map(|w| (w[0] - origin).perp_dot(w[1] - origin))
        .sum::<f64>()
        * 0.5
}

fn ccw(mut p: Vec<DVec2>) -> Vec<DVec2> {
    if signed_area(&p) < 0. {
        p.reverse();
    }
    p
}

fn half_plane(p: &[DVec2], a: DVec2, b: DVec2, inside: bool) -> Vec<DVec2> {
    let direction = b - a;
    if direction.length_squared() < 1e-28 {
        return if inside { p.to_vec() } else { vec![] };
    }
    let distance = |p| {
        let d = direction.perp_dot(p - a);
        if inside { d } else { -d }
    };
    let mut out = Vec::new();
    for (&from, &to) in p.iter().zip(p.iter().cycle().skip(1)) {
        let (d, e) = (distance(from), distance(to));
        if d.is_sign_positive() != e.is_sign_positive() && d != 0. && e != 0. {
            out.push(from.lerp(to, d / (d - e)));
        }
        if e >= 0. {
            out.push(to);
        }
    }
    out
}

// Split a convex polygon by another convex polygon. Each exterior piece is
// emitted at the first failed edge, so the pieces never overlap in area.
fn cut(mut p: Vec<DVec2>, mask: &[DVec2], mut exterior: impl FnMut(Vec<DVec2>)) -> Vec<DVec2> {
    for (&a, &b) in mask.iter().zip(mask.iter().cycle().skip(1)) {
        if signed_area(&p) <= AREA {
            return vec![];
        }
        let outside = half_plane(&p, a, b, false);
        if signed_area(&outside) > AREA {
            exterior(outside);
        }
        p = half_plane(&p, a, b, true);
    }
    if signed_area(&p) > AREA { p } else { vec![] }
}

fn overlaps(a: &[DVec2], b: &[DVec2]) -> bool {
    let bounds = |p: &[DVec2]| {
        p.iter().fold(
            (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
            |(lo, hi), &point| (lo.min(point), hi.max(point)),
        )
    };
    let (lo, hi) = bounds(a);
    let (other_lo, other_hi) = bounds(b);
    (hi.min(other_hi) - lo.max(other_lo)).min_element() > 0.
}

fn partition(
    p: Vec<DVec2>,
    masks: &[Vec<DVec2>],
    mut inside: impl FnMut(Vec<DVec2>),
) -> Vec<Vec<DVec2>> {
    let mut rest = vec![p];
    for mask in masks {
        let mut next = vec![];
        for p in rest {
            if overlaps(&p, mask) {
                let mut pieces = vec![];
                let covered = cut(p.clone(), mask, |p| pieces.push(p));
                if covered.is_empty() {
                    next.push(p);
                } else {
                    next.extend(pieces);
                    inside(covered);
                }
            } else {
                next.push(p);
            }
        }
        rest = next;
        if rest.is_empty() {
            break;
        }
    }
    rest
}

fn lift(plane: faces::Plane, p: DVec2) -> DVec3 {
    plane.origin + plane.u * p.x + plane.v * p.y
}

impl Model {
    pub fn glue_face(&mut self, id: u64) -> bool {
        self.faces.contains_key(&id)
            && !self.reference_images.contains_key(&id)
            && !self.is_hidden(id)
            && !self.is_locked(id)
            && self.wood_glue.insert(id, Coverage::Whole) != Some(Coverage::Whole)
    }

    pub(crate) fn clean_wood_glue(&mut self) {
        self.wood_glue
            .retain(|id, coverage| self.faces.get(id).is_some_and(|face| coverage.valid(face)));
    }

    // Store new coverage in the face's affine frame. Using three vertex IDs
    // keeps the patch attached through translation, rotation and nonuniform scale.
    pub(crate) fn add_glue_polygons(&mut self, id: u64, polygons: Vec<Vec<DVec3>>) -> bool {
        let face = &self.faces[&id];
        let Some(plane) = faces::plane(self, &face.vertices) else {
            return false;
        };
        let anchors = match self.wood_glue.get(&id) {
            Some(Coverage::Whole) => return false,
            Some(Coverage::Partial { anchors, .. }) => Some(*anchors),
            None => anchor_ids(self, face),
        };
        let Some(anchors) = anchors else { return false };
        let Some([o, b, c]) = anchor_points(self, anchors) else {
            return false;
        };
        let u = plane.project(b) - plane.project(o);
        let v = plane.project(c) - plane.project(o);
        let det = u.perp_dot(v);
        if det.abs() <= AREA {
            return false;
        }
        let mut masks: Vec<_> = self
            .wood_glue
            .get(&id)
            .into_iter()
            .flat_map(|coverage| coverage.polygons(self, face))
            .map(|p| ccw(p.into_iter().map(|p| plane.project(p)).collect()))
            .collect();
        let mut additions = vec![];
        for p in polygons {
            let p = ccw(p.into_iter().map(|p| plane.project(p)).collect());
            if signed_area(&p) <= AREA {
                continue;
            }
            let pieces = partition(p, &masks, |_| {});
            for piece in pieces.into_iter().filter(|p| signed_area(p) > AREA) {
                additions.push(
                    piece
                        .iter()
                        .map(|&p| {
                            let d = p - plane.project(o);
                            DVec2::new(d.perp_dot(v), u.perp_dot(d)) / det
                        })
                        .collect(),
                );
                masks.push(piece);
            }
        }
        if additions.is_empty() {
            return false;
        }
        if let Some(Coverage::Partial { polygons, .. }) = self.wood_glue.get_mut(&id) {
            polygons.extend(additions);
        } else {
            self.wood_glue.insert(
                id,
                Coverage::Partial {
                    anchors,
                    polygons: additions,
                },
            );
        }
        true
    }

    /// Reapply the intersection of an old coating and a replacement face.
    /// Never coat new stock created outside the original surface.
    pub(crate) fn transfer_glue(&mut self, id: u64, old: &[Vec<DVec3>]) {
        if old.is_empty() {
            return;
        }
        let face = &self.faces[&id];
        let Some(plane) = faces::plane(self, &face.vertices) else {
            return;
        };
        let masks: Vec<_> = old
            .iter()
            .filter(|p| {
                p.iter()
                    .all(|p| (*p - plane.origin).dot(plane.normal).abs() <= CONTACT)
            })
            .map(|p| ccw(p.iter().map(|p| plane.project(*p)).collect()))
            .collect();
        let mut covered = vec![];
        for t in self.face_triangles(face) {
            partition(ccw(t.map(|p| plane.project(p)).to_vec()), &masks, |p| {
                covered.push(p.into_iter().map(|p| lift(plane, p)).collect());
            });
        }
        self.add_glue_polygons(id, covered);
    }

    /// Split a mesh into bare and coated triangles. This is used when retained
    /// geometry changes, not when the camera or pointer moves.
    pub fn glue_mesh(
        &self,
        id: u64,
        triangles: &[[DVec3; 3]],
    ) -> (Vec<[DVec3; 3]>, Vec<[DVec3; 3]>) {
        let Some(coverage) = self.wood_glue.get(&id) else {
            return (triangles.to_vec(), vec![]);
        };
        if *coverage == Coverage::Whole {
            return (vec![], triangles.to_vec());
        }
        let face = &self.faces[&id];
        let Some(plane) = faces::plane(self, &face.vertices) else {
            return (triangles.to_vec(), vec![]);
        };
        let masks = coverage.polygons(self, face);
        split_mesh(plane, triangles, masks.iter().map(Vec::as_slice))
    }

    pub(crate) fn glued_polygons(&self, id: u64) -> Vec<Vec<DVec3>> {
        if !self.wood_glue.contains_key(&id) {
            return vec![];
        }
        self.glue_mesh(id, &self.face_triangles(&self.faces[&id]))
            .1
            .into_iter()
            .map(Vec::from)
            .collect()
    }

    /// Reattach a coating when machining replaces face IDs. Only surfaces
    /// that remain in the old plane can inherit coverage; fresh cuts are bare.
    pub(crate) fn transfer_glue_from(
        &mut self,
        source: &Self,
        source_faces: &BTreeSet<u64>,
        result_faces: impl IntoIterator<Item = u64>,
    ) {
        if source.wood_glue.is_empty() {
            return;
        }
        let coated: Vec<_> = source
            .wood_glue
            .keys()
            .filter(|id| source_faces.contains(id))
            .flat_map(|&id| source.glued_polygons(id))
            .collect();
        if !coated.is_empty() {
            for id in result_faces {
                if self.faces.contains_key(&id) {
                    self.transfer_glue(id, &coated);
                }
            }
        }
    }

    /// Area of the coated surfaces in square metres. Coplanar matching
    /// coverage contributes once, even when both sides of a joint are coated.
    pub fn wood_glue_area(&self) -> f64 {
        let mut surfaces: Vec<(faces::Plane, Vec<Vec<DVec2>>)> = Vec::new();
        let mut area = 0.;
        for id in self.wood_glue.keys() {
            let Some(face) = self.faces.get(id) else { continue };
            let Some(plane) = faces::plane(self, &face.vertices) else { continue };
            let (_, coated) = self.glue_mesh(*id, &self.face_triangles(face));
            for triangle in coated {
                let existing = surfaces.iter().position(|(other, _)| {
                    plane.normal.cross(other.normal).length() <= 1e-7
                        && triangle.iter().all(|p| {
                            (*p - other.origin).dot(other.normal).abs() <= CONTACT
                        })
                });
                let index = existing.unwrap_or_else(|| {
                    surfaces.push((plane, Vec::new()));
                    surfaces.len() - 1
                });
                let (plane, masks) = &mut surfaces[index];
                let polygon = ccw(triangle.map(|p| plane.project(p)).to_vec());
                // Subtract previously counted coverage, rather than halving
                // the total: unmatched portions still contribute in full.
                let additions = partition(polygon, masks, |_| {});
                area += additions.iter().map(|p| signed_area(p)).sum::<f64>();
                masks.extend(additions);
            }
        }
        area
    }

    pub fn glue_contacts(&mut self, selection: &BTreeSet<u64>) -> usize {
        struct Surface {
            id: u64,
            owner: Option<u64>,
            plane: faces::Plane,
            bounds: (DVec3, DVec3),
        }
        let owners: BTreeMap<_, _> = self
            .objects
            .values()
            .flat_map(|o| o.entities.iter().map(move |id| (*id, o.id)))
            .collect();
        let unavailable = |mut id| loop {
            if self.hidden.contains(&id)
                || self.locked.contains(&id)
                || self
                    .entity_tags
                    .get(&id)
                    .and_then(|tag| self.tags.get(tag))
                    .is_some_and(|tag| !tag.visible)
            {
                break true;
            }
            let Some(parent) = owners.get(&id) else {
                break false;
            };
            id = *parent;
        };
        let mut surfaces = vec![];
        for id in self.expand_entities(selection) {
            let Some(face) = self.faces.get(&id) else {
                continue;
            };
            if self.reference_images.contains_key(&id) || unavailable(id) {
                continue;
            }
            let Some(plane) = faces::plane(self, &face.vertices) else {
                continue;
            };
            let bounds = face
                .vertices
                .iter()
                .map(|id| self.vertices[id].position)
                .fold(
                    (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
                    |(lo, hi), p| (lo.min(p), hi.max(p)),
                );
            surfaces.push(Surface {
                id,
                owner: owners.get(&id).copied(),
                plane,
                bounds,
            });
        }
        let (lo, hi) = surfaces.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), s| (lo.min(s.bounds.0), hi.max(s.bounds.1)),
        );
        if surfaces.len() < 2 {
            return 0;
        }
        // Prefer the axis where each face spans the smallest share of the
        // selection. This narrows the sweep for dense, thin assemblies.
        let occupied = surfaces
            .iter()
            .map(|s| s.bounds.1 - s.bounds.0)
            .sum::<DVec3>();
        let separation = |axis: usize| {
            (hi[axis] - lo[axis]) / (occupied[axis] + CONTACT * surfaces.len() as f64)
        };
        let axis = (0..3)
            .max_by(|&a, &b| separation(a).total_cmp(&separation(b)))
            .unwrap();
        surfaces.sort_by(|a, b| {
            a.bounds.0[axis]
                .total_cmp(&b.bounds.0[axis])
                .then(a.id.cmp(&b.id))
        });
        let mut shells = BTreeMap::new();
        let mut triangulations = BTreeMap::new();
        let mut patches: BTreeMap<u64, Vec<Vec<DVec3>>> = BTreeMap::new();
        for (i, a) in surfaces.iter().enumerate() {
            for b in &surfaces[i + 1..] {
                if b.bounds.0[axis] > a.bounds.1[axis] + CONTACT {
                    break;
                }
                if (a.bounds.1.min(b.bounds.1) - a.bounds.0.max(b.bounds.0)).min_element()
                    < -CONTACT
                    || a.plane.normal.cross(b.plane.normal).length() > 1e-7
                    || ![(a, b), (b, a)].into_iter().all(|(x, y)| {
                        self.faces[&x.id]
                            .vertices
                            .iter()
                            .chain(self.faces[&x.id].holes.iter().flatten())
                            .all(|id| {
                                (self.vertices[id].position - y.plane.origin)
                                    .dot(y.plane.normal)
                                    .abs()
                                    <= CONTACT
                            })
                    })
                {
                    continue;
                }
                for id in [a.id, b.id] {
                    triangulations
                        .entry(id)
                        .or_insert_with(|| self.face_triangles(&self.faces[&id]));
                }
                let project = |p| a.plane.project(p);
                for ta in &triangulations[&a.id] {
                    let p = ccw(ta.map(project).to_vec());
                    for tb in &triangulations[&b.id] {
                        let mask = ccw(tb.map(project).to_vec());
                        if !overlaps(&p, &mask) {
                            continue;
                        }
                        let contact = cut(p.clone(), &mask, |_| {});
                        if contact.is_empty() {
                            continue;
                        }
                        let center = lift(
                            a.plane,
                            contact.iter().sum::<DVec2>() / contact.len() as f64,
                        );
                        // Face winding is not consistent in imported or drawn stock.
                        // When both pieces are closed, reject coplanar surfaces with
                        // stock on the same side (overlapping solids, not a joint).
                        let side = |s: &Surface, shells: &mut BTreeMap<_, _>| {
                            let owner = s.owner?;
                            let shell = shells
                                .entry(owner)
                                .or_insert_with(|| closed_shell(self, owner));
                            inside_positive(self, shell.as_deref()?, s.id, center, a.plane.normal)
                        };
                        if side(a, &mut shells)
                            .zip(side(b, &mut shells))
                            .is_some_and(|(x, y)| x == y)
                        {
                            continue;
                        }
                        patches
                            .entry(a.id)
                            .or_default()
                            .push(contact.iter().map(|&p| lift(a.plane, p)).collect());
                        patches.entry(b.id).or_default().push(
                            contact
                                .into_iter()
                                .map(|p| {
                                    let p = lift(a.plane, p);
                                    p - b.plane.normal * (p - b.plane.origin).dot(b.plane.normal)
                                })
                                .collect(),
                        );
                    }
                }
            }
        }
        patches
            .into_iter()
            .map(|(id, p)| usize::from(self.add_glue_polygons(id, p)))
            .sum()
    }
}

/// Partition an already clipped receiver, such as the precision viewport's
/// shadow mesh, against the cached coating. Geometry stays on its original plane.
pub fn split_mesh<'a>(
    plane: faces::Plane,
    triangles: &[[DVec3; 3]],
    masks: impl IntoIterator<Item = &'a [DVec3]>,
) -> (Vec<[DVec3; 3]>, Vec<[DVec3; 3]>) {
    let masks: Vec<_> = masks
        .into_iter()
        .map(|p| ccw(p.iter().map(|&p| plane.project(p)).collect()))
        .collect();
    let mut bare = vec![];
    let mut coated = vec![];
    let append = |out: &mut Vec<_>, p: Vec<DVec2>, reverse: bool| {
        for w in p[1..].windows(2) {
            if (w[0] - p[0]).perp_dot(w[1] - p[0]) <= 0. {
                continue;
            }
            let mut triangle = [p[0], w[0], w[1]].map(|p| lift(plane, p));
            if reverse {
                triangle.swap(1, 2);
            }
            out.push(triangle);
        }
    };
    for t in triangles {
        let p = t.map(|p| plane.project(p)).to_vec();
        let reverse = signed_area(&p) < 0.;
        let rest = partition(ccw(p), &masks, |p| append(&mut coated, p, reverse));
        for p in rest {
            append(&mut bare, p, reverse);
        }
    }
    (bare, coated)
}

fn closed_shell(model: &Model, owner: u64) -> Option<Vec<u64>> {
    let mut edges = BTreeMap::new();
    let mut ids = vec![];
    for face in model.objects[&owner]
        .entities
        .iter()
        .filter_map(|id| model.faces.get(id))
    {
        ids.push(face.id);
        for ring in std::iter::once(&face.vertices).chain(&face.holes) {
            for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
    }
    (ids.len() >= 4 && edges.values().all(|n| *n == 2)).then_some(ids)
}

fn inside_positive(
    model: &Model,
    shell: &[u64],
    face: u64,
    point: DVec3,
    normal: DVec3,
) -> Option<bool> {
    // A small oblique component avoids hitting opposite edges of rectangular
    // stock head-on. De-duplicate crossings shared by multiple adjacent faces.
    let tangent = normal.any_orthonormal_vector();
    let direction = normal + tangent * 0.037 + normal.cross(tangent) * 0.019;
    let mut hits = Vec::new();
    for &id in shell.iter().filter(|id| **id != face) {
        let face = &model.faces[&id];
        let plane = faces::plane(model, &face.vertices)?;
        let denominator = plane.normal.dot(direction);
        if denominator.abs() < 1e-12 {
            continue;
        }
        let t = (plane.origin - point).dot(plane.normal) / denominator;
        if t > CONTACT && model.face_contains(face, point + direction * t, true) {
            hits.push(t);
        }
    }
    hits.sort_by(f64::total_cmp);
    hits.dedup_by(|a, b| (*a - *b).abs() <= CONTACT);
    Some(hits.len() % 2 != 0)
}

#[cfg(test)]
#[path = "wood_glue_tests.rs"]
mod tests;
