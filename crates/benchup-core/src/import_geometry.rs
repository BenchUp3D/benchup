//! Normalize externally authored surfaces without moving shared vertices.
use crate::{DVec3, Edge, Face, Model, faces};
use std::collections::{BTreeMap, BTreeSet};

impl Model {
    /// Remove duplicate surfaces and boundary spurs, and tessellate polygons
    /// that cannot be represented by a strict planar face. Call on fresh,
    /// group-based imports before validation and before introducing linked
    /// component definitions.
    /// Simple planar faces stay intact.
    pub fn normalize_imported_faces(&mut self) -> Result<(), &'static str> {
        // SketchUp loops can walk a dangling edge in both directions. That
        // edge remains editable geometry, but is not part of the filled region.
        for face in self.faces.values_mut() {
            for ring in std::iter::once(&mut face.vertices).chain(&mut face.holes) {
                *ring = faces::strip_spurs(std::mem::take(ring));
            }
        }
        self.deduplicate_imported_faces();
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
                // A SketchUp boundary may revisit a vertex where two filled
                // regions touch. Earcut splits these pinched loops into simple
                // triangles; rejecting every repeated vertex loses valid
                // surfaces. Consecutive duplicates (including a repeated closing
                // vertex) still describe a zero-length boundary edge.
                if ring.len() < 3 || (0..ring.len()).any(|j| ring[j] == ring[(j + 1) % ring.len()])
                {
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
                let [a, b, c] = [points[t[0]], points[t[1]], points[t[2]]];
                let altitude = faces::min_altitude(a, b, c);
                if !altitude.is_finite() {
                    return Err(surface_error);
                }
                // A boundary split by neighbouring geometry leaves runs of
                // nearly collinear vertices. Earcut emits sliver triangles
                // between them that are too thin to carry a strict plane.
                // Skipping them drops at most TOL / 2 times the longest edge
                // in area, around a square millimetre on a two-metre board,
                // which is invisible next to the warp that caused it. The
                // altitude decides independently of the vertex order earcut
                // emits; the plane check is kept as a guard so every face
                // written here satisfies validation.
                if altitude <= faces::TOL || faces::plane(self, &ring).is_none() {
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
        // Tessellation can also reproduce a triangle already present in the
        // source. Keep one editable surface per exact boundary and properties.
        self.deduplicate_imported_faces();
        Ok(())
    }

    fn deduplicate_imported_faces(&mut self) {
        let mut seen = BTreeMap::new();
        let mut duplicates = BTreeMap::new();
        for face in self.faces.values() {
            let mut holes: Vec<_> = face
                .holes
                .iter()
                .map(|r| faces::canonical_ring(r))
                .collect();
            holes.sort();
            let key = (
                faces::canonical_ring(&face.vertices),
                holes,
                self.face_colors.get(&face.id).copied(),
                self.hidden.contains(&face.id),
                self.locked.contains(&face.id),
                self.entity_tags.get(&face.id).copied(),
            );
            if let Some(&original) = seen.get(&key) {
                duplicates.insert(face.id, original);
            } else {
                seen.insert(key, face.id);
            }
        }
        if duplicates.is_empty() {
            return;
        }
        for id in duplicates.keys() {
            self.faces.remove(id);
            self.face_colors.remove(id);
            self.hidden.remove(id);
            self.locked.remove(id);
            self.entity_tags.remove(id);
        }
        for object in self.objects.values_mut() {
            object.entities = std::mem::take(&mut object.entities)
                .into_iter()
                .map(|id| duplicates.get(&id).copied().unwrap_or(id))
                .collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vertex;

    fn imported_face(rings: &[&[[f64; 3]]]) -> Model {
        let mut model = Model::default();
        let mut vertices = BTreeMap::new();
        let rings: Vec<Vec<_>> = rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|p| {
                        *vertices.entry(p.map(f64::to_bits)).or_insert_with(|| {
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
    fn pinched_imported_boundary_keeps_both_lobes_and_surface_properties() {
        for height in [0., 0.00001] {
            let mut model = imported_face(&[&[
                [0., 0., 0.],
                [2., 0., 0.],
                [1., 1., height],
                [2., 2., 0.],
                [0., 2., 0.],
                [1., 1., height],
            ]]);
            let face = *model.faces.keys().next().unwrap();
            model.face_colors.insert(face, [210, 45, 30]);
            model.hidden.insert(face);
            // One lobe is also stored as a separate source triangle. Cleanup
            // must run after tessellation as well as before it.
            let triangle = model.id();
            model.faces.insert(
                triangle,
                Face {
                    id: triangle,
                    vertices: model.faces[&face].vertices[..3].to_vec(),
                    holes: vec![],
                },
            );
            model.face_colors.insert(triangle, [210, 45, 30]);
            model.hidden.insert(triangle);
            let group = model.id();
            model.objects.insert(
                group,
                serde_json::from_value(serde_json::json!({
                    "id": group, "name": "Pinched", "kind": "Group",
                    "entities": model.edges.keys().chain(model.faces.keys()).collect::<Vec<_>>(),
                }))
                .unwrap(),
            );
            let vertices = model.vertices.clone();
            let edges = model.edges.clone();
            assert!(model.validate().is_err());
            model.normalize_imported_faces().unwrap();
            model.validate().unwrap();
            assert_eq!(model.vertices, vertices);
            assert_eq!(model.edges, edges);
            assert_eq!(model.faces.len(), 2);
            assert!(model.faces.contains_key(&face));
            for face in model.faces.values() {
                assert_eq!(face.vertices.len(), 3);
                assert!(face.holes.is_empty());
                assert_eq!(model.face_colors[&face.id], [210, 45, 30]);
                assert!(model.hidden.contains(&face.id));
                assert!(model.objects[&group].entities.contains(&face.id));
            }
            let area: f64 = model.faces.values().map(|f| model.face_area(f)).sum();
            assert!((area - 2.).abs() < 1e-8);
            let before = model.clone();
            model.normalize_imported_faces().unwrap();
            assert_eq!(model, before);
            let restored: Model =
                serde_json::from_str(&serde_json::to_string(&model).unwrap()).unwrap();
            restored.validate().unwrap();
            assert_eq!(model, restored);
        }
    }

    #[test]
    fn pinched_hole_boundary_keeps_both_openings() {
        let mut model = imported_face(&[
            &[[0., 0., 0.], [4., 0., 0.], [4., 4., 0.], [0., 4., 0.]],
            &[
                [1., 1., 0.],
                [3., 1., 0.],
                [2., 2., 0.],
                [3., 3., 0.],
                [1., 3., 0.],
                [2., 2., 0.],
            ],
        ]);
        let vertices = model.vertices.clone();
        assert!(model.validate().is_err());
        model.normalize_imported_faces().unwrap();
        model.validate().unwrap();
        assert_eq!(model.vertices, vertices);
        let area: f64 = model.faces.values().map(|f| model.face_area(f)).sum();
        assert!((area - 14.).abs() < 1e-8);
        for p in [DVec3::new(2., 1.5, 0.), DVec3::new(2., 2.5, 0.)] {
            assert!(
                !model
                    .faces
                    .values()
                    .any(|f| model.face_contains(f, p, false))
            );
        }
    }

    #[test]
    fn duplicate_imported_surfaces_keep_holes_properties_and_group_membership() {
        let mut model = imported_face(&[
            &[[0., 0., 0.], [4., 0., 0.], [4., 4., 0.], [0., 4., 0.]],
            &[[1., 1., 0.], [3., 1., 0.], [3., 3., 0.], [1., 3., 0.]],
        ]);
        let face = model.faces.values().next().unwrap().clone();
        let duplicate = model.id();
        let mut copy = face.clone();
        copy.id = duplicate;
        copy.vertices.reverse();
        copy.vertices.rotate_left(1);
        copy.holes[0].reverse();
        model.faces.insert(duplicate, copy);
        for id in [face.id, duplicate] {
            model.face_colors.insert(id, [210, 45, 30]);
            model.hidden.insert(id);
            model.locked.insert(id);
        }
        let group = model.id();
        model.objects.insert(
            group,
            serde_json::from_value(serde_json::json!({
                "id": group, "name": "Duplicates", "kind": "Group",
                "entities": model.edges.keys().chain(model.faces.keys()).collect::<Vec<_>>(),
            }))
            .unwrap(),
        );
        assert!(model.validate().is_err());
        model.normalize_imported_faces().unwrap();
        model.validate().unwrap();
        assert_eq!(model.faces, BTreeMap::from([(face.id, face.clone())]));
        assert_eq!(
            model.face_colors,
            BTreeMap::from([(face.id, [210, 45, 30])])
        );
        assert_eq!(model.hidden, BTreeSet::from([face.id]));
        assert_eq!(model.locked, BTreeSet::from([face.id]));
        assert!(model.objects[&group].entities.contains(&face.id));
        assert!(!model.objects[&group].entities.contains(&duplicate));
        assert_eq!(model.face_area(&face), 12.);
        let before = model.clone();
        model.normalize_imported_faces().unwrap();
        assert_eq!(model, before);
        let restored: Model =
            serde_json::from_str(&serde_json::to_string(&model).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(model, restored);
    }

    #[test]
    fn coincident_boundaries_with_different_holes_or_colors_are_not_silently_removed() {
        for different_hole in [false, true] {
            let mut model = imported_face(&[
                &[[0., 0., 0.], [4., 0., 0.], [4., 4., 0.], [0., 4., 0.]],
                &[[1., 1., 0.], [3., 1., 0.], [3., 3., 0.], [1., 3., 0.]],
            ]);
            let mut copy = model.faces.values().next().unwrap().clone();
            copy.id = model.id();
            if different_hole {
                copy.holes.clear();
            } else {
                model.face_colors.insert(copy.id, [210, 45, 30]);
            }
            model.faces.insert(copy.id, copy);
            model.normalize_imported_faces().unwrap();
            assert_eq!(model.faces.len(), 2);
            assert!(model.validate().is_err());
        }
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

    /// A 45 mm x 2.15 m board face whose long edge was split by adjacent
    /// parts, so the ten boundary vertices tessellate into eight triangles.
    /// `depths` places each pair of split vertices below the corner plane.
    fn split_board(depths: [f64; 3]) -> Model {
        let z = 0.190587;
        imported_face(&[&[
            [1.320050, -0.000171, z],
            [1.365050, -0.000171, z],
            [1.365050, 2.150276, z],
            [1.320050, 2.150276, z],
            [1.320050, 2.149696, z - depths[0]],
            [1.320050, 2.106792, z - depths[0]],
            [1.320050, 0.389616, z - depths[1]],
            [1.320050, 0.346712, z - depths[1]],
            [1.320050, 0.043007, z - depths[2]],
            [1.320050, 0.000103, z - depths[2]],
        ]])
    }

    fn check_split_board(mut model: Model) -> Model {
        let vertices = model.vertices.clone();
        let edges = model.edges.clone();
        assert!(model.validate().is_err());
        model.normalize_imported_faces().unwrap();
        model.validate().unwrap();
        assert_eq!(model.vertices, vertices);
        for (id, edge) in &edges {
            assert_eq!(model.edges[id], *edge);
        }
        // Earcut emits eight triangles; the two slivers along the split edge
        // are skipped, not kept or rejected.
        assert_eq!(model.faces.len(), 6);
        for face in model.faces.values() {
            assert_eq!(face.vertices.len(), 3);
            let [a, b, c] = [0, 1, 2].map(|i| model.vertices[&face.vertices[i]].position);
            assert!(faces::min_altitude(a, b, c) > faces::TOL);
        }
        let area: f64 = model.faces.values().map(|f| model.face_area(f)).sum();
        assert!((area - 0.045 * 2.150447).abs() < 1e-6, "area {area}");
        let before = model.clone();
        model.normalize_imported_faces().unwrap();
        assert_eq!(model, before);
        model
    }

    #[test]
    fn split_boundary_with_collinear_slivers_imports_as_a_filled_surface() {
        // Every split vertex sits 7 um below the corners, so the slivers
        // between them are exactly collinear with zero area, which used to
        // reject the whole import.
        check_split_board(split_board([7e-6; 3]));
    }

    #[test]
    fn thin_slivers_that_pass_the_plane_check_are_still_skipped() {
        // The depths of the real deck model. Earcut lists two slivers with
        // their short edge first, so the third vertex clears the plane
        // tolerance even though the triangles are under a micrometre tall.
        check_split_board(split_board([10e-6, 8e-6, 5e-6]));
    }

    #[test]
    fn sliver_altitude_does_not_depend_on_vertex_order() {
        // A 1 mm base with a 10 um apex over a 2 m span: the plane check
        // accepts or rejects it depending on which vertex comes first.
        let model = imported_face(&[&[[0., 0., 0.], [0.001, 0., 0.], [2., 0.00001, 0.]]]);
        assert!(faces::plane(&model, &[1, 2, 3]).is_some());
        assert!(faces::plane(&model, &[2, 3, 1]).is_none());
        let p = |id: u64| model.vertices[&id].position;
        for (a, b, c) in [(1, 2, 3), (2, 3, 1), (3, 1, 2)] {
            assert!(faces::min_altitude(p(a), p(b), p(c)) <= faces::TOL);
        }
    }

    #[test]
    fn surface_made_only_of_slivers_still_fails() {
        // Not exactly collinear, so the Newell normal exists and earcut runs,
        // but the single triangle is half a micrometre tall.
        let mut model = imported_face(&[&[[0., 0., 0.], [1., 0., 0.], [2., 0.0000005, 0.]]]);
        assert_eq!(
            model.normalize_imported_faces(),
            Err(tr!("core.document.invalid_face_surface"))
        );
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
