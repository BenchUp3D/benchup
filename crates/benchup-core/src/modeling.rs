//! Atomic, validated modeling operations shared by native and browser tools.
use crate::{DVec2, DVec3, Face, Model, Vertex, document::EPS, faces};
use std::collections::{BTreeMap, BTreeSet};

impl Model {
    pub fn selection_vertices(&self, selected: &BTreeSet<u64>) -> BTreeSet<u64> {
        let mut vertices = BTreeSet::new();
        for id in self.expand_entities(selected) {
            if let Some(edge) = self.edges.get(&id) {
                vertices.extend([edge.a, edge.b]);
            }
            if let Some(face) = self.faces.get(&id) {
                vertices.extend(face.vertices.iter().copied());
                vertices.extend(face.holes.iter().flatten().copied());
            }
        }
        vertices
    }

    pub fn selection_center(&self, selected: &BTreeSet<u64>) -> Option<DVec3> {
        self.selection_bounds(selected)
            .map(|(lo, hi)| (lo + hi) * 0.5)
    }

    pub fn selection_bounds(&self, selected: &BTreeSet<u64>) -> Option<(DVec3, DVec3)> {
        let ids = self.selection_vertices(selected);
        let entities = self.expand_entities(selected);
        let points = ids.iter().map(|id| self.vertices[id].position).chain(
            entities
                .iter()
                .filter_map(|id| self.dimensions.get(id))
                .flat_map(|d| d.annotation_points(self)),
        );
        let points = points.chain(
            entities
                .iter()
                .filter_map(|id| self.texts.get(id))
                .flat_map(|t| t.layout(self)),
        );
        let mut points = points.chain(
            entities
                .iter()
                .filter_map(|id| self.guides.get(id))
                .map(|g| g.origin),
        );
        let first = points.next()?;
        Some(points.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }

    fn transform_selection(
        &mut self,
        selected: &BTreeSet<u64>,
        f: impl Fn(DVec3) -> DVec3,
    ) -> Result<(), &'static str> {
        if self.selection_locked(selected) {
            return Err(tr!("core.modeling.unlock_the_selection_before_editing_it"));
        }
        let vertices = self.selection_vertices(selected);
        let dimensions: Vec<_> = self
            .expand_entities(selected)
            .into_iter()
            .filter(|id| self.dimensions.contains_key(id))
            .collect();
        let texts: Vec<_> = self
            .expand_entities(selected)
            .into_iter()
            .filter(|id| self.texts.contains_key(id))
            .collect();
        let guides: Vec<_> = self
            .expand_entities(selected)
            .into_iter()
            .filter(|id| self.guides.contains_key(id))
            .collect();
        if vertices.is_empty() && dimensions.is_empty() && texts.is_empty() && guides.is_empty() {
            return Err(tr!("core.modeling.select_geometry_a_dimension_text_or_a_guide"));
        }
        let protected: BTreeSet<_> = self
            .edges
            .keys()
            .chain(self.faces.keys())
            .copied()
            .filter(|id| self.is_locked(*id))
            .collect();
        if !vertices.is_disjoint(&self.selection_vertices(&protected)) {
            return Err(tr!("core.modeling.unlock_the_connected_geometry_before_editing_it"));
        }
        let mut next = self.clone();
        for id in guides {
            let source = &self.guides[&id];
            let origin = f(source.origin);
            let direction = source
                .direction
                .map(|d| {
                    (f(source.origin + d) - origin)
                        .try_normalize()
                        .ok_or(tr!("core.modeling.this_change_would_collapse_a_guide"))
                })
                .transpose()?;
            next.guides
                .insert(id, crate::ConstructionGuide { origin, direction });
        }
        for id in dimensions {
            let d = &self.dimensions[&id];
            let (a, b) = d.points(self);
            let changed = next.dimensions.get_mut(&id).unwrap();
            changed.start.point = f(a);
            changed.end.point = f(b);
            if let Some(vertex) = &mut changed.vertex { vertex.point = f(d.vertex.as_ref().unwrap().position(self)); }
            changed.offset = f(a + d.offset) - f(a);
            for anchor in changed.anchors_mut() {
                if anchor.weights.iter().any(|(id, _)| !vertices.contains(id)) {
                    anchor.weights.clear();
                }
            }
        }
        for id in texts {
            let source = &self.texts[&id];
            let [a, b] = source.layout(self);
            let changed = next.texts.get_mut(&id).unwrap();
            let attached_geometry_moves = !source.anchor.weights.is_empty()
                && source
                    .anchor
                    .weights
                    .iter()
                    .all(|(id, _)| vertices.contains(id));
            // Moving a label alone repositions the text without losing its attachment.
            let anchor = if attached_geometry_moves || source.anchor.weights.is_empty() {
                f(a)
            } else {
                a
            };
            changed.anchor.point = anchor;
            changed.offset = f(b) - anchor;
        }
        for id in vertices {
            next.vertices.get_mut(&id).unwrap().position = f(self.vertices[&id].position);
        }
        next.sync_dimensions();
        next.validate().map_err(
            |_| tr!("core.modeling.this_change_would_collapse_an_edge_or_bend"),
        )?;
        *self = next;
        Ok(())
    }

    pub fn move_selection(
        &mut self,
        selected: &BTreeSet<u64>,
        delta: DVec3,
    ) -> Result<(), &'static str> {
        if !delta.is_finite() {
            return Err(tr!("core.modeling.enter_a_finite_displacement"));
        }
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.move_selection(selected, delta));
        }
        self.transform_selection(selected, |p| p + delta)?;
        for id in self.expand_entities(selected) {
            if let Some(object) = self.objects.get_mut(&id) {
                object.transform = Some(object.transform.unwrap_or_default().translated(delta));
            }
        }
        Ok(())
    }

    /// Rotate atomically about any world-space pivot, retaining component frames.
    pub fn rotate_selection(
        &mut self,
        selected: &BTreeSet<u64>,
        pivot: DVec3,
        axis: DVec3,
        degrees: f64,
    ) -> Result<(), &'static str> {
        if !pivot.is_finite() || !axis.is_finite() || !degrees.is_finite() {
            return Err(tr!("core.modeling.enter_a_finite_rotation_and_pivot"));
        }
        let axis = axis.try_normalize().ok_or(tr!("core.modeling.choose_a_rotation_axis_first"))?;
        if self.edit_context.is_some() {
            return self
                .edit_in_context(|model| model.rotate_selection(selected, pivot, axis, degrees));
        }
        let rotation = glam::DMat3::from_axis_angle(axis, degrees.to_radians());
        let rotate = |p: DVec3| pivot + rotation * (p - pivot);
        self.transform_selection(selected, rotate)?;
        for id in self.expand_entities(selected) {
            if let Some(object) = self.objects.get_mut(&id) {
                let mut frame = object.transform.unwrap_or_default();
                frame.translation = rotate(frame.translation);
                frame.basis = rotation * frame.basis;
                object.transform = Some(frame);
            }
        }
        Ok(())
    }

    /// Mirror the selection about its bounds center without detaching component instances.
    pub fn flip_selection(
        &mut self,
        selected: &BTreeSet<u64>,
        axis: crate::Axis,
    ) -> Result<(), &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.flip_selection(selected, axis));
        }
        let center = self
            .selection_center(selected)
            .ok_or(tr!("core.modeling.select_geometry_to_flip_first"))?;
        let normal = axis.vector();
        let mirror = |p: DVec3| p - normal * 2. * (p - center).dot(normal);
        self.transform_selection(selected, mirror)?;
        let signs = DVec3::ONE - normal * 2.;
        for id in self.expand_entities(selected) {
            if let Some(object) = self.objects.get_mut(&id) {
                let mut frame = object.transform.unwrap_or_default();
                frame.translation = mirror(frame.translation);
                frame.basis = glam::DMat3::from_diagonal(signs)
                    * frame.basis
                    * glam::DMat3::from_diagonal(signs);
                frame.axes *= signs;
                object.transform = Some(frame);
            }
        }
        Ok(())
    }

    pub fn scale_selection(
        &mut self,
        selected: &BTreeSet<u64>,
        center: DVec3,
        factor: f64,
    ) -> Result<(), &'static str> {
        if !factor.is_finite() || factor <= EPS || factor > 1e7 {
            return Err(tr!("core.modeling.enter_a_positive_scale_factor_such_as_2"));
        }
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.scale_selection(selected, center, factor));
        }
        self.transform_selection(selected, |p| center + (p - center) * factor)?;
        for id in self.expand_entities(selected) {
            if let Some(object) = self.objects.get_mut(&id) {
                object.transform =
                    Some(object.transform.unwrap_or_default().scaled(center, factor));
            }
        }
        Ok(())
    }

    /// Scale only the world axes selected by an explicit axis or plane lock.
    pub fn scale_selection_constrained(
        &mut self,
        selected: &BTreeSet<u64>,
        center: DVec3,
        factor: f64,
        constraint: Option<crate::inference::AxisConstraint>,
    ) -> Result<(), &'static str> {
        let Some(constraint) = constraint else {
            return self.scale_selection(selected, center, factor);
        };
        if !factor.is_finite() || factor <= EPS || factor > 1e7 {
            return Err(tr!("core.modeling.enter_a_positive_scale_factor_such_as_2"));
        }
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| {
                model.scale_selection_constrained(selected, center, factor, Some(constraint))
            });
        }
        let factors = DVec3::ONE + constraint.project(DVec3::ONE) * (factor - 1.);
        self.transform_selection(selected, |p| center + (p - center) * factors)?;
        for id in self.expand_entities(selected) {
            if let Some(object) = self.objects.get_mut(&id) {
                let mut frame = object.transform.unwrap_or_default();
                frame.translation = center + (frame.translation - center) * factors;
                frame.basis = glam::DMat3::from_diagonal(factors)
                    * frame.basis
                    * glam::DMat3::from_diagonal(DVec3::ONE / factors);
                frame.axes *= factors;
                object.transform = Some(frame);
            }
        }
        Ok(())
    }

    pub fn paint_face(&mut self, face: u64, color: [u8; 3]) -> Result<(), &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.paint_face(face, color));
        }
        if self.is_locked(face) {
            return Err(tr!("core.modeling.unlock_the_face_before_painting_it"));
        }
        if !self.faces.contains_key(&face) {
            return Err(tr!("core.modeling.click_a_filled_face_to_paint_it"));
        }
        self.set_face_color(face, Some(color));
        Ok(())
    }

    /// Remove paint from selected faces and all descendants as one atomic edit.
    pub fn clear_colors(&mut self, selected: &BTreeSet<u64>) -> Result<(), &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.clear_colors(selected));
        }
        let entities = self.expand_entities(selected);
        if entities.iter().any(|id| self.is_locked(*id)) {
            return Err(tr!("core.modeling.unlock_the_selected_items_and_their_children_before"));
        }
        for face in entities
            .into_iter()
            .filter(|id| self.faces.contains_key(id))
            .collect::<Vec<_>>()
        {
            self.set_face_color(face, None);
        }
        Ok(())
    }

    fn set_face_color(&mut self, face: u64, color: Option<[u8; 3]>) {
        // A component's material belongs to its definition; update matching faces
        // in all instances while retaining each instance's position and scale.
        let definition = self
            .objects
            .values()
            .filter_map(|object| {
                let crate::ObjectKind::Component { definition } = object.kind else {
                    return None;
                };
                object
                    .definition_entities
                    .iter()
                    .find(|(_, id)| **id == face)
                    .map(|(key, _)| (definition, *key))
            })
            .next();
        let mut faces = BTreeSet::from([face]);
        if let Some((definition, key)) = definition {
            faces.extend(self.objects.values().filter_map(|object| {
                (object.kind == crate::ObjectKind::Component { definition })
                    .then(|| object.definition_entities.get(&key).copied())
                    .flatten()
            }));
        }
        for face in faces {
            if let Some(color) = color {
                self.face_colors.insert(face, color);
            } else {
                self.face_colors.remove(&face);
            }
        }
    }

    /// Paint all descendant faces as one atomic edit, preserving component materials.
    pub fn paint_object(&mut self, object: u64, color: [u8; 3]) -> Result<(), &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.paint_object(object, color));
        }
        let object = self
            .objects
            .get(&object)
            .ok_or(tr!("core.modeling.click_a_face_in_a_group_or_component"))?;
        if self.is_locked(object.id) {
            return Err(tr!("core.modeling.unlock_the_group_or_component_before_painting_it"));
        }
        let faces: Vec<_> = self
            .expand_entities(&object.entities)
            .into_iter()
            .filter(|id| self.faces.contains_key(id))
            .collect();
        if faces.is_empty() {
            return Err(tr!("core.modeling.the_group_or_component_has_no_faces_to"));
        }
        if faces.iter().any(|id| self.is_locked(*id)) {
            return Err(tr!("core.modeling.unlock_all_faces_in_the_group_or_component"));
        }
        let mut next = self.clone();
        for face in faces {
            next.paint_face(face, color)?;
        }
        *self = next;
        Ok(())
    }

    fn editable_surface(&self, face: u64) -> Result<(), &'static str> {
        if self.reference_images.contains_key(&face) {
            return Err("Use image Properties, Move, Rotate or Scale to edit a reference image.");
        }
        if self.is_locked(face) {
            return Err(tr!("core.modeling.unlock_the_object_before_editing_it"));
        }
        if self.parent(face).is_some() {
            return Err(tr!("core.modeling.double_click_the_object_to_edit_its_faces"));
        }
        Ok(())
    }

    fn raw_vertex(&mut self, position: DVec3) -> u64 {
        let id = self.id();
        self.vertices.insert(id, Vertex { id, position });
        id
    }
    fn raw_face(
        &mut self,
        vertices: Vec<u64>,
        holes: Vec<Vec<u64>>,
        color: Option<[u8; 3]>,
    ) -> u64 {
        let key = faces::canonical_ring(&vertices);
        if let Some(id) = self
            .faces
            .values()
            .find(|face| face.vertices.len() == vertices.len() && faces::canonical_ring(&face.vertices) == key)
            .map(|face| face.id)
        {
            // Connecting the extrusion's edges can already have filled this
            // region while splitting a neighboring surface.
            self.faces.get_mut(&id).unwrap().holes = holes;
            if let Some(color) = color {
                self.face_colors.insert(id, color);
            }
            return id;
        }
        for ring in std::iter::once(&vertices).chain(holes.iter()) {
            for i in 0..ring.len() {
                self.insert_edge(ring[i], ring[(i + 1) % ring.len()]);
            }
        }
        let id = self.id();
        self.faces.insert(
            id,
            Face {
                id,
                vertices,
                holes,
            },
        );
        if let Some(color) = color {
            self.face_colors.insert(id, color);
        }
        id
    }

    /// Connect new extrusion boundaries to the editable drawing before adding
    /// their explicit faces. Raw walls alone do not split an existing floor.
    fn connect_extrusion(
        &mut self,
        rings: &mut [Vec<u64>],
        mut drawing_edges: BTreeSet<u64>,
        drawing_vertices: &BTreeSet<u64>,
        surfaces: &[faces::Plane],
    ) {
        let old_edges = drawing_edges.clone();
        for ring in rings.iter() {
            for i in 0..ring.len() {
                drawing_edges.insert(self.insert_edge(ring[i], ring[(i + 1) % ring.len()]));
            }
        }
        let mut vertices = drawing_vertices.clone();
        vertices.extend(rings.iter().flatten().copied());
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for id in rings.iter().flatten() {
            lo = lo.min(self.vertices[id].position);
            hi = hi.max(self.vertices[id].position);
        }
        let vertices: Vec<_> = vertices
            .into_iter()
            .filter(|id| {
                let point = self.vertices[id].position;
                point.cmpge(lo - DVec3::splat(EPS)).all()
                    && point.cmple(hi + DVec3::splat(EPS)).all()
            })
            .collect();
        for &id in &vertices {
            self.split_at(id, &mut drawing_edges);
        }
        // A side can retrace a boundary containing intermediate vertices. Its
        // face must use the split edges too, rather than add a long duplicate.
        for ring in rings {
            let mut split = Vec::new();
            for i in 0..ring.len() {
                let a = ring[i];
                let b = ring[(i + 1) % ring.len()];
                let start = self.vertices[&a].position;
                let delta = self.vertices[&b].position - start;
                let mut interior: Vec<_> = vertices
                    .iter()
                    .filter(|&&id| id != a && id != b)
                    .filter_map(|&id| {
                        let point = self.vertices[&id].position;
                        let t = (point - start).dot(delta) / delta.length_squared();
                        (t > EPS && t < 1. - EPS && (start + delta * t).distance(point) < EPS)
                            .then_some((t, id))
                    })
                    .collect();
                interior.sort_by(|a, b| a.0.total_cmp(&b.0));
                split.push(a);
                split.extend(interior.into_iter().map(|(_, id)| id));
            }
            *ring = split;
        }
        // Only coplanar contacts need planar region discovery. The other walls
        // and the cap retain their explicit winding and hole boundaries.
        for id in drawing_edges
            .difference(&old_edges)
            .copied()
            .collect::<Vec<_>>()
        {
            let edge = &self.edges[&id];
            let (a, b) = self.points(edge);
            if surfaces.iter().any(|plane| {
                (a - plane.origin).dot(plane.normal).abs() < faces::TOL
                    && (b - plane.origin).dot(plane.normal).abs() < faces::TOL
            }) {
                self.create_faces_with_edges(edge.a, edge.b, &drawing_edges);
            }
        }
    }

    /// Extrude a planar face, including its hole walls. Keep a base only when the
    /// face is not joined to a coplanar surface, so inset regions open into recesses.
    pub fn push_pull(&mut self, face_id: u64, distance: f64) -> Result<u64, &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.push_pull(face_id, distance));
        }
        self.editable_surface(face_id)?;
        if !distance.is_finite() || distance.abs() < EPS || distance.abs() > 1e7 {
            return Err(tr!("core.modeling.enter_a_nonzero_distance_in_meters"));
        }
        let face = self
            .faces
            .get(&face_id)
            .ok_or(tr!("core.modeling.click_a_filled_face_first"))?
            .clone();
        let normal = faces::plane(self, &face.vertices)
            .ok_or(tr!("core.modeling.the_face_must_be_planar"))?
            .normal;
        self.push_pull_along(face_id, normal * distance)
    }

    /// Extrude a face by a world-space displacement, including oblique pulls
    /// along a locked axis. A displacement within the face cannot form a solid.
    pub fn push_pull_along(&mut self, face_id: u64, delta: DVec3) -> Result<u64, &'static str> {
        self.extrude_face(face_id, delta, false)
    }

    /// Add a new cap and walls on an outward pull. Inward pulls cut the existing
    /// body instead of leaving the starting face and overlapping walls inside it.
    pub fn push_pull_along_with_new_faces(
        &mut self,
        face_id: u64,
        delta: DVec3,
    ) -> Result<u64, &'static str> {
        self.extrude_face(face_id, delta, true)
    }

    fn inward_new_face(&mut self, face_id: u64, delta: DVec3) -> Result<Option<u64>, &'static str> {
        use crate::{csg, solids};

        // Restrict the solid operation to the connected body in this edit scope.
        // Rendering still uses the shared retained GPU path; this work changes
        // authoritative topology, including contacts at the far side of a cut.
        let ids = self.solid_entities(face_id)?;
        let mut body = self.copy_entities(&ids);
        let frame = solids::Frame::new(&[&body])?;
        let solid = match solids::import(&body, frame) {
            Ok(solid) => solid,
            Err(_) => {
                // Earlier outward Ctrl pulls leave a partition between layers.
                // Omit these internal caps when resolving the body's shell.
                let mut uses: BTreeMap<(u64, u64), Vec<u64>> = BTreeMap::new();
                for face in body.faces.values() {
                    for ring in std::iter::once(&face.vertices).chain(&face.holes) {
                        for i in 0..ring.len() {
                            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                            uses.entry((a.min(b), a.max(b))).or_default().push(face.id);
                        }
                    }
                }
                let partitions: BTreeSet<_> = body
                    .faces
                    .values()
                    .filter(|face| {
                        let Some(plane) = faces::plane(&body, &face.vertices) else {
                            return false;
                        };
                        face.id != face_id
                            && std::iter::once(&face.vertices)
                                .chain(&face.holes)
                                .all(|ring| {
                                    (0..ring.len()).all(|i| {
                                        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                                        // Branches can put four or more faces
                                        // on a seam. Only a partition has walls
                                        // extending to both sides of its plane
                                        // around its complete boundary.
                                        let mut positive = false;
                                        let mut negative = false;
                                        for id in &uses[&(a.min(b), a.max(b))] {
                                            if *id == face.id {
                                                continue;
                                            }
                                            let other = &body.faces[id];
                                            for vertex in other
                                                .vertices
                                                .iter()
                                                .chain(other.holes.iter().flatten())
                                            {
                                                let depth = (body.vertices[vertex].position
                                                    - plane.origin)
                                                    .dot(plane.normal);
                                                positive |= depth > faces::TOL;
                                                negative |= depth < -faces::TOL;
                                            }
                                        }
                                        positive && negative
                                    })
                                })
                    })
                    .map(|face| face.id)
                    .collect();
                if partitions.is_empty() {
                    return Ok(None);
                }
                body.erase(&partitions);
                let Ok(solid) = solids::import(&body, frame) else {
                    return Ok(None);
                };
                solid
            }
        };
        let face = &self.faces[&face_id];
        let plane = faces::plane(self, &face.vertices).unwrap();
        let origin = frame.local(plane.origin);
        // Imported solid polygons have outward winding, even for reversed or
        // rotated source faces. A negative UI value alone cannot identify a cut.
        let inward = solid
            .iter()
            .find(|polygon| {
                polygon
                    .vertices
                    .iter()
                    .all(|p| (*p - origin).dot(plane.normal).abs() < frame.epsilon * 2.)
                    && polygon.normal.cross(plane.normal).length() < 1e-6
            })
            .is_some_and(|polygon| polygon.normal.dot(delta) < -EPS);
        if !inward {
            return Ok(None);
        }
        if self.selection_locked(&ids) {
            return Err(tr!(
                "core.modeling.unlock_the_connected_geometry_before_editing_it"
            ));
        }
        // A swept copy supplies a closed cutter, including concave outlines and
        // holes. Subtraction removes the original cap, trims shared walls, and
        // welds contact edges rather than overlaying another extruded shell.
        let mut cutter = self.copy_entities(&BTreeSet::from([face_id]));
        cutter.extrude_face(face_id, delta, false)?;
        let result = csg::apply(
            solid,
            solids::import(&cutter, frame)?,
            csg::Boolean::Difference,
            frame.epsilon,
        )?;
        let mut next = self.clone();
        next.erase(&ids);
        let group = solids::output(&mut next, result, frame, "", false, true)?;
        let result_ids = group
            .map(|id| next.dissolve_objects(&BTreeSet::from([id]), true))
            .unwrap_or_default();
        let result_faces: Vec<_> = result_ids
            .into_iter()
            .filter(|id| next.faces.contains_key(id))
            .collect();
        for id in &result_faces {
            let point = next.face_triangles(&next.faces[id])[0]
                .iter()
                .copied()
                .sum::<DVec3>()
                / 3.;
            let tag = match body
                .faces
                .values()
                .find(|face| body.face_contains(face, point, false))
            {
                Some(face) => body.entity_tags.get(&face.id),
                None => self.entity_tags.get(&face_id),
            }
            .copied();
            if let Some(tag) = tag {
                next.entity_tags.insert(*id, tag);
            }
        }
        let cap_point = self.face_triangles(face)[0].iter().copied().sum::<DVec3>() / 3. + delta;
        let cap = result_faces
            .iter()
            .copied()
            .find(|id| {
                let face = &next.faces[id];
                faces::plane(&next, &face.vertices)
                    .is_some_and(|p| p.normal.cross(plane.normal).length() < 1e-6)
                    && next.face_contains(face, cap_point, true)
            })
            .or_else(|| result_faces.last().copied());
        // Keep the cap last, matching ordinary extrusions and viewport selection.
        // A cut consuming the whole body has no remaining face to select.
        let cap = cap
            .map(|id| {
                let mut face = next.faces.remove(&id).unwrap();
                let cap = next.id();
                face.id = cap;
                next.faces.insert(cap, face);
                if let Some(color) = next.face_colors.remove(&id) {
                    next.face_colors.insert(cap, color);
                }
                if let Some(tag) = next.entity_tags.remove(&id) {
                    next.entity_tags.insert(cap, tag);
                }
                cap
            })
            .unwrap_or(face_id);
        next.validate()?;
        *self = next;
        Ok(Some(cap))
    }

    fn extrude_face(
        &mut self,
        face_id: u64,
        delta: DVec3,
        keep_starting_face: bool,
    ) -> Result<u64, &'static str> {
        if self.edit_context.is_some() {
            return self
                .edit_in_context(|model| model.extrude_face(face_id, delta, keep_starting_face));
        }
        self.editable_surface(face_id)?;
        let distance = delta.length();
        if !delta.is_finite() || distance < EPS || distance > 1e7 {
            return Err(tr!("core.modeling.enter_a_nonzero_distance_in_meters"));
        }
        let face = self
            .faces
            .get(&face_id)
            .ok_or(tr!("core.modeling.click_a_filled_face_first"))?
            .clone();
        let normal = faces::plane(self, &face.vertices)
            .ok_or(tr!("core.modeling.the_face_must_be_planar"))?
            .normal;
        if delta.dot(normal).abs() < EPS {
            return Err(tr!("core.modeling.choose_an_axis_that_is_not_parallel_to"));
        }
        if keep_starting_face && let Some(cap) = self.inward_new_face(face_id, delta)? {
            return Ok(cap);
        }
        let color = self.face_colors.get(&face_id).copied();
        // On an existing solid, moving the cap stretches the adjoining walls.
        let boundary: Vec<_> = std::iter::once(&face.vertices)
            .chain(face.holes.iter())
            .flat_map(|ring| (0..ring.len()).map(move |i| (ring[i], ring[(i + 1) % ring.len()])))
            .collect();
        let shares_edge = |other: &Face, a, b| {
            std::iter::once(&other.vertices)
                .chain(other.holes.iter())
                .any(|ring| {
                    (0..ring.len()).any(|i| {
                        let (c, d) = (ring[i], ring[(i + 1) % ring.len()]);
                        (a == c && b == d) || (a == d && b == c)
                    })
                })
        };
        if !keep_starting_face
            && boundary.iter().all(|&(a, b)| {
                self.faces.values().any(|other| {
                    other.id != face_id
                        && self.parent(other.id).is_none()
                        && faces::plane(self, &other.vertices)
                            .is_some_and(|p| p.normal.cross(normal).length() > EPS)
                        && shares_edge(other, a, b)
                })
            })
        {
            self.move_selection(&BTreeSet::from([face_id]), delta)?;
            return Ok(face_id);
        }
        let mut next = self.clone();
        let drawing_edges = next.drawing_edges();
        let drawing_vertices: BTreeSet<_> = drawing_edges
            .iter()
            .flat_map(|id| [next.edges[id].a, next.edges[id].b])
            .collect();
        let surfaces: Vec<_> = self
            .faces
            .values()
            .filter(|face| {
                self.parent(face.id).is_none()
                    && !self.is_hidden(face.id)
                    && !self.is_locked(face.id)
            })
            .filter_map(|face| faces::plane(self, &face.vertices))
            .collect();
        let joined_surface = self.faces.values().any(|other| {
            other.id != face_id
                && self.parent(other.id).is_none()
                && faces::plane(self, &other.vertices)
                    .is_some_and(|p| p.normal.cross(normal).length() <= EPS)
                && boundary.iter().any(|&(a, b)| shares_edge(other, a, b))
        });
        if joined_surface && !keep_starting_face {
            // The surrounding surface already supplies the opening's boundary.
            // Leaving this face behind would cover the recess and add an internal cap.
            next.erase(&BTreeSet::from([face_id]));
        }
        let mut top = BTreeMap::new();
        for id in std::iter::once(&face.vertices)
            .chain(face.holes.iter())
            .flatten()
        {
            top.entry(*id).or_insert_with(|| {
                let point = self.vertices[id].position + delta;
                next.vertices
                    .values()
                    .find(|v| {
                        !keep_starting_face
                            && drawing_vertices.contains(&v.id)
                            && v.position.distance(point) < EPS
                    })
                    .map(|v| v.id)
                    .unwrap_or_else(|| next.raw_vertex(point))
            });
        }
        let mut rings = Vec::new();
        for ring in std::iter::once(&face.vertices).chain(face.holes.iter()) {
            for i in 0..ring.len() {
                let a = ring[i];
                let b = ring[(i + 1) % ring.len()];
                rings.push(vec![a, b, top[&b], top[&a]]);
            }
        }
        let walls = rings.len();
        rings.push(face.vertices.iter().map(|id| top[id]).collect());
        rings.extend(
            face.holes
                .iter()
                .map(|r| r.iter().map(|id| top[id]).collect::<Vec<_>>()),
        );
        if !keep_starting_face {
            next.connect_extrusion(&mut rings, drawing_edges, &drawing_vertices, &surfaces);
        }
        for ring in &rings[..walls] {
            next.raw_face(ring.clone(), vec![], color);
        }
        let cap = next.raw_face(rings[walls].clone(), rings[walls + 1..].to_vec(), color);
        next.validate()?;
        *self = next;
        Ok(cap)
    }

    /// Positive offsets inset the surface; negative offsets extend it outward.
    pub fn offset_face(&mut self, face_id: u64, distance: f64) -> Result<(), &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.offset_face(face_id, distance));
        }
        self.editable_surface(face_id)?;
        if !distance.is_finite() || distance.abs() < EPS || distance.abs() > 1e7 {
            return Err(tr!("core.modeling.enter_a_nonzero_offset_in_meters"));
        }
        let face = self
            .faces
            .get(&face_id)
            .ok_or(tr!("core.modeling.click_a_filled_face_first"))?;
        let plane = faces::plane(self, &face.vertices).ok_or(tr!("core.modeling.the_face_must_be_planar"))?;
        let rings: Vec<_> = std::iter::once(&face.vertices)
            .chain(face.holes.iter())
            .enumerate()
            .map(|(i, r)| {
                let points: Vec<_> = r
                    .iter()
                    .map(|id| plane.project(self.vertices[id].position))
                    .collect();
                offset_ring(&points, if i == 0 { distance } else { -distance })
            })
            .collect::<Result<_, _>>()?;
        // Reject colliding boundaries before touching the document.
        for i in 1..rings.len() {
            if !rings[i]
                .iter()
                .all(|p| faces::in_polygon(*p, &rings[0], false))
            {
                return Err(tr!("core.modeling.offset_is_too_large_for_this_face"));
            }
            for j in 0..i {
                if rings_intersect(&rings[i], &rings[j])
                    || (j > 0
                        && (faces::in_polygon(rings[i][0], &rings[j], true)
                            || faces::in_polygon(rings[j][0], &rings[i], true)))
                {
                    return Err(tr!("core.modeling.offset_boundaries_would_overlap"));
                }
            }
        }
        let mut next = self.clone();
        let color = self.face_colors.get(&face_id).copied();
        let old_faces: BTreeSet<_> = self.faces.keys().copied().collect();
        for ring in rings {
            for i in 0..ring.len() {
                let world = |p: DVec2| plane.origin + plane.u * p.x + plane.v * p.y;
                next.add_edge(world(ring[i]), world(ring[(i + 1) % ring.len()]))?;
            }
        }
        next.face_colors.retain(|id, _| next.faces.contains_key(id));
        next.reference_images.retain(|id, _| next.faces.contains_key(id));
        if let Some(color) = color {
            for id in next.faces.keys().filter(|id| !old_faces.contains(id)) {
                next.face_colors.insert(*id, color);
            }
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}

fn rings_intersect(a: &[DVec2], b: &[DVec2]) -> bool {
    (0..a.len()).any(|i| {
        (0..b.len()).any(|j| {
            crate::selection::segment_distance(
                a[i],
                a[(i + 1) % a.len()],
                b[j],
                b[(j + 1) % b.len()],
            ) < EPS
        })
    })
}
fn offset_ring(points: &[DVec2], distance: f64) -> Result<Vec<DVec2>, &'static str> {
    let area = |p: &[DVec2]| {
        (0..p.len())
            .map(|i| p[i].perp_dot(p[(i + 1) % p.len()]))
            .sum::<f64>()
            * 0.5
    };
    let sign = area(points).signum();
    let mut result = Vec::new();
    for i in 0..points.len() {
        let a = points[(i + points.len() - 1) % points.len()];
        let b = points[i];
        let c = points[(i + 1) % points.len()];
        let u = (b - a).normalize();
        let v = (c - b).normalize();
        let n = |d: DVec2| DVec2::new(-d.y, d.x) * sign * distance;
        let (p, q) = (b + n(u), b + n(v));
        let cross = u.perp_dot(v);
        let point = if cross.abs() < 1e-9 {
            if u.dot(v) < 0. {
                return Err(tr!("core.modeling.cannot_offset_a_folded_boundary"));
            }
            p
        } else {
            p + u * (q - p).perp_dot(v) / cross
        };
        if !point.is_finite() {
            return Err(tr!("core.modeling.cannot_offset_this_corner"));
        }
        result.push(point);
    }
    let invalid = tr!("core.modeling.offset_is_too_large_or_crosses_itself");
    if area(&result) * area(points) <= EPS {
        return Err(invalid);
    }
    for i in 0..result.len() {
        if (result[(i + 1) % result.len()] - result[i])
            .dot(points[(i + 1) % points.len()] - points[i])
            <= EPS
        {
            return Err(invalid);
        }
        for j in i + 1..result.len() {
            if j == i + 1 || (i == 0 && j == result.len() - 1) {
                continue;
            }
            if crate::selection::segment_distance(
                result[i],
                result[(i + 1) % result.len()],
                result[j],
                result[(j + 1) % result.len()],
            ) < EPS
            {
                return Err(invalid);
            }
        }
        if distance > 0.
            && (!faces::in_polygon(result[i], points, false)
                || (0..points.len()).any(|j| {
                    crate::selection::distance_to_segment(
                        result[i],
                        points[j],
                        points[(j + 1) % points.len()],
                    ) < distance - 1e-6
                }))
        {
            return Err(invalid);
        }
        if distance < 0. && !faces::in_polygon(points[i], &result, false) {
            return Err(invalid);
        }
    }
    Ok(result)
}

/// A reusable translation of a fixed, valid model for interactive previews.
/// Only affected geometry is checked here; commits still use `move_selection`.
pub struct MovePreview {
    vertices: Vec<(u64, DVec3)>,
    dimensions: Vec<crate::Dimension>,
    texts: Vec<crate::TextAnnotation>,
    guides: Vec<(u64, crate::ConstructionGuide)>,
    objects: Vec<(u64, crate::InstanceTransform)>,
    edges: Vec<u64>,
    faces: Vec<(u64, bool)>,
}
impl MovePreview {
    pub fn new(model: &Model, selected: &BTreeSet<u64>) -> Result<Self, &'static str> {
        if model.edit_context.is_some() {
            return Err(tr!("core.modeling.scoped_moves_require_component_synchronization"));
        }
        if model.selection_locked(selected) {
            return Err(tr!("core.modeling.unlock_the_selection_before_editing_it"));
        }
        let vertices = model.selection_vertices(selected);
        let entities = model.expand_entities(selected);
        let dimensions: Vec<_> = entities
            .iter()
            .filter_map(|id| model.dimensions.get(id))
            .map(|d| {
                let mut d = d.clone();
                let (a, b) = d.points(model);
                d.start.point = a;
                d.end.point = b;
                if let Some(vertex) = &mut d.vertex { vertex.point = vertex.position(model); }
                for anchor in d.anchors_mut() {
                    if anchor.weights.iter().any(|(id, _)| !vertices.contains(id)) {
                        anchor.weights.clear();
                    }
                }
                d
            })
            .collect();
        let texts: Vec<_> = entities
            .iter()
            .filter_map(|id| model.texts.get(id))
            .cloned()
            .collect();
        let guides: Vec<_> = entities
            .iter()
            .filter_map(|id| model.guides.get(id).map(|g| (*id, g.clone())))
            .collect();
        if vertices.is_empty() && dimensions.is_empty() && texts.is_empty() && guides.is_empty() {
            return Err(tr!("core.modeling.select_geometry_a_dimension_text_or_a_guide"));
        }
        let protected = model
            .edges
            .keys()
            .chain(model.faces.keys())
            .copied()
            .filter(|id| model.is_locked(*id))
            .collect();
        if !vertices.is_disjoint(&model.selection_vertices(&protected)) {
            return Err(tr!("core.modeling.unlock_the_connected_geometry_before_editing_it"));
        }
        let edges = model
            .edges
            .values()
            .filter(|e| vertices.contains(&e.a) || vertices.contains(&e.b))
            .map(|e| e.id)
            .collect();
        let faces = model
            .faces
            .values()
            .filter_map(|face| {
                let mut ids = face.vertices.iter().chain(face.holes.iter().flatten());
                let affected = ids.clone().any(|id| vertices.contains(id));
                affected.then(|| (face.id, !ids.all(|id| vertices.contains(id))))
            })
            .collect();
        let objects = entities
            .iter()
            .filter_map(|id| model.objects.get(id))
            .map(|o| (o.id, o.transform.unwrap_or_default()))
            .collect();
        Ok(Self {
            vertices: vertices
                .into_iter()
                .map(|id| (id, model.vertices[&id].position))
                .collect(),
            dimensions,
            texts,
            guides,
            objects,
            edges,
            faces,
        })
    }
    /// `preview` is a clone of the source passed to `new`, reused between frames.
    pub fn update(&self, preview: &mut Model, delta: DVec3) -> Result<(), &'static str> {
        let invalid: &str =
            tr!("core.modeling.this_change_would_collapse_an_edge_or_bend");
        if !delta.is_finite() {
            return Err(tr!("core.modeling.enter_a_finite_displacement"));
        }
        for &(id, point) in &self.vertices {
            let point = point + delta;
            if !point.is_finite() || point.abs().max_element() > 1e10 {
                return Err(invalid);
            }
            preview.vertices.get_mut(&id).unwrap().position = point;
        }
        for source in &self.dimensions {
            let d = preview.dimensions.get_mut(&source.id).unwrap();
            d.clone_from(source);
            d.start.point += delta;
            d.end.point += delta;
            if let Some(vertex) = &mut d.vertex { vertex.point += delta; }
            d.offset = (source.start.point + source.offset + delta) - (source.start.point + delta);
        }
        for source in &self.texts {
            let text = preview.texts.get_mut(&source.id).unwrap();
            text.clone_from(source);
            if source.anchor.weights.is_empty() {
                text.anchor.point += delta;
            } else if !source
                .anchor
                .weights
                .iter()
                .all(|(id, _)| self.vertices.iter().any(|(v, _)| v == id))
            {
                text.offset += delta;
            }
        }
        for (id, source) in &self.guides {
            let mut guide = source.clone();
            guide.origin += delta;
            if !guide.valid() {
                return Err(invalid);
            }
            preview.guides.insert(*id, guide);
        }
        for &(id, frame) in &self.objects {
            preview.objects.get_mut(&id).unwrap().transform = Some(frame.translated(delta));
        }
        preview.sync_dimensions();
        if self
            .edges
            .iter()
            .any(|id| preview.edge_length(&preview.edges[id]) < EPS)
        {
            return Err(invalid);
        }
        for &(id, partial) in &self.faces {
            let face = &preview.faces[&id];
            if std::iter::once(&face.vertices)
                .chain(face.holes.iter())
                .any(|ring| faces::plane(preview, ring).is_none())
                || (partial && preview.face_area(face) <= EPS)
            {
                return Err(invalid);
            }
        }
        preview.validate_dimensions().map_err(|_| invalid)?;
        preview.validate_texts().map_err(|_| invalid)?;
        Ok(())
    }
}

/// Reuses source coordinates and a single destination model while scaling.
/// Topology and lock dependencies are collected once; commit still performs
/// the complete validated operation, including component synchronization.
pub struct ScalePreview {
    affected: MovePreview,
    texts: Vec<([DVec3; 2], bool)>,
    areas: BTreeMap<u64, DVec3>,
    gpu_bounds: Option<(DVec3, DVec3)>,
    min_edge: f64,
    min_altitude: f64,
    min_area: f64,
}
impl ScalePreview {
    pub fn new(model: &Model, selected: &BTreeSet<u64>) -> Result<Self, &'static str> {
        let affected = MovePreview::new(model, selected)?;
        let vertices: BTreeSet<_> = affected.vertices.iter().map(|(id, _)| *id).collect();
        let texts = affected
            .texts
            .iter()
            .map(|text| {
                (
                    text.layout(model),
                    text.anchor.weights.is_empty()
                        || text
                            .anchor
                            .weights
                            .iter()
                            .all(|(id, _)| vertices.contains(id)),
                )
            })
            .collect();
        let areas: BTreeMap<u64, DVec3> = affected
            .faces
            .iter()
            .filter(|(_, partial)| !partial)
            .map(|(id, _)| {
                let face = &model.faces[id];
                let normal = faces::plane(model, &face.vertices).unwrap().normal;
                (*id, normal * model.face_area(face))
            })
            .collect();
        let min_area = areas.values().map(|area| area.length()).fold(f64::INFINITY, f64::min);
        let entities = model.expand_entities(selected);
        // The shader transforms complete selected primitives. Shared vertices,
        // scoped definitions and attached annotations need the CPU edit path.
        let gpu_bounds = (model.dimensions.is_empty() && model.texts.is_empty()
            && model.connections.is_empty() && model.reference_images.is_empty()
            && affected.guides.is_empty()
            && affected.edges.iter().all(|id| entities.contains(id))
            && affected.faces.iter().all(|(id, partial)| !partial && entities.contains(id)))
            .then(|| model.selection_bounds(selected)).flatten();
        let min_edge = affected.edges.iter().map(|id| model.edge_length(&model.edges[id]))
            .fold(f64::INFINITY, f64::min);
        let min_altitude = affected.faces.iter().flat_map(|(id, _)| {
            let face = &model.faces[id];
            std::iter::once(&face.vertices).chain(face.holes.iter()).map(|ring| {
                let origin = model.vertices[&ring[0]].position;
                let u = (model.vertices[&ring[1]].position - origin).normalize();
                ring.iter().skip(2).map(|id| u.cross(model.vertices[id].position - origin).length())
                    .fold(0., f64::max)
            })
        }).fold(f64::INFINITY, f64::min);
        Ok(Self {
            affected,
            texts,
            areas,
            gpu_bounds,
            min_edge,
            min_altitude,
            min_area,
        })
    }

    /// Conservative, constant-time eligibility for a GPU-only preview. A
    /// positive diagonal transform preserves complete faces; bounds and lower
    /// singular-value limits reject numeric collapse before touching buffers.
    pub fn gpu_factors(&self, center: DVec3, factor: f64,
        constraint: Option<crate::inference::AxisConstraint>) -> Option<DVec3> {
        let (lo, hi) = self.gpu_bounds?;
        if !factor.is_finite() || factor <= EPS || factor > 1e7 { return None; }
        let factors = constraint.map_or(DVec3::splat(factor), |lock|
            DVec3::ONE + lock.project(DVec3::ONE) * (factor - 1.));
        let min = factors.min_element();
        if self.min_edge * min < EPS * 2.
            || self.min_area * min * min <= EPS * EPS * 2.
            || self.min_altitude * min * min / factors.max_element() <= faces::TOL * 2. {
            return None;
        }
        [lo, hi].into_iter().all(|point| {
            let point = center + (point - center) * factors;
            point.is_finite() && point.abs().max_element() <= 1e10
        }).then_some(factors)
    }

    /// The destination starts as a clone of the source and is reused. Invalid
    /// previews may be partially updated and must not be displayed or committed.
    pub fn update(
        &self,
        preview: &mut Model,
        center: DVec3,
        factor: f64,
        constraint: Option<crate::inference::AxisConstraint>,
    ) -> Result<(), &'static str> {
        let invalid: &str = tr!("core.modeling.this_change_would_collapse_an_edge_or_bend");
        if !factor.is_finite() || factor <= EPS || factor > 1e7 {
            return Err(tr!("core.modeling.enter_a_positive_scale_factor_such_as_2"));
        }
        let factors = constraint.map_or(DVec3::splat(factor), |lock| {
            DVec3::ONE + lock.project(DVec3::ONE) * (factor - 1.)
        });
        let scale = |point: DVec3| center + (point - center) * factors;
        for &(id, point) in &self.affected.vertices {
            let point = scale(point);
            if !point.is_finite() || point.abs().max_element() > 1e10 {
                return Err(invalid);
            }
            preview.vertices.get_mut(&id).unwrap().position = point;
        }
        for source in &self.affected.dimensions {
            let dimension = preview.dimensions.get_mut(&source.id).unwrap();
            dimension.clone_from(source);
            dimension.start.point = scale(source.start.point);
            dimension.end.point = scale(source.end.point);
            if let Some(vertex) = &mut dimension.vertex { vertex.point = scale(source.vertex.as_ref().unwrap().point); }
            dimension.offset =
                scale(source.start.point + source.offset) - scale(source.start.point);
        }
        for (source, &(points, attached)) in self.affected.texts.iter().zip(&self.texts) {
            let text = preview.texts.get_mut(&source.id).unwrap();
            text.clone_from(source);
            text.anchor.point = if attached {
                scale(points[0])
            } else {
                points[0]
            };
            text.offset = scale(points[1]) - text.anchor.point;
        }
        for (id, source) in &self.affected.guides {
            let mut guide = source.clone();
            guide.origin = scale(source.origin);
            guide.direction = source
                .direction
                .map(|direction| {
                    (scale(source.origin + direction) - guide.origin)
                        .try_normalize()
                        .ok_or(invalid)
                })
                .transpose()?;
            if !guide.valid() {
                return Err(invalid);
            }
            preview.guides.insert(*id, guide);
        }
        for &(id, frame) in &self.affected.objects {
            let scaled = if constraint.is_none() {
                frame.scaled(center, factor)
            } else {
                let mut scaled = frame;
                scaled.translation = scale(frame.translation);
                scaled.basis = glam::DMat3::from_diagonal(factors)
                    * frame.basis
                    * glam::DMat3::from_diagonal(DVec3::ONE / factors);
                scaled.axes *= factors;
                scaled
            };
            if !scaled.valid() {
                return Err(invalid);
            }
            preview.objects.get_mut(&id).unwrap().transform = Some(scaled);
        }
        preview.sync_dimensions();
        if self
            .affected
            .edges
            .iter()
            .any(|id| preview.edge_length(&preview.edges[id]) < EPS)
        {
            return Err(invalid);
        }
        for &(id, partial) in &self.affected.faces {
            let face = &preview.faces[&id];
            if std::iter::once(&face.vertices)
                .chain(face.holes.iter())
                .any(|ring| faces::plane(preview, ring).is_none())
            {
                return Err(invalid);
            }
            // A complete face is an affine transform of its source. Its area
            // follows the transformed area vector, with no triangulation per tick.
            let area = if partial {
                preview.face_area(face)
            } else {
                (self.areas[&id] / factors * factors.x * factors.y * factors.z).length()
            };
            if area <= EPS * EPS {
                return Err(invalid);
            }
        }
        preview.validate_dimensions().map_err(|_| invalid)?;
        preview.validate_texts().map_err(|_| invalid)?;
        Ok(())
    }
}
