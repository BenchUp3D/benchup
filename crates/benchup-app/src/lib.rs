mod realtime;
mod account;
mod server_config;
mod app;
mod identity;
mod brand_mark;
mod cutting_layout;
mod cutting_plan;
mod environment;
mod empty_state;
mod estimate_pdf;
mod pdf_header;
mod icons;
mod inspector_table;
pub(crate) use benchup_io::{interchange, mesh_exchange};
mod modal;
mod natural_text;
mod picking;
mod project_files;
mod release_notes;
mod renderer;
mod screenshots;
mod shadows;
#[cfg(not(target_arch = "wasm32"))]
mod viewport_workers;
mod storage;
mod theme;
#[cfg(not(target_arch = "wasm32"))]
mod updates;

#[cfg(not(target_arch = "wasm32"))]
pub use updates::initialize as initialize_updater;
#[cfg(not(target_arch = "wasm32"))]
pub use identity::{APP_ID, preferences_path};

pub use app::BenchUpApp;

#[cfg(target_arch = "wasm32")]
mod tool_cursor;

mod frame_rate;

mod system_info;

#[cfg(test)]
mod test_ui;

#[cfg(test)]
mod pdf_qa_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod update_repository;

mod reference_image_gpu;

#[cfg(target_arch = "wasm32")]
pub use storage::prepare_browser_storage;

#[cfg(not(target_arch = "wasm32"))]
pub mod desktop_files;
