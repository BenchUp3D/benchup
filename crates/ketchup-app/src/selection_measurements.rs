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
#[derive(Default)]
pub(super) struct Cache {
    revision: Option<u64>,
    generation: u64,
    results: BTreeMap<Key, Result<Summary, String>>,
    pending: BTreeSet<Key>,
    completed: Arc<Mutex<Vec<Completion>>>,
    #[cfg(target_arch = "wasm32")]
    client: Option<eframe::wasm_bindgen::JsValue>,
    #[cfg(not(target_arch = "wasm32"))]
    sender: Option<std::sync::mpsc::Sender<(u64, Key, Arc<Model>, egui::Context)>>,
}
impl Cache {
    pub fn prime(&mut self, model: Arc<Model>, revision: u64) {
        if self.revision == Some(revision) {
            return;
        }
        self.revision = Some(revision);
        self.generation = self.generation.wrapping_add(1);
        self.results.clear();
        self.pending.clear();
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(previous) = self.client.take() {
                if let Ok(stop) = js_sys::Reflect::get(&previous, &"stop".into()) {
                    use eframe::wasm_bindgen::JsCast;
                    if let Some(stop) = stop.dyn_ref::<js_sys::Function>() {
                        let _ = stop.call0(&previous);
                    }
                }
            }
            if let Ok(source) = serde_json::to_string(&*model) {
                let launch = js_sys::Function::new_with_args(
                    "source",
                    include_str!("selection_measurement_worker.js"),
                );
                self.client = launch
                    .call1(&eframe::wasm_bindgen::JsValue::NULL, &source.into())
                    .ok();
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = model;
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
                self.results.insert(key, result);
            }
        }
        let key = Key {
            context,
            selected: selected.clone(),
        };
        if let Some(result) = self.results.get(&key) {
            return Some(result.clone());
        }
        if !self.pending.insert(key.clone()) {
            return None;
        }
        let generation = self.generation;
        #[cfg(target_arch = "wasm32")]
        {
            use eframe::wasm_bindgen::{JsCast, JsValue};
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
                    .name("ketchup-inspector".into())
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
pub fn ketchup_measurement_load(json: &str) -> Result<(), eframe::wasm_bindgen::JsValue> {
    let model: Model = serde_json::from_str(json)
        .map_err(|e| eframe::wasm_bindgen::JsValue::from_str(&e.to_string()))?;
    WORKER.with(|state| *state.borrow_mut() = (model, solid_tools::VolumeCache::default()));
    Ok(())
}
#[cfg(target_arch = "wasm32")]
#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn ketchup_measure_selection(
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
