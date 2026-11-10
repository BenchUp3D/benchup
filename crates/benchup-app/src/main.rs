#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use benchup_core::tr;
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // Installer hooks must run before opening windows or loading user data.
    benchup_app::initialize_updater();

    let viewport = eframe::egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_inner_size([1440., 900.])
        .with_min_inner_size([800., 540.])
        .with_icon(
            eframe::icon_data::from_png_bytes(include_bytes!(
                "../../../assets/branding/icon-256.png"
            ))
            .expect(tr!("startup.the_bundled_benchup_icon_must_be_a_valid")),
        );
    // Migrate preferences before eframe opens its storage under the new ID.
    let viewport = viewport.with_app_id(benchup_app::APP_ID);
    let preferences_path = benchup_app::preferences_path()
        .map_err(|error| eframe::Error::AppCreation(error.into()))?;

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 32,
        // Keep CPU preparation and the 3D GPU passes in flight together.
        // eframe's one-frame GUI default misses refresh deadlines while orbiting.
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration::default()
            .with_surface_config(eframe::egui_wgpu::SurfaceConfig::HIGH_THROUGHPUT),
        viewport,
        persistence_path: Some(preferences_path),
        ..Default::default()
    };
    eframe::run_native(
        tr!("startup.benchup_3d_workspace"),
        options,
        Box::new(|cc| Ok(Box::new(benchup_app::BenchUpApp::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // The inspector worker imports this same module for pure geometry exports.
    if web_sys::window().is_none() {
        return;
    }
    use eframe::wasm_bindgen::JsCast;
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("A browser document is required");
        let canvas = document
            .get_element_by_id("benchup-canvas")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("Missing viewport canvas");
        let _ = canvas.set_attribute("aria-label", tr!("web.canvas_label"));
        let options = eframe::WebOptions {
            renderer: eframe::Renderer::Wgpu,
            depth_buffer: 32,
            ..Default::default()
        };
        benchup_app::prepare_browser_storage().await;
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(|cc| Ok(Box::new(benchup_app::BenchUpApp::new(cc)))),
            )
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(()) => loading.remove(),
                Err(_) => loading.set_text_content(Some(tr!(
                    "startup.benchup_could_not_start_its_gpu_renderer_enable"
                ))),
            }
        }
    });
}
