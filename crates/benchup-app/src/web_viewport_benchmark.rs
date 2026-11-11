//! Opt-in automation only: no saved settings or changes to the source project.
use super::*;
use eframe::wasm_bindgen::JsValue;
use std::cell::RefCell;

thread_local! {
    static BENCHMARK: RefCell<(bool, Option<Benchmark>)> = const { RefCell::new((false, None)) };
}
struct Benchmark {
    camera: Camera,
    mode: String,
    action: Option<u64>,
    action_frames: Vec<f64>,
    acted: bool,
    actions: usize,
    root: Option<u64>,
    child: Option<u64>,
    start: f64,
    last: f64,
    number: u64,
    seconds: f64,
    samples: Vec<f64>,
    selected_samples: Vec<f64>,
    unselected_samples: Vec<f64>,
    last_selected: bool,
    finished: bool,
    draw_frame: u64,
    draws: usize,
    resident_frames: usize,
    selected_frames: usize,
    group_frames: usize,
    scopes: BTreeSet<u64>,
    yaw_range: [f64; 2],
    pitch_range: [f64; 2],
}
pub(super) fn tick(app: &mut BenchUpApp, ctx: &egui::Context) {
    BENCHMARK.with(|state| {
        let mut state = state.borrow_mut();
        let now = ctx.input(|i| i.time);
        if !state.0 {
            state.0 = true;
            let window = web_sys::window().unwrap();
            let request = js_sys::Reflect::get(&window, &"benchupViewportBenchmark".into()).ok();
            if let Some(seconds) = request.and_then(|v| v.as_f64()) {
                let camera = js_sys::Reflect::get(&window, &"benchupViewportBenchmarkCamera".into()).ok()
                    .and_then(|value| js_sys::JSON::stringify(&value).ok())
                    .and_then(|value| value.as_string())
                    .and_then(|value| serde_json::from_str::<Camera>(&value).ok())
                    .filter(Camera::valid).unwrap_or_else(|| app.doc.camera.clone());
                let mode = js_sys::Reflect::get(&window, &"benchupViewportBenchmarkMode".into()).ok().and_then(|v|v.as_string()).unwrap_or_else(|| "orbit".into());
                let root = app.doc.model.root_entities().into_iter().find(|id| app.doc.model.objects.contains_key(id));
                let child = root.and_then(|root|app.doc.model.objects[&root].entities.iter().copied().find(|id|app.doc.model.objects.contains_key(id)));
                state.1 = Some(Benchmark { camera, mode, action: None, actions: 0, action_frames: Vec::new(), acted: false, root, child, start: now,
                    last: now, number: u64::MAX, seconds: seconds.clamp(4., 120.),
                    samples: Vec::new(), selected_samples: Vec::new(), unselected_samples: Vec::new(),
                    last_selected: !app.selected.is_empty(), finished: false, draw_frame: u64::MAX,
                    draws: 0, resident_frames: 0, selected_frames:0,group_frames:0,scopes:BTreeSet::new(), yaw_range: [f64::INFINITY,f64::NEG_INFINITY],
                    pitch_range: [f64::INFINITY,f64::NEG_INFINITY] });
            }
        }
        let Some(bench) = &mut state.1 else { return };
        app.release_notes.open = false;
        publish_state(app);
        if bench.mode == "observe" { return; }
        let number = ctx.cumulative_frame_nr();
        if bench.finished || bench.number == number { return; }
        bench.number = number;
        let elapsed = now - bench.start;
        if elapsed >= 5. {
            let interval = (now - bench.last) * 1000.;
            bench.samples.push(interval);
            if bench.last_selected { bench.selected_samples.push(interval); }
            else { bench.unselected_samples.push(interval); }
            if bench.acted { bench.action_frames.push(interval); }
        }
        bench.acted = false;
        bench.last = now;
        app.release_notes.open = false;
        app.doc.camera = bench.camera.clone();
        if bench.mode == "orbit" || bench.mode == "sun" {
            app.set_tool(Tool::Orbit);
            if bench.mode == "sun" {
                app.doc.display.shadows = true;
                app.doc.display.sun_angle = 80. + 15. * (elapsed * 0.6).sin();
            } else {
                app.doc.camera.yaw += 0.35 * (elapsed * 0.8).sin();
                app.doc.camera.pitch += 0.1 * (elapsed * 0.65).sin();
            }
        } else if bench.mode == "scale" {
            if app.tool != Tool::Scale { app.set_tool(Tool::Scale); }
            if app.modeling.is_none() && elapsed > 1. {
                app.selected = bench.root.into_iter().collect();
                let (_, hi) = app.doc.model.selection_bounds(&app.selected).unwrap();
                let point = app.viewport.min + pv(app.doc.camera.project(hi, dv(app.viewport.size())).unwrap());
                app.begin_modeling(point, ctx);
            }
            if let Some(edit) = &app.modeling {
                let pointer = edit.pointer;
                app.length_input = format!("{:.6}", 1. + 0.15 * (elapsed * 1.5).sin());
                app.update_modeling(pointer);
                bench.acted = true;
                bench.actions += 1;
            }
        } else {
            if app.tool != Tool::Select { app.set_tool(Tool::Select); }
            // The same pick query used by viewport clicks, moving over the scene.
            let size = dv(app.viewport.size());
            let point = DVec2::new(size.x * (0.5 + 0.18 * (elapsed * 0.8).sin()), size.y * 0.47);
            let hit = app.pick_at(point,size,true);
            app.hover = hit;
            let action = (elapsed / 0.25) as u64;
            if bench.action != Some(action) {
                bench.action = Some(action);
                bench.acted = true;
                bench.actions += 1;
                if bench.mode == "select" {
                    app.selected = if action % 2 == 0 { hit.or(bench.root).into_iter().collect() } else { BTreeSet::new() };
                } else {
                    match action % 4 {
                        0 => { app.navigate_context(None,ctx); if let Some(id)=bench.root {app.enter_object(id,ctx);} }
                        1 => { if let Some(id)=bench.child {app.enter_object(id,ctx);} }
                        2 => app.close_edit_context(ctx),
                        _ => app.navigate_context(None,ctx),
                    }
                }
            }
        }
        bench.last_selected = !app.selected.is_empty();
        if elapsed >= bench.seconds + 5. {
            bench.finished = true;
            // Leave a deterministic real editor state for screenshot/reload QA.
            if bench.mode == "select" {
                app.selected = bench.root.into_iter().collect();
            } else if bench.mode == "groups" {
                app.navigate_context(None, ctx);
                if let Some(root) = bench.root { app.enter_object(root, ctx); }
            }
            let n = bench.samples.len();
            let mean = bench.samples.iter().sum::<f64>() / n.max(1) as f64;
            bench.samples.sort_by(f64::total_cmp);
            bench.action_frames.sort_by(f64::total_cmp);
            let stats = |samples: &mut Vec<f64>| {
                samples.sort_by(f64::total_cmp);
                let mean = samples.iter().sum::<f64>() / samples.len().max(1) as f64;
                serde_json::json!({"frames": samples.len(), "mean_ms": mean,
                    "fps": if mean > 0. { 1000. / mean } else { 0. },
                    "p95_ms": samples.get(samples.len() * 95 / 100)})
            };
            let result = serde_json::json!({ "mode": bench.mode, "actions": bench.actions,
                "selected": stats(&mut bench.selected_samples), "unselected": stats(&mut bench.unselected_samples),
                "final_selection": app.selected, "final_context": app.doc.model.edit_context,
                "gpu_scaling": app.modeling.as_ref().is_some_and(|edit| edit.gpu_scale.is_some()),
                "action_frame_p95_ms": bench.action_frames.get(bench.action_frames.len()*95/100), "frames": n, "fps": 1000. / mean,
                "mean_ms": mean, "p95_ms": bench.samples.get(n * 95 / 100),
                "faces": app.doc.model.faces.len(), "edges": app.doc.model.edges.len(),
                "camera": bench.camera, "draws": bench.draws, "resident_frames": bench.resident_frames,"selected_frames":bench.selected_frames,"group_frames":bench.group_frames,"scopes":bench.scopes,"measurements_ready":app.selection_measurements.ready_count(),"measurement_errors":app.selection_measurements.error_count(),"measurement_error":app.selection_measurements.first_error(),
                "yaw_range": bench.yaw_range, "pitch_range": bench.pitch_range,
                "edges_visible": app.doc.display.edges, "shadows":app.doc.display.shadows,"shadow_builds":app.viewports.resident_shadow_builds(),"resident_builds": app.viewports.resident_builds(), "size": [ctx.viewport_rect().width(), ctx.viewport_rect().height()] });
            let _ = js_sys::Reflect::set(&web_sys::window().unwrap(),
                &JsValue::from_str("benchupViewportBenchmarkResult"),
                &js_sys::JSON::parse(&result.to_string()).unwrap());
            app.doc.camera = bench.camera.clone();
        } else { ctx.request_repaint(); }
    });
}

// Observe cameras actually submitted by the viewport, not just requested poses.
pub(super) fn record_view(
    camera: &Camera,
    ctx: &egui::Context,
    resident: bool,
    selected: usize,
    context: Option<u64>,
) {
    BENCHMARK.with(|state| {
        let mut state = state.borrow_mut();
        let Some(bench) = &mut state.1 else { return };
        let frame = ctx.cumulative_frame_nr();
        if bench.finished || bench.draw_frame == frame || ctx.input(|i| i.time) - bench.start < 5. {
            return;
        }
        bench.draw_frame = frame;
        bench.draws += 1;
        bench.resident_frames += usize::from(resident);
        bench.selected_frames += usize::from(selected != 0);
        bench.group_frames += usize::from(context.is_some());
        if let Some(id) = context {
            bench.scopes.insert(id);
        }
        bench.yaw_range[0] = bench.yaw_range[0].min(camera.yaw);
        bench.yaw_range[1] = bench.yaw_range[1].max(camera.yaw);
        bench.pitch_range[0] = bench.pitch_range[0].min(camera.pitch);
        bench.pitch_range[1] = bench.pitch_range[1].max(camera.pitch);
    });
}

fn publish_state(app: &BenchUpApp) {
    let value = serde_json::json!({"selected": app.selected.iter().take(512).copied().collect::<Vec<_>>(),
            "selected_count":app.selected.len(), "selection_truncated":app.selected.len()>512,
            "context": app.doc.model.edit_context,
            "tool": format!("{:?}", app.tool), "measurement_value": app.measurement_value(),
            "faces": app.doc.model.faces.len(), "edges": app.doc.model.edges.len(),
            "file_pending": app.project.pending.is_some(), "file_error": app.project.error,
            "history_position": app.doc.history.position(), "screenshot": app.screenshot.benchmark_state(),
            "measurement_points": app.measurement_points(), "guides": app.doc.model.guides.len(),
            "camera": app.doc.camera, "walk_feet": app.walk.person.as_ref().map(|p|p.feet),
            "shadows":app.doc.display.shadows,"shadow_builds":app.viewports.resident_shadow_builds(),"resident_builds": app.viewports.resident_builds()});
    let _ = js_sys::Reflect::set(
        &web_sys::window().unwrap(),
        &JsValue::from_str("benchupViewportBenchmarkState"),
        &js_sys::JSON::parse(&value.to_string()).unwrap(),
    );
}

// Run off the UI thread via the benchmark's worker. Compare the indexed path
// with the original exhaustive queries on the actual project, not a proxy mesh.
#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn benchup_validate_snapping(source: &str) -> Result<String, JsValue> {
    use benchup_core::inference::{
        FaceQuery, SnapTargets, infer_axis_alignment_with_constraint, infer_with_constraint,
    };
    let error = |message: String| JsValue::from_str(&message);
    let doc = Document::from_json(source).map_err(error)?;
    let mut model = doc.model;
    let root = model
        .root_entities()
        .into_iter()
        .find(|id| model.objects.contains_key(id));
    let mut faces = renderer::FaceCache::default();
    let mut entities = renderer::EntityCache::default();
    let mut picking = crate::picking::PickingCache::default();
    let empty = BTreeSet::new();
    let size = DVec2::new(1064., 748.);
    let mut camera = doc.camera;
    let mut comparisons = 0;
    let mut max_error: f64 = 0.;
    faces.update_for_revision(&model, 1);
    for context in [None, root] {
        model.edit_context = context;
        entities.update_for_revision(&model, 1);
        for projection in [
            benchup_core::Projection::Perspective,
            benchup_core::Projection::Parallel,
        ] {
            camera.projection = projection;
            picking.prepare_snapping(&model, &faces, &entities, &camera, size);
            let indexed = picking.snap_query(&faces, &entities);
            let full = picking
                .snap_query(&faces, &entities)
                .excluding(Some(&empty));
            for i in 0..12 {
                let cursor = DVec2::new(
                    size.x * (0.3 + 0.05 * i as f64),
                    size.y * (0.45 + 0.1 * (i as f64).sin()),
                );
                let start = (i % 4 != 0).then_some(camera.target);
                let constraint = match i % 4 {
                    1 => Some(AxisConstraint::Axis(Axis::X)),
                    2 => Some(AxisConstraint::Plane(Axis::Z)),
                    3 => Some(AxisConstraint::Axis(Axis::Z)),
                    _ => None,
                };
                let targets = match i % 3 {
                    1 => SnapTargets {
                        edges: true,
                        faces: false,
                        face_centers: false,
                    },
                    2 => SnapTargets {
                        edges: false,
                        faces: true,
                        face_centers: true,
                    },
                    _ => SnapTargets::default(),
                };
                let infer = |query: &dyn FaceQuery| {
                    infer_with_constraint(
                        &model,
                        &camera,
                        size,
                        cursor,
                        start,
                        constraint,
                        true,
                        true,
                        Some(query),
                        targets,
                    )
                };
                let a = infer(&indexed);
                let b = infer(&full);
                comparisons += 1;
                if a.map(|s| s.kind) != b.map(|s| s.kind) {
                    return Err(error(format!(
                        "Snap target mismatch at {cursor:?}, {projection:?}, {context:?}: {a:?} / {b:?}"
                    )));
                }
                if let (Some(a), Some(b)) = (a, b) {
                    max_error = max_error.max(a.point.distance(b.point));
                    if let Some(start) = start {
                        let align = |query: &dyn FaceQuery| {
                            infer_axis_alignment_with_constraint(
                                &model,
                                &camera,
                                size,
                                cursor,
                                start,
                                constraint,
                                true,
                                a,
                                Some(query),
                            )
                        };
                        let a = align(&indexed);
                        let b = align(&full);
                        comparisons += 1;
                        max_error = max_error.max(a.snap.point.distance(b.snap.point));
                        if a.snap.kind != b.snap.kind || a.guides.len() != b.guides.len() {
                            return Err(error(format!(
                                "Axis guide mismatch at {cursor:?}, {projection:?}, {context:?}: {a:?} / {b:?}"
                            )));
                        }
                    }
                }
            }
        }
    }
    if max_error > 1e-7 {
        return Err(error(format!("Snap position error: {max_error}")));
    }
    Ok(serde_json::json!({"passed":true,"comparisons":comparisons,"max_position_error_m":max_error}).to_string())
}

/// Validate retained indices through actual Rooftop edits in an isolated worker.
#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn benchup_validate_erase_caches(
    source: &str,
    context: u64,
    edge: u64,
) -> Result<String, JsValue> {
    let error = |message: String| JsValue::from_str(&message);
    let doc = Document::from_json(source).map_err(error)?;
    let mut model = doc.model;
    model.edit_context = Some(context);
    if model.entity_in_context(edge) != Some(edge) || !model.edges.contains_key(&edge) {
        return Err(error(
            "The edge must belong directly to the editing context.".into(),
        ));
    }
    let original = model.clone();
    let size = DVec2::new(1064., 748.);
    let (a, b) = model.points(&model.edges[&edge]);
    let pixel = doc
        .camera
        .project((a + b) * 0.5, size)
        .ok_or_else(|| error("The edge must be in view.".into()))?;
    let mut faces = renderer::FaceCache::default();
    let mut entities = renderer::EntityCache::default();
    let mut picking = crate::picking::PickingCache::default();
    faces.update_for_revision(&model, 1);
    entities.update_for_revision(&model, 1);
    picking.prepare_view(&model, &faces, &doc.camera, size);
    let initial = picking.benchmark_builds();
    let mut checks = 0;
    for step in 0..4 {
        if step % 2 == 0 {
            model.erase(&BTreeSet::from([edge]));
        } else {
            model = original.clone();
        }
        model.validate().map_err(|message| error(message.into()))?;
        faces.update_for_revision(&model, step + 2);
        entities.update_for_revision(&model, step + 2);
        picking.prepare_view(&model, &faces, &doc.camera, size);
        let local = model
            .object_contents(context)
            .ok_or_else(|| error("The editing context disappeared.".into()))?;
        for offset in [
            DVec2::ZERO,
            DVec2::new(4., 3.),
            DVec2::new(-8., 5.),
            DVec2::new(16., -12.),
        ] {
            let cursor = pixel + offset;
            let actual = picking.pick(
                &model,
                &faces,
                &entities,
                &doc.camera,
                size,
                cursor,
                9.,
                false,
            );
            let expected = selection::pick(&local, &doc.camera, size, cursor, 9.);
            if actual != expected {
                return Err(error(format!(
                    "Eraser query differs after edit {step}: {actual:?} != {expected:?}"
                )));
            }
            checks += 1;
        }
        if picking.benchmark_builds() != initial {
            return Err(error(format!(
                "A local erase rebuilt the picking indices: {:?} -> {:?}",
                initial,
                picking.benchmark_builds()
            )));
        }
    }
    Ok(serde_json::json!({"passed":true,"queries":checks,"initial_builds":initial,"final_builds":picking.benchmark_builds()}).to_string())
}

#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn benchup_validate_edit_pipeline(
    source: &str,
    context: u64,
    edge: u64,
) -> Result<String, JsValue> {
    let doc = Document::from_json(source).map_err(|e| JsValue::from_str(&e))?;
    let shadow_checks = renderer::SceneCache::validate_shadow_cache(&doc)
        .map_err(|e| JsValue::from_str(&e))?;
    let checks = renderer::SceneCache::validate_edits(doc.clone(), context, edge)
        .map_err(|e| JsValue::from_str(&e))?;
    let snapping_checks = crate::picking::PickingCache::validate_snap_edits(doc.model, &doc.camera)
        .map_err(|e| JsValue::from_str(&e))?;
    Ok(
        serde_json::json!({"passed":true,"checks":checks,"snapping_checks":snapping_checks,"shadow_checks":shadow_checks})
            .to_string(),
    )
}

#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn benchup_validate_box_selection() -> Result<String, JsValue> {
    let checks = crate::picking::PickingCache::validate_box_selection()
        .map_err(|e| JsValue::from_str(&e))?;
    let measurement_cache_checks = selection_measurements::validate_request_cache()
        .map_err(|e| JsValue::from_str(&e))?;
    Ok(serde_json::json!({"passed":true,"comparisons":checks,"measurement_cache_checks":measurement_cache_checks}).to_string())
}

/// Independent ownership-index comparison for a complete marquee fixture.
#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn benchup_box_selection(source: &str, query: &str) -> Result<String, JsValue> {
    #[derive(serde::Deserialize)]
    struct Scene {
        model: Model,
        selection_context: Option<u64>,
    }
    #[derive(serde::Deserialize)]
    struct Query {
        camera: Camera,
        size: DVec2,
        start: DVec2,
        end: DVec2,
        guides: bool,
    }
    let error = |e: String| JsValue::from_str(&e);
    let mut scene: Scene = serde_json::from_str(source).map_err(|e| error(e.to_string()))?;
    let query: Query = serde_json::from_str(query).map_err(|e| error(e.to_string()))?;
    scene.model.edit_context = scene.selection_context;
    scene.model.validate().map_err(|e| error(e.into()))?;
    let selected = selection::box_select_with_guides(
        &scene.model, &query.camera, query.size, query.start, query.end, query.guides,
    );
    serde_json::to_string(&selected).map_err(|e| error(e.to_string()))
}
