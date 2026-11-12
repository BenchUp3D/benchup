//! Show feedback immediately and generate PDF reports off the UI thread.
use super::*;
use crate::project_files::{self, Pending, PdfViewer};
#[cfg(target_arch = "wasm32")]
use crate::project_files::Outcome;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Estimates,
    CutReport(CutReport, bool),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) enum Request {
    Estimates(time_estimates::PdfInput),
    CutReport {
        report: CutReport,
        name: String,
        model: std::sync::Arc<Model>,
        selected: BTreeSet<u64>,
        layout: crate::cutting_layout::Settings,
    },
}
impl Request {
    fn render(self) -> Result<Vec<u8>, String> {
        match self {
            Self::Estimates(input) => input.render(),
            Self::CutReport { report, name, model, selected, layout } =>
                project_files::render_cut_report(report, &name, &model, &selected, layout),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn start(ctx: &egui::Context, name: String, request: Request, _viewer: PdfViewer) -> Pending {
    let report = match &request { Request::CutReport { report, .. } => Some(*report), _ => None };
    project_files::run(ctx, move || {
        // Give the click's loading frame time to reach the screen even when
        // validation immediately produces a warning.
        std::thread::sleep(Duration::from_millis(50));
        let bytes = request.render()?;
        project_files::save_pdf_contents(&name, report, &bytes)
    })
}

#[cfg(target_arch = "wasm32")]
#[eframe::wasm_bindgen::prelude::wasm_bindgen]
pub fn benchup_generate_pdf(source: &str) -> Result<Vec<u8>, eframe::wasm_bindgen::JsValue> {
    serde_json::from_str::<Request>(source).map_err(|error| error.to_string())
        .and_then(Request::render).map_err(|error| error.into())
}

#[cfg(target_arch = "wasm32")]
pub(super) fn start(ctx: &egui::Context, _name: String, request: Request, viewer: PdfViewer) -> Pending {
    use eframe::wasm_bindgen::{JsCast, JsValue};
    let (sender, receiver) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        // A timer keeps working when the reserved viewer becomes the active
        // tab; waiting for an editor animation frame would stall in that case.
        let delay = js_sys::Promise::new(&mut |resolve, _reject| {
            let _ = web_sys::window().unwrap().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 50);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(delay).await;
        let promise = serde_json::to_string(&request).map_err(|error| error.to_string())
            .and_then(|source| {
                js_sys::Function::new_with_args("source", include_str!("pdf_worker.js"))
                    .call1(&JsValue::NULL, &source.into())
                    .and_then(|value| value.dyn_into::<js_sys::Promise>()).map_err(js_error)
            });
        let result = match promise {
            Ok(promise) => wasm_bindgen_futures::JsFuture::from(promise).await.map_err(js_error)
                .and_then(|value| value.dyn_into::<js_sys::Uint8Array>()
                    .map_err(|_| tr!("pdf_export.could_not_read").to_string()))
                .and_then(|bytes| viewer.show(&bytes.to_vec())).map(|_| Outcome::Exported),
            Err(error) => Err(error),
        };
        let _ = sender.send(result);
        ctx.request_repaint();
    });
    receiver
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: eframe::wasm_bindgen::JsValue) -> String {
    error.as_string().or_else(|| js_sys::Reflect::get(&error, &"message".into()).ok()
        .and_then(|message| message.as_string())).unwrap_or_else(|| tr!("pdf_export.could_not_generate").into())
}
