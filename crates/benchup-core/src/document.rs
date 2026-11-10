use crate::Camera;
pub use crate::history::History;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// PHP's associative JSON roundtrip historically encoded empty objects as [].
/// Accept that one legacy representation only on map fields; real sequences
/// are still rejected, and serialization always emits a JSON object.
pub(crate) fn deserialize_map_or_empty_sequence<'de, D, K, V>(
    deserializer: D,
) -> Result<BTreeMap<K, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    struct MapVisitor<K, V>(std::marker::PhantomData<(K, V)>);
    impl<'de, K: Deserialize<'de> + Ord, V: Deserialize<'de>> serde::de::Visitor<'de>
        for MapVisitor<K, V>
    {
        type Value = BTreeMap<K, V>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(tr!("core.document.a_map_or_an_empty_legacy_array"))
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
            Deserialize::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
        fn visit_seq<S: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: S,
        ) -> Result<Self::Value, S::Error> {
            if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom(
                    tr!("core.document.expected_a_map_only_empty_legacy_arrays_are"),
                ));
            }
            Ok(BTreeMap::new())
        }
    }
    deserializer.deserialize_any(MapVisitor::<K, V>(std::marker::PhantomData))
}

pub const EPS: f64 = 1.0e-7;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub id: u64,
    pub position: DVec3,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: u64,
    pub a: u64,
    pub b: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub id: u64,
    pub vertices: Vec<u64>,
    #[serde(default)]
    pub holes: Vec<Vec<u64>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Model {
    #[serde(default, deserialize_with = "deserialize_map_or_empty_sequence")]
    pub guides: BTreeMap<u64, crate::ConstructionGuide>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub connections: BTreeMap<u64, crate::connections::Connection>,
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub vertices: BTreeMap<u64, Vertex>,
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub edges: BTreeMap<u64, Edge>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub faces: BTreeMap<u64, Face>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub face_colors: BTreeMap<u64, [u8; 3]>,
    #[serde(default, deserialize_with = "deserialize_map_or_empty_sequence")]
    pub reference_images: BTreeMap<u64, crate::ReferenceImage>,
    #[serde(default, deserialize_with = "deserialize_map_or_empty_sequence")]
    pub image_assets: BTreeMap<crate::reference_images::ImageAssetId, crate::ImageAsset>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub dimensions: BTreeMap<u64, crate::Dimension>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub texts: BTreeMap<u64, crate::TextAnnotation>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub objects: BTreeMap<u64, crate::Object>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub stock_materials: BTreeMap<crate::MaterialId, crate::StockMaterial>,
    #[serde(default)]
    pub hidden: BTreeSet<u64>,
    #[serde(default)]
    pub locked: BTreeSet<u64>,
    #[serde(skip)]
    pub edit_context: Option<u64>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub tags: BTreeMap<u64, crate::Tag>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub entity_tags: BTreeMap<u64, u64>,
    pub(crate) next_id: u64,
}

impl Model {
    /// Editing scope changes interaction, not document geometry or appearance.
    pub fn same_scene(&self, other: &Self) -> bool {
        self.reference_images == other.reference_images && self.same_scene_geometry(other)
    }
    /// Retained scene geometry excludes image appearance uniforms. Image
    /// rendering reads current settings directly from the document, while
    /// geometry/measurement snapshots may retain earlier settings.
    pub fn same_scene_geometry(&self, other: &Self) -> bool {
        self.guides == other.guides
            && self.connections == other.connections
            && self.vertices == other.vertices
            && self.edges == other.edges
            && self.faces == other.faces
            && self.image_assets == other.image_assets
            && self.reference_images.len() == other.reference_images.len()
            && self.reference_images.iter().all(|(id, image)| {
                other.reference_images.get(id).is_some_and(|other| {
                    image.asset == other.asset
                        && image.vertices == other.vertices
                        && image.name == other.name
                        && image.placement_offset == other.placement_offset
                })
            })
            && self.face_colors == other.face_colors
            && self.dimensions == other.dimensions
            && self.texts == other.texts
            && self.objects == other.objects
            && self.stock_materials == other.stock_materials
            && self.hidden == other.hidden
            && self.locked == other.locked
            && self.tags == other.tags
            && self.entity_tags == other.entity_tags
            && self.next_id == other.next_id
    }
    pub(crate) fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    pub fn points(&self, edge: &Edge) -> (DVec3, DVec3) {
        (
            self.vertices[&edge.a].position,
            self.vertices[&edge.b].position,
        )
    }
    pub fn edge_length(&self, edge: &Edge) -> f64 {
        let (a, b) = self.points(edge);
        a.distance(b)
    }
    fn vertex_at(&mut self, position: DVec3, drawing_vertices: &BTreeSet<u64>) -> u64 {
        if let Some(v) = self
            .vertices
            .values()
            .find(|v| drawing_vertices.contains(&v.id) && v.position.distance(position) < EPS)
        {
            return v.id;
        }
        let id = self.id();
        self.vertices.insert(id, Vertex { id, position });
        id
    }
    pub(crate) fn insert_edge(&mut self, a: u64, b: u64) -> u64 {
        if let Some(e) = self
            .edges
            .values()
            .find(|e| (e.a == a && e.b == b) || (e.a == b && e.b == a))
        {
            return e.id;
        }
        let id = self.id();
        self.edges.insert(id, Edge { id, a, b });
        id
    }
    fn split_at(&mut self, vertex: u64, drawing_edges: &mut BTreeSet<u64>) {
        let point = self.vertices[&vertex].position;
        let splits: Vec<_> = self
            .edges
            .values()
            .filter(|e| drawing_edges.contains(&e.id))
            .filter(|e| e.a != vertex && e.b != vertex)
            .filter_map(|e| {
                let (a, b) = self.points(e);
                let t = (point - a).dot(b - a) / a.distance_squared(b);
                (t > EPS && t < 1.0 - EPS && (a + (b - a) * t).distance(point) < EPS)
                    .then_some(e.clone())
            })
            .collect();
        for edge in splits {
            for face in self.faces.values_mut() {
                for ring in std::iter::once(&mut face.vertices).chain(face.holes.iter_mut()) {
                    if let Some(i) = (0..ring.len()).find(|&i| {
                        let a = ring[i];
                        let b = ring[(i + 1) % ring.len()];
                        (a == edge.a && b == edge.b) || (a == edge.b && b == edge.a)
                    }) {
                        ring.insert(i + 1, vertex);
                    }
                }
            }
            self.edges.remove(&edge.id);
            drawing_edges.remove(&edge.id);
            drawing_edges.insert(self.insert_edge(edge.a, vertex));
            drawing_edges.insert(self.insert_edge(vertex, edge.b));
        }
    }
    pub fn add_edge(&mut self, start: DVec3, end: DVec3) -> Result<u64, &'static str> {
        self.draw_edge(start, end, false)
    }
    /// Draw an edge, accepting a boundary already present in the drawing.
    pub fn ensure_edge(&mut self, start: DVec3, end: DVec3) -> Result<u64, &'static str> {
        self.draw_edge(start, end, true)
    }
    /// Commit a connected drawing in one scoped transaction. A rectangle must
    /// not copy, replace and synchronize the same assembly four times.
    pub fn draw_polyline(
        &mut self,
        points: &[DVec3],
        allow_existing: bool,
    ) -> Result<(), &'static str> {
        if self.edit_context.is_some() {
            if self.try_draw_scoped_geometry(points, allow_existing)? {
                return Ok(());
            }
            return self.edit_in_context(|model| model.draw_polyline(points, allow_existing));
        }
        for pair in points.windows(2) {
            self.draw_edge(pair[0], pair[1], allow_existing)?;
        }
        Ok(())
    }
    fn draw_edge(
        &mut self,
        start: DVec3,
        end: DVec3,
        allow_existing: bool,
    ) -> Result<u64, &'static str> {
        if self.edit_context.is_some() {
            return self.edit_in_context(|model| model.draw_edge(start, end, allow_existing));
        }
        if !start.is_finite() || !end.is_finite() {
            return Err(tr!("core.document.coordinates_must_be_finite"));
        }
        // Keep newly drawn geometry within the same limits enforced on reload.
        // Reject before allocating vertices or splitting an existing edge.
        if start.abs().max_element() > 1e10 || end.abs().max_element() > 1e10 {
            return Err(tr!("core.document.coordinates_must_be_within_ten_billion_meters_of"));
        }
        if start.distance(end) < EPS {
            return Err(tr!("core.document.give_your_line_a_little_length"));
        }
        let mut drawing_edges = self.drawing_edges();
        if let Some(edge) = self
            .edges
            .values()
            .filter(|e| drawing_edges.contains(&e.id))
            .find(|e| {
                let (a, b) = self.points(e);
                (a.distance(start) < EPS && b.distance(end) < EPS)
                    || (a.distance(end) < EPS && b.distance(start) < EPS)
            })
            .cloned()
        {
            // Retracing an existing boundary heals an intentionally erased face.
            if self.create_faces_with_edges(edge.a, edge.b, &drawing_edges) || allow_existing {
                return Ok(edge.id);
            }
            return Err(tr!("core.document.that_edge_already_exists"));
        }
        let drawing_vertices = drawing_edges
            .iter()
            .flat_map(|id| {
                let edge = &self.edges[id];
                [edge.a, edge.b]
            })
            .collect();
        let a = self.vertex_at(start, &drawing_vertices);
        let b = self.vertex_at(end, &drawing_vertices);
        self.split_at(a, &mut drawing_edges);
        self.split_at(b, &mut drawing_edges);
        let id = self.insert_edge(a, b);
        drawing_edges.insert(id);
        self.create_faces_with_edges(a, b, &drawing_edges);
        Ok(id)
    }
    pub fn erase(&mut self, ids: &BTreeSet<u64>) {
        if let Some(context) = self.edit_context {
            // Removing geometry from ordinary groups cannot change a component
            // definition. Keep this local edit off the full-model transactional
            // copy, component propagation and validation used for other edits.
            if self
                .edit_path()
                .iter()
                .all(|id| matches!(self.objects[id].kind, crate::ObjectKind::Group))
            {
                if self.is_locked(context) {
                    return;
                }
                self.erase_in_scope(ids);
                return;
            }
            let _ = self.edit_in_context(|model| {
                model.erase(ids);
                Ok(())
            });
            return;
        }
        self.erase_in_scope(ids);
    }
    fn erase_in_scope(&mut self, ids: &BTreeSet<u64>) {
        self.sync_dimensions();
        let ids = self.erasable_entities(ids);
        if ids.is_empty() {
            return;
        }
        let vertices: std::collections::HashSet<_> = ids
            .iter()
            .filter_map(|id| self.edges.get(id))
            .flat_map(|edge| [edge.a, edge.b])
            .collect();
        self.dimensions.retain(|id, _| !ids.contains(id));
        self.texts.retain(|id, _| !ids.contains(id));
        self.guides.retain(|id, _| !ids.contains(id));
        let removed: BTreeSet<_> = self
            .edges
            .values()
            .filter(|e| ids.contains(&e.id))
            .map(|e| (e.a.min(e.b), e.a.max(e.b)))
            .collect();
        let removed_faces: BTreeSet<_> = self
            .faces
            .iter()
            .filter_map(|(&id, face)| {
                (ids.contains(&id)
                    || (std::iter::once(&face.vertices)
                        .chain(face.holes.iter())
                        .any(|ring| {
                            (0..ring.len()).any(|i| {
                                let a = ring[i];
                                let b = ring[(i + 1) % ring.len()];
                                removed.contains(&(a.min(b), a.max(b)))
                            })
                        })
                        && (self.edit_context.is_none() || self.entity_in_context(id).is_some())))
                .then_some(id)
            })
            .collect();
        for id in &removed_faces {
            self.faces.remove(id);
            self.face_colors.remove(id);
        }
        self.face_colors.retain(|id, _| self.faces.contains_key(id));
        self.reference_images.retain(|id, _| self.faces.contains_key(id));
        self.edges.retain(|id, _| !ids.contains(id));
        let used: std::collections::HashSet<_> = self
            .edges
            .values()
            .flat_map(|e| [e.a, e.b])
            .filter(|id| vertices.contains(id))
            .collect();
        for id in vertices.difference(&used) {
            self.vertices.remove(id);
        }
        self.sync_dimensions();
        self.objects.retain(|id, _| !ids.contains(id));
        let removed: BTreeSet<_> = ids.union(&removed_faces).copied().collect();
        for id in &removed {
            self.entity_tags.remove(id);
            self.hidden.remove(id);
            self.locked.remove(id);
        }
        if removed.len() <= 512 {
            for object in self.objects.values_mut() {
                for id in &removed {
                    object.entities.remove(id);
                }
            }
        } else {
            for object in self.objects.values_mut() {
                object.entities.retain(|id| !removed.contains(id));
            }
        }
    }
    pub fn bounds(&self) -> Option<(DVec3, DVec3)> {
        // Expand inherited visibility once; walking every object's membership
        // for each edge becomes quadratic on imported assemblies.
        let mut hidden_roots = self.hidden.clone();
        hidden_roots.extend(self.entity_tags.iter().filter_map(|(entity, tag)| {
            self.tags
                .get(tag)
                .is_some_and(|tag| !tag.visible)
                .then_some(*entity)
        }));
        let hidden = self.expand_entities(&hidden_roots);
        let used: BTreeSet<_> = self
            .edges
            .values()
            .filter(|e| !hidden.contains(&e.id))
            .flat_map(|e| [e.a, e.b])
            .chain(
                self.faces
                    .values()
                    .filter(|f| !hidden.contains(&f.id))
                    .flat_map(|f| f.vertices.iter().chain(f.holes.iter().flatten()).copied()),
            )
            .collect();
        let vertices = used.iter().map(|id| self.vertices[id].position).chain(
            self.dimensions
                .values()
                .filter(|d| !hidden.contains(&d.id))
                .flat_map(|d| d.layout(self)),
        );
        let vertices = vertices.chain(
            self.texts
                .values()
                .filter(|t| !hidden.contains(&t.id))
                .flat_map(|t| t.layout(self)),
        );
        let mut vertices = vertices.chain(
            self.guides
                .iter()
                .filter(|(id, _)| !hidden.contains(id))
                .map(|(_, guide)| guide.origin),
        );
        let first = vertices.next()?;
        Some(vertices.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.connections.len() > 10000
            || self.vertices.len() > 1_000_000
            || self.edges.len() > 1_000_000
            || self.faces.len() > 1_000_000
            || self.objects.len() > 1_000_000
            || self.dimensions.len() > 1_000_000
            || self.texts.len() > 1_000_000
            || self.guides.len() > 1_000_000
        {
            return Err(tr!("core.document.model_is_too_large"));
        }
        if self.connections.values().any(|c| match &c.operation {
            crate::connections::ConnectionOperation::Fasteners(s) => !s.valid(),
            crate::connections::ConnectionOperation::Joinery(s) => !s.valid(),
        }) {
            return Err(tr!("core.document.invalid_connection_parameters"));
        }
        let max_id = self
            .vertices
            .keys()
            .chain(self.edges.keys())
            .chain(self.faces.keys())
            .chain(self.objects.keys())
            .chain(self.dimensions.keys())
            .chain(self.texts.keys())
            .chain(self.connections.keys())
            .chain(self.guides.keys())
            .max()
            .copied()
            .unwrap_or(0);
        if self.next_id < max_id || self.next_id > u64::MAX - 2_000_001 {
            return Err(tr!("core.document.invalid_model_ids"));
        }
        for (id, v) in &self.vertices {
            if *id != v.id || !v.position.is_finite() || v.position.abs().max_element() > 1e10 {
                return Err(tr!("core.document.invalid_vertex"));
            }
        }
        if self.guides.values().any(|guide| !guide.valid()) {
            return Err(tr!("core.document.invalid_construction_guide"));
        }
        let mut pairs = BTreeSet::new();
        for (id, e) in &self.edges {
            if *id != e.id
                || e.a == e.b
                || !self.vertices.contains_key(&e.a)
                || !self.vertices.contains_key(&e.b)
            {
                return Err(tr!("core.document.invalid_edge"));
            }
            if self.edge_length(e) < EPS || !pairs.insert((e.a.min(e.b), e.a.max(e.b))) {
                return Err(tr!("core.document.invalid_edge"));
            }
        }
        let mut boundaries = BTreeSet::new();
        for (id, face) in &self.faces {
            if *id != face.id
                || self.vertices.contains_key(id)
                || self.edges.contains_key(id)
                || !boundaries.insert(crate::faces::canonical_ring(&face.vertices))
            {
                return Err(tr!("core.document.invalid_face_id"));
            }
            for ring in std::iter::once(&face.vertices).chain(face.holes.iter()) {
                if ring.len() < 3
                    || ring.iter().any(|v| !self.vertices.contains_key(v))
                    || crate::faces::plane(self, ring).is_none()
                {
                    return Err(tr!("core.document.invalid_face_boundary"));
                }
                for i in 0..ring.len() {
                    let a = ring[i];
                    let b = ring[(i + 1) % ring.len()];
                    if !pairs.contains(&(a.min(b), a.max(b))) {
                        return Err(tr!("core.document.face_boundary_is_missing_an_edge"));
                    }
                }
            }
            let triangles = self.face_triangles(face);
            let area: f64 = triangles
                .iter()
                .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() * 0.5)
                .sum();
            // EPS is a distance in metres; surface area is in square metres.
            if triangles.is_empty() || area <= EPS * EPS {
                return Err(tr!("core.document.invalid_face_surface"));
            }
        }
        self.validate_tags()?;
        self.validate_objects()?;
        self.validate_dimensions()?;
        self.validate_texts()?;
        self.validate_reference_images()?;
        if self
            .face_colors
            .keys()
            .any(|id| !self.faces.contains_key(id))
        {
            return Err(tr!("core.document.paint_refers_to_a_missing_face"));
        }
        Ok(())
    }
    pub fn contains_entity(&self, id: &u64) -> bool {
        self.edges.contains_key(id)
            || self.faces.contains_key(id)
            || self.objects.contains_key(id)
            || self.dimensions.contains_key(id)
            || self.texts.contains_key(id)
            || self.guides.contains_key(id)
    }
}

/// Public linkage only. Credentials and selection leases are never stored in a project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SharedProject {
    pub server: String,
    pub id: String,
    pub revision: u64,
    pub server_updated_at: u64,
    #[serde(default)]
    pub file_updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_project: Option<SharedProject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_author: Option<crate::HistoryAuthor>,
    /// Assigned when tracking shared material usage; absent in older projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<uuid::Uuid>,
    pub version: u32,
    pub name: String,
    pub model: Model,
    #[serde(
        default,
        deserialize_with = "crate::history::deserialize_history",
        skip_serializing_if = "History::is_empty"
    )]
    pub history: History,
    pub camera: Camera,
    /// Per-viewport framing belongs to this project, unlike reusable layout preferences.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[serde(deserialize_with = "deserialize_map_or_empty_sequence")]
    pub viewport_cameras: BTreeMap<u64, Camera>,
    /// Selection belongs to this project, including the object open for editing.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub selection: BTreeSet<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_context: Option<u64>,
    #[serde(default = "enabled")]
    pub snap_to_geometry: bool,
    #[serde(default = "enabled")]
    pub snap_to_axes: bool,
    #[serde(default)]
    pub display: crate::DisplaySettings,
}
fn enabled() -> bool {
    true
}
impl Default for Document {
    fn default() -> Self {
        Self {
            project_id: None,
            shared_project: None,
            project_author: None,
            version: 2,
            name: tr!("core.document.untitled_model").into(),
            model: Model::default(),
            history: History::default(),
            camera: Camera::default(),
            viewport_cameras: BTreeMap::new(),
            selection: BTreeSet::new(),
            selection_context: None,
            snap_to_geometry: true,
            snap_to_axes: true,
            display: crate::DisplaySettings::default(),
        }
    }
}
impl Document {
    /// A durable edit checkpoint avoids serializing all unchanged geometry and
    /// the original import in history on every mouse click.
    pub fn checkpoint(&self, base_model: &Model, base_history: &History) -> DocumentCheckpoint {
        let metadata = Self {
            shared_project: self.shared_project.clone(),
            project_author: self.project_author.clone(),
            project_id: self.project_id,
            version: self.version,
            name: self.name.clone(),
            model: Model::default(),
            history: History::default(),
            camera: self.camera.clone(),
            viewport_cameras: self.viewport_cameras.clone(),
            selection: self.selection.clone(),
            selection_context: self.selection_context,
            snap_to_geometry: self.snap_to_geometry,
            snap_to_axes: self.snap_to_axes,
            display: self.display.clone(),
        };
        DocumentCheckpoint {
            metadata,
            model: crate::history::ModelChange::between(base_model, &self.model),
            history: crate::history::Checkpoint::between(base_history, &self.history),
        }
    }
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut doc: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        doc.restore_loaded()?;
        Ok(doc)
    }
    fn restore_loaded(&mut self) -> Result<(), String> {
        let doc = self;
        if doc.version != 1 && doc.version != 2 {
            return Err(tr!("core.document.unsupported_document_version").into());
        }
        doc.model.validate().map_err(str::to_owned)?;
        if doc.version == 1 {
            let edges: Vec<_> = doc.model.edges.values().map(|e| (e.a, e.b)).collect();
            for (a, b) in edges {
                doc.model.create_faces(a, b);
            }
            doc.version = 2;
        }
        doc.model.initialize_instance_transforms();
        doc.history.restore(&doc.model);
        if !doc.display.valid() {
            return Err(tr!("core.document.invalid_display_settings").into());
        }
        if !doc.camera.valid() || doc.viewport_cameras.values().any(|camera| !camera.valid()) {
            return Err(tr!("core.document.invalid_camera").into());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct DocumentCheckpoint {
    metadata: Document,
    model: crate::history::ModelChange,
    history: crate::history::Checkpoint,
}
impl DocumentCheckpoint {
    pub fn restore(self, base_model: &Model, base_history: &History) -> Result<Document, String> {
        let mut doc = self.metadata;
        doc.model = base_model.clone();
        if !self.model.apply_checked(&mut doc.model, true) {
            return Err("Autosave edits do not match the saved project.".into());
        }
        doc.history = self
            .history
            .restore(base_history)
            .ok_or("Autosave history does not match the saved project.")?;
        doc.restore_loaded()?;
        Ok(doc)
    }
}

#[cfg(test)]
mod legacy_json_map_tests {
    use super::*;
    #[test]
    fn php_empty_maps_load_and_are_saved_as_objects() {
        let mut json = serde_json::to_value(Document::default()).unwrap();
        for field in [
            "connections",
            "vertices",
            "edges",
            "faces",
            "face_colors",
            "reference_images",
            "image_assets",
            "dimensions",
            "texts",
            "objects",
            "stock_materials",
            "tags",
            "entity_tags",
        ] {
            json["model"][field] = serde_json::json!([]);
        }
        json["viewport_cameras"] = serde_json::json!([]);
        let loaded = Document::from_json(&json.to_string()).unwrap();
        let saved = serde_json::to_value(loaded).unwrap();
        for field in [
            "connections",
            "vertices",
            "edges",
            "faces",
            "face_colors",
            "reference_images",
            "image_assets",
            "dimensions",
            "texts",
            "objects",
            "stock_materials",
            "tags",
            "entity_tags",
        ] {
            assert!(saved["model"][field].is_object(), "{field}");
        }
        assert!(saved["model"]["hidden"].is_array());
        assert!(saved["model"]["locked"].is_array());
    }
    #[test]
    fn legacy_definition_maps_preserve_groups_and_undo_history() {
        let mut doc = Document::default();
        let edge = doc.model.add_edge(DVec3::ZERO, DVec3::X).unwrap();
        let before = doc.model.clone();
        let group = doc
            .model
            .make_object(&BTreeSet::from([edge]), false, "Group")
            .unwrap();
        doc.history.record("Group", before, &doc.model);
        let mut json = serde_json::to_value(&doc).unwrap();
        json["model"]["objects"][group.to_string()]["definition_entities"] = serde_json::json!([]);
        json["history"]["undo"][0]["change"]["objects"][0][2]["definition_entities"] =
            serde_json::json!([]);
        let mut loaded = Document::from_json(&json.to_string()).unwrap();
        assert_eq!(loaded.history.undo_label(), Some("Group"));
        assert!(loaded.history.undo(&mut loaded.model));
        assert!(loaded.model.objects.is_empty());
        assert!(loaded.history.redo(&mut loaded.model));
        assert!(loaded.model.objects.contains_key(&group));
    }
    #[test]
    fn nonempty_sequences_are_not_silently_treated_as_maps() {
        let mut json = serde_json::to_value(Document::default()).unwrap();
        json["model"]["vertices"] = serde_json::json!([{"id":1,"position":[0,0,0]}]);
        assert!(Document::from_json(&json.to_string()).is_err());
    }
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;
    #[test]
    fn small_checkpoints_restore_metadata_history_undo_and_branches() {
        let mut base = Document::default();
        let empty = base.model.clone();
        for i in 0..256 {
            base.model
                .add_edge(DVec3::X * i as f64, DVec3::X * i as f64 + DVec3::Y)
                .unwrap();
        }
        base.history.record("Import", empty, &base.model);
        let mut doc = base.clone();
        for action in 0..4 {
            let before = doc.model.clone();
            doc.model
                .add_edge(
                    DVec3::Z * (action + 1) as f64,
                    DVec3::Z * (action + 1) as f64 + DVec3::X,
                )
                .unwrap();
            doc.history.record("Draw", before, &doc.model);
            if action == 1 {
                assert!(doc.history.undo(&mut doc.model));
            }
            doc.name = format!("Edited {action}");
            doc.selection = doc
                .model
                .edges
                .keys()
                .next_back()
                .copied()
                .into_iter()
                .collect();
            doc.snap_to_geometry = false;
            let encoded =
                serde_json::to_string(&doc.checkpoint(&base.model, &base.history)).unwrap();
            assert!(encoded.len() < serde_json::to_string(&doc).unwrap().len() / 3);
            let saved: DocumentCheckpoint = serde_json::from_str(&encoded).unwrap();
            let restored = saved.restore(&base.model, &base.history).unwrap();
            assert_eq!(doc, restored);
        }
        let mut wrong = base.model.clone();
        wrong.vertices.values_mut().next().unwrap().position.x += 1.;
        // A checkpoint that changes this vertex must reject an incompatible base.
        doc.model.vertices.values_mut().next().unwrap().position.x += 2.;
        assert!(
            doc.checkpoint(&base.model, &base.history)
                .restore(&wrong, &base.history)
                .is_err()
        );
    }
}
