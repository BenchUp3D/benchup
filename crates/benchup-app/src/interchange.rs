//! Geometry interchange with a native SKP reader on desktop and a browser worker.
use benchup_core::Model;
use benchup_core::tr;

// Published JavaScript artifacts are vendored to avoid an SDK, subprocess,
// network request, or JavaScript build step in either application.
#[cfg(target_arch = "wasm32")]
const SKP_READER: &str = concat!(
    "var benchupModules = {};\n",
    "benchupModules.fflate = (function() { var exports = {}, module = {exports};\n",
    include_str!("../vendor/skp/fflate.cjs"),
    "\nreturn module.exports; })();\n",
    "benchupModules.earcut = (function() { var exports = {}, module = {exports};\n",
    include_str!("../vendor/skp/earcut.cjs"),
    "\nreturn module.exports; })();\n",
    "benchupModules.flatbuffers = (function() {\n",
    include_str!("../vendor/skp/flatbuffers.js"),
    "\nreturn flatbuffers; })();\n",
    "benchupModules.openskp = (function() { var exports = {}, module = {exports}; var require = name => benchupModules[name];\n",
    include_str!("../vendor/skp/openskp.cjs"),
    "\nreturn module.exports; })();\n",
    include_str!("skp_adapter.js"),
);

// The native adapter and web worker return stable codes for BenchUp errors.
fn skp_error_message(error: &str) -> &str {
    match error {
        "benchup.skp.model_too_large" => tr!("interchange.skp_model_too_large"),
        "benchup.skp.instance_transform" => tr!("interchange.skp_instance_transform"),
        "benchup.skp.invalid_coordinates" => tr!("interchange.skp_invalid_coordinates"),
        "benchup.skp.nested_components" => tr!("interchange.skp_nested_components"),
        "benchup.skp.missing_vertex" => tr!("interchange.skp_missing_vertex"),
        "benchup.skp.missing_edge" => tr!("interchange.skp_missing_edge"),
        "benchup.skp.disconnected_boundary" => tr!("interchange.skp_disconnected_boundary"),
        "benchup.skp.invalid_boundary" => tr!("interchange.skp_invalid_boundary"),
        "benchup.skp.missing_definition" => tr!("interchange.skp_missing_definition"),
        "benchup.skp.no_geometry" => tr!("interchange.skp_no_geometry"),
        "benchup.skp.worker_failed" => tr!("interchange.skp_worker_failed"),
        "benchup.skp.worker_message" => tr!("interchange.skp_worker_message"),
        "benchup.skp.incomplete_import" => tr!("interchange.skp_incomplete_import"),
        _ => error,
    }
}

pub const MAX_SKP_BYTES: usize = 64 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
pub fn import_skp(bytes: &[u8], name: &str) -> Result<Model, String> {
    if bytes.len() > MAX_SKP_BYTES {
        return Err(tr!("interchange.sketchup_import_is_limited_to_64_mib_per").into());
    }
    let json =
        parse_skp(bytes).map_err(|e| tr!("interchange.could_not_read_sketchup_file", e = e))?;
    decode_model(&json, name)
}

fn decode_model(json: &str, name: &str) -> Result<Model, String> {
    let mut model: Model = serde_json::from_str(json).map_err(|e| e.to_string())?;
    model.normalize_imported_faces().map_err(str::to_owned)?;
    model.validate().map_err(str::to_owned)?;
    // The adapter always emits the outer import group last.
    if let Some((&root_id, _)) = model.objects.last_key_value() {
        for (id, group) in &mut model.objects {
            if *id == root_id {
                group.name = name.to_owned();
            } else if group.name.is_empty() {
                group.name = tr!("interchange.sketchup_group").into();
            }
        }
    }
    Ok(model)
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_skp(bytes: &[u8]) -> Result<String, String> {
    use std::ffi::{CStr, c_char};
    unsafe extern "C" {
        fn benchup_skp_parse(bytes: *const u8, len: usize, success: *mut i32) -> *mut c_char;
        fn benchup_skp_free(value: *mut c_char);
    }
    let mut success = 0;
    // The adapter catches C++ exceptions and returns one owned, NUL-terminated
    // UTF-8 buffer. It borrows the input only for the duration of this call.
    let value = unsafe { benchup_skp_parse(bytes.as_ptr(), bytes.len(), &mut success) };
    if value.is_null() {
        return Err(tr!("interchange.not_enough_memory_to_read_this_sketchup_file").into());
    }
    let text = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        benchup_skp_free(value);
    }
    if success == 1 {
        Ok(text)
    } else {
        Err(skp_error_message(&text).into())
    }
}

#[cfg(target_arch = "wasm32")]
pub async fn import_skp_async(bytes: &[u8], name: &str) -> Result<Model, String> {
    use eframe::wasm_bindgen::JsCast;
    if bytes.len() > MAX_SKP_BYTES {
        return Err(tr!("interchange.sketchup_import_is_limited_to_64_mib_per").into());
    }
    // Parsing runs in a dedicated worker so the picker, spinner, and application
    // keep responding while the reader expands the component tree.
    let launch = js_sys::Function::new_with_args("source, bytes", include_str!("skp_worker.js"));
    let promise = launch
        .call2(
            &eframe::wasm_bindgen::JsValue::NULL,
            &SKP_READER.into(),
            &js_sys::Uint8Array::from(bytes),
        )
        .map_err(js_error)?
        .dyn_into::<js_sys::Promise>()
        .map_err(js_error)?;
    let json = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(js_error)?
        .as_string()
        .ok_or(tr!("interchange.sketchup_reader_returned_no_model"))?;
    decode_model(&json, name)
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: eframe::wasm_bindgen::JsValue) -> String {
    let message = js_sys::Reflect::get(&error, &"message".into())
        .ok()
        .and_then(|message| message.as_string())
        .or_else(|| error.as_string())
        .unwrap_or_else(|| tr!("interchange.sketchup_reader_failed").into());
    skp_error_message(&message).into()
}

/// OBJ uses metre coordinates with BenchUp's Z-up convention. Triangulating
/// faces keeps holes intact even in readers without polygon-hole support.
pub fn export_obj(model: &Model) -> Result<String, String> {
    use std::{collections::BTreeMap, fmt::Write};
    let mut out = String::from("# BenchUp OBJ export\n# Units: metres; Z up\n");
    let visible_vertices: std::collections::BTreeSet<_> = model
        .edges
        .values()
        .filter(|e| !model.is_hidden(e.id))
        .flat_map(|e| [e.a, e.b])
        .chain(
            model
                .faces
                .values()
                .filter(|f| !model.is_hidden(f.id))
                .flat_map(|f| f.vertices.iter().chain(f.holes.iter().flatten()).copied()),
        )
        .collect();
    let mut indices = BTreeMap::new();
    let mut positions = BTreeMap::new();
    for vertex in model
        .vertices
        .values()
        .filter(|v| visible_vertices.contains(&v.id))
    {
        let p = vertex.position;
        let _ = writeln!(out, "v {} {} {}", p.x, p.y, p.z);
        indices.insert(vertex.id, indices.len() + 1);
        positions.insert(p.to_array().map(f64::to_bits), indices[&vertex.id]);
    }
    for face in model.faces.values().filter(|f| !model.is_hidden(f.id)) {
        for triangle in model.face_triangles(face) {
            let [a, b, c] = triangle.map(|p| positions[&p.to_array().map(f64::to_bits)]);
            let _ = writeln!(out, "f {a} {b} {c}");
        }
    }
    for edge in model.edges.values().filter(|e| !model.is_hidden(e.id)) {
        let _ = writeln!(out, "l {} {}", indices[&edge.a], indices[&edge.b]);
    }
    if !out
        .lines()
        .any(|line| line.starts_with("f ") || line.starts_with("l "))
    {
        return Err(tr!("interchange.there_is_no_visible_geometry_to_export").into());
    }
    Ok(out)
}

#[cfg(test)]
#[path = "interchange_tests.rs"]
mod tests;
