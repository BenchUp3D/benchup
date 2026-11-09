//! Exact inspector measurements run off the UI thread. Cached geometry is
//! shared across editing scopes; stale results never reach another project.
use super::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct Summary {
    pub edges: usize,
    pub faces: usize,
    pub dimensions: usize,
    pub texts: usize,
    pub guides: usize,
    pub length: f64,
    pub area: f64,
    pub perimeter: f64,
    pub sides: Vec<Vec<f64>>,
    pub size: Option<DVec3>,
    pub volume: Option<f64>,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    context: Option<u64>,
    selected: BTreeSet<u64>,
}
type Completion = (u64, Key, Result<Summary, String>);
fn selected_guide(model: &Model, mut id: u64, selected: &BTreeSet<u64>) -> bool {
    loop {
        if selected.contains(&id) {
            return true;
        }
        let Some(parent) = model.parent(id) else {
            return false;
        };
        id = parent;
    }
}
fn guide_only_change(before: &Model, after: &Model) -> bool {
    if before.vertices != after.vertices
        || before.edges != after.edges
        || before.faces != after.faces
        || before.face_colors != after.face_colors
        || before.dimensions != after.dimensions
        || before.texts != after.texts
        || before.connections != after.connections
        || before.stock_materials != after.stock_materials
        || before.hidden != after.hidden
        || before.locked != after.locked
        || before.tags != after.tags
        || before.entity_tags != after.entity_tags
        || before.objects.len() != after.objects.len()
    {
        return false;
    }
    before.objects.iter().all(|(id, old)| {
        let Some(new) = after.objects.get(id) else {
            return false;
        };
        if old == new {
            return true;
        }
        if !old
            .entities
            .symmetric_difference(&new.entities)
            .all(|id| before.guides.contains_key(id) || after.guides.contains_key(id))
        {
            return false;
        }
        let mut attributes = old.clone();
        attributes.entities.clone_from(&new.entities);
        attributes == *new
    })
}
#[derive(Default)]
pub(super) struct Cache {
    source: Option<Arc<Model>>,
    revision: Option<u64>,
    generation: u64,
    results: BTreeMap<Key, Result<Summary, String>>,
    pending: BTreeSet<Key>,
    active: Option<(Key, Option<Result<Summary, String>>)>,
    completed: Arc<Mutex<Vec<Completion>>>,
    #[cfg(target_arch = "wasm32")]
    client: Option<eframe::wasm_bindgen::JsValue>,
    #[cfg(target_arch = "wasm32")]
    covered: BTreeSet<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    sender: Option<std::sync::mpsc::Sender<(u64, Key, Arc<Model>, egui::Context)>>,
}
impl Cache {
    pub fn prime(&mut self, model: Arc<Model>, revision: u64) {
        if self.revision == Some(revision) {
            return;
        }
        self.revision = Some(revision);
        let guides_only = self
            .source
            .as_ref()
            .is_some_and(|before| guide_only_change(before, &model));
        self.source = Some(model);
        if guides_only {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.results.clear();
        self.pending.clear();
        self.active = None;
        #[cfg(target_arch = "wasm32")]
        {
            self.covered.clear();
            if let Some(previous) = self.client.take() {
                if let Ok(stop) = js_sys::Reflect::get(&previous, &"stop".into()) {
                    use eframe::wasm_bindgen::JsCast;
                    if let Some(stop) = stop.dyn_ref::<js_sys::Function>() {
                        let _ = stop.call0(&previous);
                    }
                }
            }
        }
    }
    pub fn request(
        &mut self,
        model: Arc<Model>,
        revision: u64,
        selected: &BTreeSet<u64>,
        context: Option<u64>,
        ctx: &egui::Context,
    ) -> Option<Result<Summary, String>> {
        self.prime(Arc::clone(&model), revision);
        for (generation, key, result) in self.completed.lock().unwrap().drain(..) {
            if generation == self.generation {
                self.pending.remove(&key);
                if let Some((active, cached)) = &mut self.active {
                    if *active == key {
                        *cached = Some(result.clone());
                    }
                }
                self.results.insert(key, result);
            }
        }
        if self
            .active
            .as_ref()
            .is_some_and(|(key, _)| key.context == context && key.selected == *selected)
        {
            return self.active_result(&model, selected);
        }
        let key = Key {
            context,
            selected: selected.clone(),
        };
        let cached = self.results.get(&key).cloned();
        self.active = Some((key.clone(), cached));
        if self.active.as_ref().unwrap().1.is_some() {
            return self.active_result(&model, selected);
        }
        if !self.pending.insert(key.clone()) {
            return None;
        }
        let generation = self.generation;
        #[cfg(target_arch = "wasm32")]
        {
            use eframe::wasm_bindgen::{JsCast, JsValue};
            // An erase leaves nothing selected. Do not serialize the entire
            // scene for a measurement worker until the inspector requests it.
            let missing: BTreeSet<_> = selected.difference(&self.covered).copied().collect();
            if self.client.is_none() || !missing.is_empty() {
                if let Ok(source) = serde_json::to_string(&model.copy_entities(&missing)) {
                    if let Some(client) = &self.client {
                        if let Ok(merge) = js_sys::Reflect::get(client, &"merge".into())
                            .and_then(|value| value.dyn_into::<js_sys::Function>())
                        {
                            let _ = merge.call1(client, &source.into());
                        }
                    } else {
                        let launch = js_sys::Function::new_with_args(
                            "source",
                            include_str!("selection_measurement_worker.js"),
                        );
                        self.client = launch.call1(&JsValue::NULL, &source.into()).ok();
                    }
                    self.covered.extend(model.expand_entities(&missing));
                }
            }
            let result = self
                .client
                .as_ref()
                .ok_or_else(|| "The measurement worker is unavailable.".to_string())
                .and_then(|client| {
                    let function = js_sys::Reflect::get(client, &"request".into())
                        .map_err(|e| format!("{e:?}"))?
                        .dyn_into::<js_sys::Function>()
                        .map_err(|e| format!("{e:?}"))?;
                    function
                        .call2(
                            client,
                            &serde_json::to_string(selected).unwrap().into(),
                            &serde_json::to_string(&context).unwrap().into(),
                        )
                        .map_err(|e| format!("{e:?}"))?
                        .dyn_into::<js_sys::Promise>()
                        .map_err(|e| format!("{e:?}"))
                });
            let completed = Arc::clone(&self.completed);
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = match result {
                    Ok(promise) => wasm_bindgen_futures::JsFuture::from(promise)
                        .await
                        .map_err(|e: JsValue| format!("{e:?}"))
                        .and_then(|v| {
                            v.as_string()
                                .ok_or_else(|| "Invalid measurement result.".into())
                        })
                        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string())),
                    Err(error) => Err(error),
                };
                completed.lock().unwrap().push((generation, key, result));
                ctx.request_repaint();
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let sender = self.sender.get_or_insert_with(|| {
                let (sender, receiver) =
                    std::sync::mpsc::channel::<(u64, Key, Arc<Model>, egui::Context)>();
                let completed = Arc::clone(&self.completed);
                std::thread::Builder::new()
                    .name("benchup-inspector".into())
                    .spawn(move || {
                        let mut volumes = solid_tools::VolumeCache::default();
                        for (generation, key, model, ctx) in receiver {
                            let mut model = (*model).clone();
                            model.edit_context = key.context;
                            let result = measure(&model, &key.selected, &mut volumes);
                            completed
                                .lock()
                                .unwrap()
                                .push((generation, key, Ok(result)));
                            ctx.request_repaint();
                        }
                    })
                    .expect("Inspector worker thread");
                sender
            });
            if sender.send((generation, key, model, ctx.clone())).is_err() {
                self.sender = None;
            }
        }
        None
    }
    fn active_result(
        &self,
        model: &Model,
        selected: &BTreeSet<u64>,
    ) -> Option<Result<Summary, String>> {
        let mut result = self.active.as_ref()?.1.clone()?;
        if let Ok(summary) = &mut result {
            // Guide-only edits preserve geometric results, but membership counts
            // still follow the current document without a worker round trip.
            summary.guides = model
                .guides
                .keys()
                .filter(|&&id| selected_guide(model, id, selected))
                .count();
        }
        Some(result)
    }
    #[cfg(target_arch = "wasm32")]
    pub fn first_error(&self) -> Option<String> {
        self.results
            .values()
            .find_map(|value| value.as_ref().err().cloned())
    }
    #[cfg(target_arch = "wasm32")]
    pub fn error_count(&self) -> usize {
        self.results.values().filter(|value| value.is_err()).count()
    }
    #[cfg(target_arch = "wasm32")]
    pub fn ready_count(&self) -> usize {
        self.results.values().filter(|value| value.is_ok()).count()
    }
}
impl Drop for Cache {
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        if let Some(client) = &self.client {
            use eframe::wasm_bindgen::JsCast;
            if let Ok(stop) = js_sys::Reflect::get(client, &"stop".into()) {
                if let Some(stop) = stop.dyn_ref::<js_sys::Function>() {
                    let _ = stop.call0(client);
                }
            }
        }
    }
}
fn measure(
    model: &Model,
    selected: &BTreeSet<u64>,
    volumes: &mut solid_tools::VolumeCache,
) -> Summary {
    let ids = model.expand_entities(selected);
    let mut result = Summary::default();
    for id in &ids {
        if let Some(edge) = model.edges.get(id) {
            result.edges += 1;
            result.length += model.edge_length(edge);
        }
        if let Some(face) = model.faces.get(id) {
            result.faces += 1;
            result.area += model
                .face_triangles(face)
                .iter()
                .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() * 0.5)
                .sum::<f64>();
            let sides: Vec<Vec<f64>> = std::iter::once(&face.vertices)
                .chain(&face.holes)
                .map(|ring| {
                    (0..ring.len())
                        .map(|i| {
                            model.vertices[&ring[i]]
                                .position
                                .distance(model.vertices[&ring[(i + 1) % ring.len()]].position)
                        })
                        .collect()
                })
                .collect();
            result.perimeter += sides.iter().flatten().sum::<f64>();
            result.sides = if result.faces == 1 { sides } else { Vec::new() };
        }
        result.dimensions += usize::from(model.dimensions.contains_key(id));
        result.texts += usize::from(model.texts.contains_key(id));
        result.guides += usize::from(model.guides.contains_key(id));
    }
    result.size = volumes.solid_selection_size(model, selected);
    if selected.len() == 1
        && let Some(&id) = selected.first()
        && model.objects.contains_key(&id)
    {
        result.volume = volumes.volume(model, id).ok();
    }
    result
}
#[cfg(target_arch = "wasm32")]
thread_local! {static WORKER:std::cell::RefCell<(Model,solid_tools::VolumeCache)>=std::cell::RefCell::new((Model::default(),solid_tools::VolumeCache::default()));}
#[cfg(target_arch = "wasm32")]
#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn benchup_measurement_load(json: &str) -> Result<(), eframe::wasm_bindgen::JsValue> {
    let model: Model = serde_json::from_str(json)
        .map_err(|e| eframe::wasm_bindgen::JsValue::from_str(&e.to_string()))?;
    WORKER.with(|state| *state.borrow_mut() = (model, solid_tools::VolumeCache::default()));
    Ok(())
}
#[cfg(target_arch = "wasm32")]
#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn benchup_measurement_merge(json: &str) -> Result<(), eframe::wasm_bindgen::JsValue> {
    let source: Model = serde_json::from_str(json)
        .map_err(|e| eframe::wasm_bindgen::JsValue::from_str(&e.to_string()))?;
    WORKER.with(|state| {
        let (model, volumes) = &mut *state.borrow_mut();
        macro_rules! merge { ($($field:ident),*) => { $(model.$field.extend(source.$field);)* }; }
        merge!(
            vertices,
            edges,
            faces,
            face_colors,
            dimensions,
            texts,
            guides,
            objects,
            connections,
            stock_materials,
            tags,
            entity_tags,
            hidden,
            locked
        );
        *volumes = solid_tools::VolumeCache::default();
    });
    Ok(())
}
#[cfg(target_arch = "wasm32")]
#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn benchup_measure_selection(
    selected: &str,
    context: &str,
) -> Result<String, eframe::wasm_bindgen::JsValue> {
    let selected: BTreeSet<u64> = serde_json::from_str(selected)
        .map_err(|e| eframe::wasm_bindgen::JsValue::from_str(&e.to_string()))?;
    let context: Option<u64> = serde_json::from_str(context)
        .map_err(|e| eframe::wasm_bindgen::JsValue::from_str(&e.to_string()))?;
    WORKER.with(|state| {
        let (model, volumes) = &mut *state.borrow_mut();
        model.edit_context = context;
        serde_json::to_string(&measure(model, &selected, volumes))
            .map_err(|e| eframe::wasm_bindgen::JsValue::from_str(&e.to_string()))
    })
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(super) fn validate_request_cache() -> Result<usize, String> {
    let model = Arc::new(Model::default());
    let selected: BTreeSet<_> = (1..150_000).collect();
    let ctx = egui::Context::default();
    let mut cache = Cache::default();
    cache.prime(model.clone(), 1);
    let key = Key {
        context: None,
        selected: selected.clone(),
    };
    cache.pending.insert(key.clone());
    cache.active = Some((key.clone(), None));
    let address = cache.active.as_ref().unwrap().0.selected.first().unwrap() as *const u64;
    if cache
        .request(model.clone(), 1, &selected, None, &ctx)
        .is_some()
        || cache.pending.len() != 1
    {
        return Err("Pending measurement request was duplicated".into());
    }
    cache
        .completed
        .lock()
        .unwrap()
        .push((cache.generation, key, Ok(Summary::default())));
    if !cache
        .request(model.clone(), 1, &selected, None, &ctx)
        .is_some_and(|r| r.is_ok())
        || !cache.pending.is_empty()
    {
        return Err("Completed measurement did not replace the pending request".into());
    }
    if !cache
        .request(model.clone(), 1, &selected, None, &ctx)
        .is_some_and(|r| r.is_ok())
        || address != cache.active.as_ref().unwrap().0.selected.first().unwrap() as *const u64
    {
        return Err("Unchanged selection cloned its active measurement key".into());
    }
    let context = Some(999);
    cache.pending.insert(Key {
        context,
        selected: selected.clone(),
    });
    if cache
        .request(model.clone(), 1, &selected, context, &ctx)
        .is_some()
    {
        return Err("Measurement result leaked across editing scopes".into());
    }
    let mut changed = (*model).clone();
    changed
        .add_edge(DVec3::ZERO, DVec3::X)
        .map_err(str::to_owned)?;
    cache.prime(Arc::new(changed), 2);
    if cache.active.is_some() || !cache.results.is_empty() || !cache.pending.is_empty() {
        return Err("Geometry change retained stale selection measurements".into());
    }
    Ok(5)
}

#[cfg(test)]
mod guide_cache_tests {
    use super::*;
    #[test]
    fn unchanged_large_selection_reuses_pending_and_completed_requests() {
        assert_eq!(validate_request_cache().unwrap(), 5);
    }
    #[test]
    fn guide_edits_preserve_geometric_results_and_refresh_membership_counts() {
        let mut model = Model::default();
        let edge = model.add_edge(DVec3::ZERO, DVec3::X).unwrap();
        let group = model
            .make_object(&BTreeSet::from([edge]), false, "Part")
            .unwrap();
        let selected = BTreeSet::from([group]);
        let mut cache = Cache::default();
        cache.prime(Arc::new(model.clone()), 1);
        let key = Key {
            context: None,
            selected: selected.clone(),
        };
        cache
            .results
            .insert(key, Ok(measure(&model, &selected, &mut Default::default())));
        let generation = cache.generation;
        model.edit_context = Some(group);
        model.add_guide(DVec3::splat(50.), None).unwrap();
        model.edit_context = None;
        let result = cache
            .request(
                Arc::new(model.clone()),
                2,
                &selected,
                None,
                &egui::Context::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(cache.generation, generation);
        let fresh = measure(&model, &selected, &mut Default::default());
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::to_value(fresh).unwrap()
        );
        model.vertices.values_mut().next().unwrap().position.z += 1.;
        cache.prime(Arc::new(model), 3);
        assert_ne!(cache.generation, generation);
        assert!(cache.results.is_empty());
    }
}
