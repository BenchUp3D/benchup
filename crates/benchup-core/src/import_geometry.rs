//! Normalize externally authored surfaces without moving shared vertices.
use crate::{DVec3, Edge, Face, Model, document::EPS, faces};
use std::collections::{BTreeMap, BTreeSet};

impl Model {
    /// Remove boundary spurs and tessellate imported polygons that cannot be
    /// represented by a strict planar face. Call on fresh, group-based imports
    /// before validation and before introducing linked component definitions.
    /// Planar faces stay intact.
    pub fn normalize_imported_faces(&mut self) -> Result<(), &'static str> {
        // SketchUp loops can walk a dangling edge in both directions. That
        // edge remains editable geometry, but is not part of the filled region.
        for face in self.faces.values_mut() {
            for ring in std::iter::once(&mut face.vertices).chain(&mut face.holes) {
                *ring = faces::strip_spurs(std::mem::take(ring));
            }
        }
        let warped: Vec<_> = self
            .faces
            .values()
            .filter(|face| {
                std::iter::once(&face.vertices)
                    .chain(&face.holes)
                    .any(|ring| faces::plane(self, ring).is_none())
            })
            .cloned()
            .collect();
        if warped.is_empty() {
            return Ok(());
        }
        let mut edges: BTreeMap<_, _> = self
            .edges
            .values()
            .map(|e| ((e.a.min(e.b), e.a.max(e.b)), e.id))
            .collect();
        for face in warped {
            let boundary_error = tr!("core.document.invalid_face_boundary");
            let surface_error = tr!("core.document.invalid_face_surface");
            let mut ids: Vec<u64> = Vec::new();
            let mut holes = Vec::new();
            for (i, ring) in std::iter::once(&face.vertices)
                .chain(&face.holes)
                .enumerate()
            {
                if ring.len() < 3 || ring.iter().collect::<BTreeSet<_>>().len() != ring.len() {
                    return Err(boundary_error);
                }
                if i > 0 {
                    holes.push(ids.len());
                }
                for j in 0..ring.len() {
                    let (a, b) = (ring[j], ring[(j + 1) % ring.len()]);
                    if !edges.contains_key(&(a.min(b), a.max(b))) {
                        return Err(tr!("core.document.face_boundary_is_missing_an_edge"));
                    }
                }
                ids.extend(ring);
            }
            let points: Vec<DVec3> = ids
                .iter()
                .map(|id| {
                    self.vertices
                        .get(id)
                        .map(|v| v.position)
                        .ok_or(boundary_error)
                })
                .collect::<Result<_, _>>()?;
            let origin = points[0];
            // Newell's normal uses the entire outer loop. A plane based on the
            // first three vertices is unstable when they are almost collinear.
            let outer: Vec<_> = points[..face.vertices.len()]
                .iter()
                .map(|p| *p - origin)
                .collect();
            let normal = (0..outer.len())
                .map(|i| outer[i].cross(outer[(i + 1) % outer.len()]))
                .sum::<DVec3>()
                .try_normalize()
                .ok_or(surface_error)?;
            let u = outer
                .iter()
                .map(|p| *p - normal * p.dot(normal))
                .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
                .and_then(DVec3::try_normalize)
                .ok_or(surface_error)?;
            let v = normal.cross(u);
            let coordinates: Vec<_> = points
                .iter()
                .flat_map(|p| {
                    let d = *p - origin;
                    [d.dot(u), d.dot(v)]
                })
                .collect();
            let indices = earcutr::earcut(&coordinates, &holes, 2).map_err(|_| surface_error)?;
            if indices.is_empty() {
                return Err(surface_error);
            }
            let mut triangles = Vec::new();
            for t in indices.chunks_exact(3) {
                let ring = vec![ids[t[0]], ids[t[1]], ids[t[2]]];
                let area = (points[t[1]] - points[t[0]])
                    .cross(points[t[2]] - points[t[0]])
                    .length()
                    * 0.5;
                if !area.is_finite() {
                    return Err(surface_error);
                }
                // A boundary split by neighbouring geometry leaves runs of
                // nearly collinear vertices. Earcut emits sliver triangles
                // between them that are too thin to carry a strict plane; the
                // filled region loses nothing visible when they are skipped.
                if area <= EPS * EPS || faces::plane(self, &ring).is_none() {
                    continue;
                }
                triangles.push(ring);
            }
            if triangles.is_empty() {
                return Err(surface_error);
            }
            // Bound expansion as well as parsing. IDs in fresh imports start at 1.
            if self
                .next_id
                .checked_add((triangles.len() * 4) as u64)
                .is_none_or(|n| n > 1_000_000)
            {
                return Err(tr!("core.document.model_is_too_large"));
            }
            let mut added = BTreeSet::new();
            for (i, vertices) in triangles.into_iter().enumerate() {
                for j in 0..3 {
                    let (a, b) = (vertices[j], vertices[(j + 1) % 3]);
                    edges.entry((a.min(b), a.max(b))).or_insert_with(|| {
                        let id = self.id();
                        self.edges.insert(id, Edge { id, a, b });
                        // Tessellation diagonals must not appear as drawn edges.
                        self.hidden.insert(id);
                        added.insert(id);
                        id
                    });
                }
                let id = if i == 0 { face.id } else { self.id() };
                self.faces.insert(
                    id,
                    Face {
                        id,
                        vertices,
                        holes: vec![],
                    },
                );
                if i > 0 {
                    added.insert(id);
                    if let Some(color) = self.face_colors.get(&face.id).copied() {
                        self.face_colors.insert(id, color);
                    }
                    if self.hidden.contains(&face.id) {
                        self.hidden.insert(id);
                    }
                }
            }
            for object in self
                .objects
                .values_mut()
                .filter(|o| o.entities.contains(&face.id))
            {
                object.entities.extend(&added);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vertex;

    fn imported_face(rings: &[&[[f64; 3]]]) -> Model {
        let mut model = Model::default();
        let rings: Vec<Vec<_>> = rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|p| {
                        let id = model.id();
                        model.vertices.insert(
                            id,
                            Vertex {
                                id,
                                position: DVec3::from_array(*p),
                            },
                        );
                        id
                    })
                    .collect()
            })
            .collect();
        for ring in &rings {
            for i in 0..ring.len() {
                model.insert_edge(ring[i], ring[(i + 1) % ring.len()]);
            }
        }
        let id = model.id();
        model.faces.insert(
            id,
            Face {
                id,
                vertices: rings[0].clone(),
                holes: rings[1..].to_vec(),
            },
        );
        model
    }

    #[test]
    fn sub_millimetre_imported_surface_survives_validation_and_reload() {
        let mut model = imported_face(&[&[[0., 0., 0.], [0.000254, 0., 0.], [0., 0.000254, 0.]]]);
        let before = model.clone();
        model.normalize_imported_faces().unwrap();
        assert_eq!(model, before);
        model.validate().unwrap();
        let restored: Model =
            serde_json::from_str(&serde_json::to_string(&model).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(model, restored);
    }

    #[test]
    fn warped_surface_keeps_hole_positions_color_visibility_and_group() {
        let mut model = imported_face(&[
            &[[0., 0., 0.], [1., 0., 0.], [1., 1., 0.00001], [0., 1., 0.]],
            &[
                [0.25, 0.25, 0.],
                [0.75, 0.25, 0.],
                [0.75, 0.75, 0.],
                [0.25, 0.75, 0.],
            ],
        ]);
        let face = *model.faces.keys().next().unwrap();
        model.face_colors.insert(face, [210, 45, 30]);
        let entities: BTreeSet<_> = model
            .edges
            .keys()
            .chain(model.faces.keys())
            .copied()
            .collect();
        let group = model.id();
        model.objects.insert(
            group,
            serde_json::from_value(serde_json::json!({
                "id": group, "name": "Imported", "kind": "Group", "entities": entities,
            }))
            .unwrap(),
        );
        model.hidden.insert(face);
        let vertices = model.vertices.clone();
        let edges = model.edges.clone();
        assert!(model.validate().is_err());
        model.normalize_imported_faces().unwrap();
        model.validate().unwrap();
        assert_eq!(model.vertices, vertices);
        assert_eq!(model.faces.len(), 8);
        for f in model.faces.values() {
            assert_eq!(f.vertices.len(), 3);
            assert!(f.holes.is_empty());
            assert_eq!(model.face_colors[&f.id], [210, 45, 30]);
            assert!(model.hidden.contains(&f.id));
            assert!(model.objects[&group].entities.contains(&f.id));
            let center = f
                .vertices
                .iter()
                .map(|id| model.vertices[id].position)
                .sum::<DVec3>()
                / 3.;
            assert!(center.x <= 0.25 || center.x >= 0.75 || center.y <= 0.25 || center.y >= 0.75);
        }
        for (id, edge) in &edges {
            assert_eq!(model.edges[id], *edge);
        }
        for id in model.edges.keys().filter(|id| !edges.contains_key(*id)) {
            assert!(model.hidden.contains(id));
            assert!(model.objects[&group].entities.contains(id));
        }
        assert!(
            (model
                .faces
                .values()
                .map(|f| model.face_area(f))
                .sum::<f64>()
                - 0.75)
                .abs()
                < 1e-8
        );
        let before = model.clone();
        model.normalize_imported_faces().unwrap();
        assert_eq!(model, before);
    }

    #[test]
    fn nearly_collinear_start_does_not_destabilize_imported_plane() {
        let mut model = imported_face(&[&[
            [0., 0., 0.],
            [1., 0., 0.],
            [1.001, 0.000002, 0.],
            [1.001, 0., 1.],
            [0., 0., 1.],
        ]]);
        assert!(model.validate().is_err());
        model.normalize_imported_faces().unwrap();
        model.validate().unwrap();
        assert_eq!(model.vertices.len(), 5);
        assert!(
            (model
                .faces
                .values()
                .map(|f| model.face_area(f))
                .sum::<f64>()
                - 1.001)
                .abs()
                < 2e-6
        );
    }

    #[test]
    fn hole_boundary_spur_keeps_its_loose_edge_and_the_original_opening() {
        let mut model = imported_face(&[
            &[[0., 0., 0.], [4., 0., 0.], [4., 4., 0.], [0., 4., 0.]],
            &[[1., 1., 0.], [3., 1., 0.], [3., 3., 0.], [1., 3., 0.]],
        ]);
        let face_id = *model.faces.keys().next().unwrap();
        let before = model.faces[&face_id].clone();
        let spur = model.id();
        model.vertices.insert(
            spur,
            Vertex {
                id: spur,
                position: DVec3::new(1., 5., 0.),
            },
        );
        model.insert_edge(before.holes[0][0], spur);
        model.faces.get_mut(&face_id).unwrap().holes[0].splice(1..1, [spur, before.holes[0][0]]);
        let vertices = model.vertices.clone();
        let edges = model.edges.clone();
        assert!(model.validate().is_err());
        model.normalize_imported_faces().unwrap();
        model.validate().unwrap();
        assert_eq!(model.vertices, vertices);
        assert_eq!(model.edges, edges);
        assert_eq!(model.faces[&face_id], before);
        assert_eq!(model.face_area(&model.faces[&face_id]), 12.);
    }

    #[test]
    fn split_boundary_with_collinear_slivers_imports_as_a_filled_surface() {
        // A 45 mm x 2.15 m board face whose long edge was split by adjacent
        // parts. The split vertices sit 5-10 um off the plane, so the face
        // is tessellated; earcut then emits slivers along the split edge.
        let mut model = imported_face(&[&[
            [1.320050, -0.000171, 0.190587],
            [1.365050, -0.000171, 0.190587],
            [1.365050, 2.150276, 0.190587],
            [1.320050, 2.150276, 0.190587],
            [1.320050, 2.149696, 0.190577],
            [1.320050, 2.106792, 0.190577],
            [1.320050, 0.389616, 0.190579],
            [1.320050, 0.346712, 0.190579],
            [1.320050, 0.043007, 0.190582],
            [1.320050, 0.000103, 0.190582],
        ]]);
        let vertices = model.vertices.clone();
        assert!(model.validate().is_err());
        model.normalize_imported_faces().unwrap();
        model.validate().unwrap();
        assert_eq!(model.vertices, vertices);
        assert!(model.faces.values().all(|f| f.vertices.len() == 3));
        let area: f64 = model.faces.values().map(|f| model.face_area(f)).sum();
        assert!((area - 0.045 * 2.150447).abs() < 1e-6, "area {area}");
        let before = model.clone();
        model.normalize_imported_faces().unwrap();
        assert_eq!(model, before);
    }

    #[test]
    fn malformed_boundaries_and_zero_area_surfaces_still_fail() {
        let mut collinear = imported_face(&[&[[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]]]);
        assert!(collinear.normalize_imported_faces().is_err());
        assert!(collinear.validate().is_err());
        let mut missing = imported_face(&[&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]]);
        missing.vertices.remove(&1);
        assert!(missing.normalize_imported_faces().is_err());
        let mut repeated = imported_face(&[&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]]);
        repeated.faces.values_mut().next().unwrap().vertices.push(1);
        assert!(repeated.normalize_imported_faces().is_err());
    }
}
