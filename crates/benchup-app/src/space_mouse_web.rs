//! Browser device ownership and callbacks; HID decoding stays in space_mouse.
use super::{Shared, egui};
use eframe::wasm_bindgen::{self, JsCast, closure::Closure, prelude::*};
use js_sys::{Function, Uint8Array};
use std::sync::{Arc, Mutex};

#[wasm_bindgen(module = "/src/space_mouse_web.js")]
extern "C" {
    type Connection;

    #[wasm_bindgen(js_name = createSpaceMouse)]
    fn create(on_report: &Function, on_reset: &Function, wake: &Function) -> Connection;

    #[wasm_bindgen(method)]
    fn sync(this: &Connection, enabled: bool, focused: bool);
    #[wasm_bindgen(method)]
    fn connect(this: &Connection);
    #[wasm_bindgen(method)]
    fn dispose(this: &Connection);
    #[wasm_bindgen(method, getter)]
    fn device(this: &Connection) -> Option<String>;
    #[wasm_bindgen(method, getter)]
    fn status(this: &Connection) -> String;
    #[wasm_bindgen(method, getter, js_name = canConnect)]
    fn can_connect(this: &Connection) -> bool;
}

pub(super) struct Browser {
    connection: Connection,
    // Own the callbacks for precisely as long as the JS controller can use them.
    _on_report: Closure<dyn FnMut(Uint8Array)>,
    _on_reset: Closure<dyn FnMut()>,
    _wake: Closure<dyn FnMut()>,
}

impl Browser {
    pub fn new(ctx: egui::Context, shared: Arc<Mutex<Shared>>) -> Self {
        let state = shared.clone();
        let repaint = ctx.clone();
        let on_report = Closure::wrap(Box::new(move |report: Uint8Array| {
            if report.length() > 13 {
                return;
            }
            let changed = state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .report(&report.to_vec());
            if changed {
                repaint.request_repaint();
            }
        }) as Box<dyn FnMut(Uint8Array)>);
        let on_reset = Closure::wrap(Box::new(move || {
            shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear_input();
        }) as Box<dyn FnMut()>);
        let wake = Closure::wrap(Box::new(move || ctx.request_repaint()) as Box<dyn FnMut()>);
        let connection = create(
            on_report.as_ref().unchecked_ref(),
            on_reset.as_ref().unchecked_ref(),
            wake.as_ref().unchecked_ref(),
        );
        Self {
            connection,
            _on_report: on_report,
            _on_reset: on_reset,
            _wake: wake,
        }
    }

    pub fn sync(&self, enabled: bool, focused: bool) {
        self.connection.sync(enabled, focused);
    }

    pub fn connect(&self) {
        self.connection.connect();
    }

    pub fn can_connect(&self) -> bool {
        self.connection.can_connect()
    }

    pub fn device(&self) -> Option<String> {
        self.connection.device()
    }

    pub fn status(&self) -> String {
        super::browser_status_label(&self.connection.status(), self.connection.device().as_deref())
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.connection.dispose();
    }
}
