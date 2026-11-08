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
pub(super) fn tick(app: &mut KetchupApp, ctx: &egui::Context) {
    BENCHMARK.with(|state| {
        let mut state = state.borrow_mut();
        let now = ctx.input(|i| i.time);
        if !state.0 {
            state.0 = true;
            let window = web_sys::window().unwrap();
            let request = js_sys::Reflect::get(&window, &"ketchupViewportBenchmark".into()).ok();
            if let Some(seconds) = request.and_then(|v| v.as_f64()) {
                let camera = js_sys::Reflect::get(&window, &"ketchupViewportBenchmarkCamera".into()).ok()
                    .and_then(|value| js_sys::JSON::stringify(&value).ok())
                    .and_then(|value| value.as_string())
                    .and_then(|value| serde_json::from_str::<Camera>(&value).ok())
                    .filter(Camera::valid).unwrap_or_else(|| app.doc.camera.clone());
                let mode = js_sys::Reflect::get(&window, &"ketchupViewportBenchmarkMode".into()).ok().and_then(|v|v.as_string()).unwrap_or_else(|| "orbit".into());
                let root = app.doc.model.root_entities().into_iter().find(|id| app.doc.model.objects.contains_key(id));
                let child = root.and_then(|root|app.doc.model.objects[&root].entities.iter().copied().find(|id|app.doc.model.objects.contains_key(id)));
                state.1 = Some(Benchmark { camera, mode, action: None, actions: 0, action_frames: Vec::new(), acted: false, root, child, start: now,
                    last: now, number: u64::MAX, seconds: seconds.clamp(4., 120.),
                    samples: Vec::new(), finished: false, draw_frame: u64::MAX,
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
            if bench.acted { bench.action_frames.push(interval); }
        }
        bench.acted = false;
        bench.last = now;
        app.release_notes.open = false;
        app.doc.camera = bench.camera.clone();
        if bench.mode == "orbit" {
            app.set_tool(Tool::Orbit);
            app.doc.camera.yaw += 0.35 * (elapsed * 0.8).sin();
            app.doc.camera.pitch += 0.1 * (elapsed * 0.65).sin();
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
            let result = serde_json::json!({ "mode": bench.mode, "actions": bench.actions,
                "final_selection": app.selected, "final_context": app.doc.model.edit_context,
                "action_frame_p95_ms": bench.action_frames.get(bench.action_frames.len()*95/100), "frames": n, "fps": 1000. / mean,
                "mean_ms": mean, "p95_ms": bench.samples.get(n * 95 / 100),
                "faces": app.doc.model.faces.len(), "edges": app.doc.model.edges.len(),
                "camera": bench.camera, "draws": bench.draws, "resident_frames": bench.resident_frames,"selected_frames":bench.selected_frames,"group_frames":bench.group_frames,"scopes":bench.scopes,"measurements_ready":app.selection_measurements.ready_count(),"measurement_errors":app.selection_measurements.error_count(),"measurement_error":app.selection_measurements.first_error(),
                "yaw_range": bench.yaw_range, "pitch_range": bench.pitch_range,
                "edges_visible": app.doc.display.edges, "resident_builds": app.viewports.resident_builds(), "size": [ctx.viewport_rect().width(), ctx.viewport_rect().height()] });
            let _ = js_sys::Reflect::set(&web_sys::window().unwrap(),
                &JsValue::from_str("ketchupViewportBenchmarkResult"),
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

fn publish_state(app: &KetchupApp) {
    let value = serde_json::json!({"selected": app.selected, "context": app.doc.model.edit_context,
            "tool": format!("{:?}", app.tool), "measurement_value": app.measurement_value(),
            "measurement_points": app.measurement_points(), "guides": app.doc.model.guides.len(),
            "camera": app.doc.camera, "walk_feet": app.walk.person.as_ref().map(|p|p.feet),
            "resident_builds": app.viewports.resident_builds()});
    let _ = js_sys::Reflect::set(
        &web_sys::window().unwrap(),
        &JsValue::from_str("ketchupViewportBenchmarkState"),
        &js_sys::JSON::parse(&value.to_string()).unwrap(),
    );
}

// Run off the UI thread via the benchmark's worker. Compare the indexed path
// with the original exhaustive queries on the actual project, not a proxy mesh.
#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn ketchup_validate_snapping(source: &str) -> Result<String, JsValue> {
    use ketchup_core::inference::{
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
            ketchup_core::Projection::Perspective,
            ketchup_core::Projection::Parallel,
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
