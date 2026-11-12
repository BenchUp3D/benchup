//! IndexedDB-backed browser recovery, loaded before constructing the app.
use super::*;
use eframe::wasm_bindgen::{self, prelude::*};
use std::cell::RefCell;

#[wasm_bindgen(module = "/src/browser_storage.js")]
extern "C" {
    #[wasm_bindgen(js_name = loadRecovery)]
    fn load_recovery() -> js_sys::Promise;
    #[wasm_bindgen(catch, js_name = readRecovery)]
    fn read_recovery(key: &str) -> Result<Option<String>, JsValue>;
    #[wasm_bindgen(catch, js_name = writeRecovery)]
    fn write_recovery(key: &str, value: &str, snapshot: bool) -> Result<(), JsValue>;
    #[wasm_bindgen(js_name = recoveryPending)]
    fn recovery_pending() -> bool;
    #[wasm_bindgen(js_name = takeRecoveryResult)]
    fn take_recovery_result() -> Option<String>;
}
thread_local! {
    static STARTUP_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}
fn error(value: JsValue) -> String {
    let message = js_sys::Reflect::get(&value, &JsValue::from_str("message"))
        .ok()
        .and_then(|v| v.as_string())
        .or_else(|| value.as_string());
    message.unwrap_or_else(|| tr!("storage.browser_unavailable", error = format!("{value:?}")))
}
pub async fn prepare() {
    if let Err(value) = wasm_bindgen_futures::JsFuture::from(load_recovery()).await {
        STARTUP_ERROR.with(|saved| *saved.borrow_mut() = Some(error(value)));
    }
}
pub fn load() -> Result<Option<Document>, String> {
    *AUTOSAVE_TOKEN.lock().unwrap() = None;
    if let Some(error) = STARTUP_ERROR.with(|saved| saved.borrow().clone()) {
        return Err(error);
    }
    let Some(json) = read_recovery("benchup.document.v1").map_err(error)? else {
        return Ok(None);
    };
    let selection = read_recovery("benchup.selection.v1").map_err(error)?;
    let edits = read_recovery("benchup.edits.v1").map_err(error)?;
    restore_autosave(&json, selection.as_deref(), edits.as_deref()).map(Some)
}
pub fn save(doc: &Document) -> Result<(), String> {
    let (json, token) = autosave_json(doc)?;
    // Update the checkpoint base only after the transaction is successfully
    // submitted. An asynchronous failure invalidates it through poll_save().
    write_recovery("benchup.document.v1", &json, true).map_err(error)?;
    *AUTOSAVE_TOKEN.lock().unwrap() = Some(token);
    remember_base(doc);
    Ok(())
}
pub fn save_edits(doc: &Document) -> Result<(), String> {
    let Some(json) = edits_json(doc)? else {
        return save(doc);
    };
    write_recovery("benchup.edits.v1", &json, false).map_err(error)?;
    save_selection(doc)
}
pub fn save_selection(doc: &Document) -> Result<(), String> {
    let Some(json) = selection_json(doc)? else {
        return save(doc);
    };
    write_recovery("benchup.selection.v1", &json, false).map_err(error)
}
pub fn is_pending() -> bool {
    recovery_pending()
}
pub fn poll_save() -> Option<Result<(), String>> {
    take_recovery_result().map(|message| {
        if message.is_empty() {
            Ok(())
        } else {
            invalidate_selection_source();
            Err(message)
        }
    })
}
