#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ketchup_core::tr;
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // Installer hooks must run before opening windows or loading user data.
    ketchup_app::initialize_updater();

    let viewport = eframe::egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_inner_size([1440., 900.])
        .with_min_inner_size([800., 540.])
        .with_icon(
            eframe::icon_data::from_png_bytes(include_bytes!(
                "../../../assets/branding/icon-256.png"
            ))
            .expect(tr!("startup.the_bundled_ketchup3d_icon_must_be_a_valid")),
        );
    // GNOME/Wayland resolves the icon through the matching .desktop file.
    #[cfg(target_os = "linux")]
    let viewport = viewport.with_app_id("io.github.Ketchup3D.Ketchup");
    // eframe also uses the app ID for preferences; preserve the former default.
    #[cfg(not(target_os = "linux"))]
    let viewport = viewport.with_app_id("KetchUp — 3D workspace");

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 32,
        // Keep CPU preparation and the 3D GPU passes in flight together.
        // eframe's one-frame GUI default misses refresh deadlines while orbiting.
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration::default()
            .with_surface_config(eframe::egui_wgpu::SurfaceConfig::HIGH_THROUGHPUT),
        viewport,
        persistence_path: std::env::var_os("KETCHUP_VIEWPORT_BENCHMARK")
            .and_then(|_| std::env::var_os("KETCHUP_DATA_DIR"))
            .map(|path| std::path::PathBuf::from(path).join("preferences.ron")),
        ..Default::default()
    };
    eframe::run_native(
        tr!("startup.ketchup3d_3d_workspace"),
        options,
        Box::new(|cc| Ok(Box::new(ketchup_app::KetchupApp::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("A browser document is required");
        let canvas = document
            .get_element_by_id("ketchup-canvas")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("Missing viewport canvas");
        let _ = canvas.set_attribute("aria-label", tr!("web.canvas_label"));
        let options = eframe::WebOptions {
            renderer: eframe::Renderer::Wgpu,
            depth_buffer: 32,
            ..Default::default()
        };
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(|cc| Ok(Box::new(ketchup_app::KetchupApp::new(cc)))),
            )
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(())=>loading.remove(),
                Err(_)=>loading.set_text_content(Some(tr!("startup.ketchup3d_could_not_start_its_gpu_renderer_enable"))),
            }
        }
    });
}
