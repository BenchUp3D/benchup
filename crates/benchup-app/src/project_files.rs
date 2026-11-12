//! Portable project files are separate from the local recovery/autosave document.
use benchup_core::tr;
use crate::cutting_plan::{CutReport, CuttingPlan};
use flate2::{Compression, read::MultiGzDecoder, write::GzEncoder};
use benchup_core::Document;
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "project_file_tests.rs"]
mod tests;

#[derive(Debug)]
pub enum Outcome {
    Opened(Box<Document>, Option<PathBuf>),
    #[cfg(target_arch = "wasm32")]
    BrowserOpened(Box<Document>, String, String),
    Saved(Option<PathBuf>),
    Imported {
        model: Box<benchup_core::Model>,
        selected: std::collections::BTreeSet<u64>,
        recovered: bool,
    },
    Exported,
    Cancelled,
}
pub type Pending = mpsc::Receiver<Result<Outcome, String>>;

const DEFAULT_PROJECT_EXTENSION: &str = "bup";
const PROJECT_EXTENSIONS: &[&str] = &[DEFAULT_PROJECT_EXTENSION, "k3d", "ketchup"];

pub fn file_name(name: &str) -> String {
    let name = portable_name(name);
    if has_project_extension(std::path::Path::new(&name)) {
        name
    } else {
        format!("{name}.{DEFAULT_PROJECT_EXTENSION}")
    }
}

fn has_project_extension(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|extension| {
        PROJECT_EXTENSIONS.iter().any(|supported| {
            extension
                .as_encoded_bytes()
                .eq_ignore_ascii_case(supported.as_bytes())
        })
    })
}

pub fn set_document_name(doc: &mut Document, path: &std::path::Path) {
    if doc.shared_project.is_some() {
        return;
    }
    if let Some(name) = path.file_stem() {
        doc.name = name.to_string_lossy().into_owned();
    }
}

fn portable_name(name: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim_matches([' ', '.']);
    if name.is_empty() {
        tr!("project_files.untitled_model")
    } else {
        name
    }
    .to_owned()
}

fn exchange_name(name: &str, extension: &str) -> String {
    let name = portable_name(name);
    if name
        .to_ascii_lowercase()
        .ends_with(&format!(".{extension}"))
    {
        name
    } else {
        format!("{name}.{extension}")
    }
}

/// Portable previews use PNG with a private gzip document chunk. Headless saves
/// retain gzip JSON; recovery and save snapshots stay JSON.
pub(crate) fn encode(json: &str) -> Result<Vec<u8>, String> {
    let mut value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let preview = value.as_object_mut().and_then(|v| v.remove("viewport_preview_png"));
    let json = if preview.is_some() { serde_json::to_string(&value).map_err(|e| e.to_string())? } else { json.to_owned() };
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    let compressed = encoder.finish().map_err(|e| e.to_string())?;
    if let Some(preview) = preview {
        let mut png: Vec<u8> = serde_json::from_value(preview).map_err(|e| e.to_string())?;
        // Insert a private ancillary chunk immediately before IEND. Standard PNG
        // readers ignore it and display the saved GPU viewport thumbnail.
        let end = png.len().checked_sub(12).ok_or("Invalid project preview")?;
        let mut chunk = Vec::with_capacity(compressed.len() + 12);
        chunk.extend_from_slice(&(u32::try_from(compressed.len()).map_err(|e| e.to_string())?).to_be_bytes());
        chunk.extend_from_slice(b"buPd");
        chunk.extend_from_slice(&compressed);
        chunk.extend_from_slice(&png_crc(&chunk[4..]).to_be_bytes());
        png.splice(end..end, chunk);
        Ok(png)
    } else { Ok(compressed) }
}

pub fn decode(bytes: &[u8]) -> Result<Document, String> {
    let bytes = project_payload(bytes)?;
    // Detect the content rather than the extension so all project extensions
    // accept compressed files and older, plain JSON files unchanged.
    let mut decompressed = Vec::new();
    let bytes: &[u8] = if bytes.starts_with(&[0x1f, 0x8b]) {
        MultiGzDecoder::new(bytes)
            .read_to_end(&mut decompressed)
            .map_err(|error| tr!("project_files.could_not_decompress_project", error = error))?;
        &decompressed
    } else {
        bytes
    };
    let json = std::str::from_utf8(bytes).map_err(|_| tr!("project_files.the_project_is_not_utf_8_text"))?;
    Document::from_json(json)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run(
    ctx: &eframe::egui::Context,
    action: impl FnOnce() -> Result<Outcome, String> + Send + 'static,
) -> Pending {
    let (sender, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    benchup_core::i18n::spawn_localized(move || {
        let _ = sender.send(action());
        ctx.request_repaint();
    });
    receiver
}

#[cfg(not(target_arch = "wasm32"))]
pub fn open(ctx: &eframe::egui::Context) -> Pending {
    run(ctx, || {
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr!("project_files.open_benchup_project"))
            .add_filter(tr!("project_files.benchup_project"), PROJECT_EXTENSIONS)
            .pick_file()
        else {
            return Ok(Outcome::Cancelled);
        };
        read_project(path)
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn read_project(path: PathBuf) -> Result<Outcome, String> {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        decode(&bytes).map(|mut doc| {
            if let Some(link) = &mut doc.shared_project {
                if let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) {
                    link.file_updated_at = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                }
            }
            Outcome::Opened(Box::new(doc), Some(path))
        })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn open_path(ctx: &eframe::egui::Context, path: PathBuf) -> Pending {
    run(ctx, move || read_project(path))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write(path: &std::path::Path, contents: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary
        .write_all(contents)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save(
    ctx: &eframe::egui::Context,
    path: Option<PathBuf>,
    name: String,
    contents: String,
) -> Pending {
    run(ctx, move || {
        let path = if let Some(path) = path {
            path
        } else {
            let Some(path) = rfd::FileDialog::new()
                .set_title(tr!("project_files.save_benchup_project_as"))
                .add_filter(tr!("project_files.benchup_project"), PROJECT_EXTENSIONS)
                .set_file_name(file_name(&name))
                .save_file()
            else {
                return Ok(Outcome::Cancelled);
            };
            project_path(path)?
        };
        write(&path, &encode(&contents)?)?;
        Ok(Outcome::Saved(Some(path)))
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn project_path(path: PathBuf) -> Result<PathBuf, String> {
    if has_project_extension(&path) {
        Ok(path)
    } else {
        extension_path(path, DEFAULT_PROJECT_EXTENSION)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn extension_path(path: PathBuf, extension: &str) -> Result<PathBuf, String> {
    if path.extension().is_some_and(|ext| {
        ext.as_encoded_bytes()
            .eq_ignore_ascii_case(extension.as_bytes())
    }) {
        return Ok(path);
    }
    let mut name = path.into_os_string();
    name.push(format!(".{extension}"));
    let path = PathBuf::from(name);
    // The picker confirmed overwriting the selected path, not an appended path.
    if path.exists() {
        return Err(tr!(
            "project_files.already_exists_select_that_file_in_the_save",
            path.display(),
            extension = extension,
        ));
    }
    Ok(path)
}

#[cfg(target_arch = "wasm32")]
pub fn open(ctx: &eframe::egui::Context) -> Pending {
    let (sender, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = if let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter(tr!("project_files.benchup_project"), PROJECT_EXTENSIONS)
            .pick_file()
            .await
        {
            match wasm_bindgen_futures::JsFuture::from(file.inner().array_buffer()).await {
                Ok(buffer) => decode(&js_sys::Uint8Array::new(&buffer).to_vec()).map(|mut doc| {
                    if let Some(link) = &mut doc.shared_project {
                        link.file_updated_at = (file.inner().last_modified() / 1000.) as u64;
                    }
                    set_document_name(&mut doc, std::path::Path::new(&file.file_name()));
                    Outcome::BrowserOpened(Box::new(doc), file.file_name(), format!("{}:{}:{}", file.file_name(), file.inner().size(), file.inner().last_modified()))
                }),
                Err(_) => Err(tr!("project_files.the_browser_could_not_read_this_project").into()),
            }
        } else {
            Ok(Outcome::Cancelled)
        };
        let _ = sender.send(result);
        ctx.request_repaint();
    });
    receiver
}

#[cfg(target_arch = "wasm32")]
pub fn save(
    ctx: &eframe::egui::Context,
    path: Option<PathBuf>,
    name: String,
    contents: String,
) -> Pending {
    save_payload(ctx, path, name, async move { encode(&contents) })
}

#[cfg(target_arch = "wasm32")]
pub(super) fn save_preview(
    ctx: &eframe::egui::Context,
    path: Option<PathBuf>,
    name: String,
    contents: String,
    mut capture: crate::app::screenshot_dialog::Capture,
) -> Pending {
    let parent = ctx.clone();
    save_payload(ctx, path, name, async move {
        loop {
            if let Some(pages) = capture.poll(&parent)? {
                let page = pages.into_iter().next().ok_or("The viewport preview is empty.")?;
                return encode(&with_preview(&contents, page)?);
            }
            let delay = js_sys::Promise::new(&mut |resolve, _reject| {
                let _ = web_sys::window().unwrap()
                    .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 16);
            });
            wasm_bindgen_futures::JsFuture::from(delay).await.map_err(|e| format!("{e:?}"))?;
        }
    })
}

#[cfg(target_arch = "wasm32")]
fn save_payload(
    ctx: &eframe::egui::Context,
    path: Option<PathBuf>,
    name: String,
    payload: impl std::future::Future<Output = Result<Vec<u8>, String>> + 'static,
) -> Pending {
    let (sender, receiver) = mpsc::channel();
    let name = file_name(
        path.as_deref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or(&name),
    );
    let launch = js_sys::Function::new_with_args(
        "name, contents, description",
        include_str!("project_save.js"),
    );
    // This future starts on the next microtask. Open the picker synchronously
    // before compression can consume the browser's transient user activation.
    let contents = wasm_bindgen_futures::future_to_promise(async move {
        let bytes = payload.await
            .map_err(|error| eframe::wasm_bindgen::JsValue::from_str(&error))?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()).into())
    });
    let result = launch.call3(
        &eframe::wasm_bindgen::JsValue::NULL,
        &eframe::wasm_bindgen::JsValue::from_str(&name),
        &contents,
        &eframe::wasm_bindgen::JsValue::from_str(tr!("project_files.benchup_project")),
    );
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            match result {
                Ok(value) if !value.is_null() => {
                    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&value))
                        .await
                        .map(|value| match value.as_string() {
                            // Browsers expose the chosen filename, not its full path.
                            Some(name) => Outcome::Saved(Some(PathBuf::from(name))),
                            None => Outcome::Cancelled,
                        })
                }
                result => {
                    result?;
                    let value = wasm_bindgen_futures::JsFuture::from(contents).await?;
                    let bytes = js_sys::Uint8Array::new(&value).to_vec();
                    download_bytes(&name, &bytes, "application/x-benchup")?;
                    Ok(Outcome::Saved(None))
                }
            }
        }
        .await
        .map_err(browser_save_error);
        let _ = sender.send(result);
        ctx.request_repaint();
    });
    receiver
}

#[cfg(target_arch = "wasm32")]
fn browser_save_error(error: eframe::wasm_bindgen::JsValue) -> String {
    js_sys::Reflect::get(&error, &"message".into())
        .ok()
        .and_then(|message| message.as_string())
        .or_else(|| error.as_string())
        .unwrap_or_else(|| tr!("project_files.the_browser_could_not_save_this_project").into())
}

#[cfg(target_arch = "wasm32")]
fn download_bytes(
    name: &str,
    contents: &[u8],
    mime: &str,
) -> Result<(), eframe::wasm_bindgen::JsValue> {
    use eframe::wasm_bindgen::{JsCast, JsValue, closure::Closure};
    let window = web_sys::window().ok_or_else(|| JsValue::from_str(tr!("project_files.no_browser_window")))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str(tr!("project_files.no_browser_document")))?;
    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str(tr!("project_files.no_browser_document_body")))?;
    let anchor = document
        .create_element("a")?
        .dyn_into::<web_sys::HtmlAnchorElement>()?;
    let url = blob_url(contents, mime)?;
    anchor.set_href(&url);
    anchor.set_download(name);
    if let Err(error) = body.append_child(&anchor) {
        let _ = web_sys::Url::revoke_object_url(&url);
        return Err(error);
    }
    anchor.click();
    anchor.remove();
    // Keep the URL alive long enough for the browser to start the download.
    let cleanup = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    let _ = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(cleanup.unchecked_ref(), 10_000);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn blob_url(contents: &[u8], mime: &str) -> Result<String, eframe::wasm_bindgen::JsValue> {
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(contents));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)?;
    web_sys::Url::create_object_url_with_blob(&blob)
}

/// Reserve the viewer during the click's user activation. Generation can then
/// run asynchronously without triggering the browser's popup blocker.
#[cfg(target_arch = "wasm32")]
pub(crate) struct PdfViewer(Option<web_sys::Window>);
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PdfViewer;

impl PdfViewer {
    pub(crate) fn reserve() -> Result<Self, String> {
        #[cfg(not(target_arch = "wasm32"))]
        { Ok(Self) }
        #[cfg(target_arch = "wasm32")]
        {
            use eframe::wasm_bindgen::JsValue;
            let window = web_sys::window().ok_or_else(|| tr!("project_files.no_browser_window").to_string())?;
            let tab = window.open_with_url_and_target("about:blank", "_blank")
                .map_err(|error| error.as_string().unwrap_or_else(|| tr!("project_files.the_browser_could_not_open_this_pdf").into()))?
                .ok_or_else(|| tr!("project_files.allow_pop_ups_for_benchup_then_generate_the").to_string())?;
            let _ = tab.set_opener(&JsValue::NULL);
            if let Some(document) = tab.document() {
                document.set_title(tr!("project_files.preparing_pdf"));
                if let Some(body) = document.body() {
                    body.set_text_content(Some(tr!("project_files.preparing_pdf")));
                    let _ = body.set_attribute("style", "margin:0;min-height:100vh;display:grid;place-content:center;font:18px system-ui,sans-serif;color:#26332b;background:#f4f6f2");
                }
            }
            Ok(Self(Some(tab)))
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn show(mut self, contents: &[u8]) -> Result<(), String> {
        let window = web_sys::window().ok_or_else(|| tr!("project_files.no_browser_window").to_string())?;
        let tab = self.0.as_ref().unwrap();
        if tab.closed().unwrap_or(true) { return Err("The PDF viewer was closed before generation finished.".into()); }
        let url = blob_url(contents, "application/pdf").map_err(|error| format!("{error:?}"))?;
        if let Err(error) = js_sys::Reflect::set(tab, &"location".into(), &url.clone().into()) {
            let _ = web_sys::Url::revoke_object_url(&url);
            return Err(format!("{error:?}"));
        }
        release_pdf_when_closed(window, self.0.take().unwrap(), url);
        Ok(())
    }
}
#[cfg(target_arch = "wasm32")]
impl Drop for PdfViewer {
    fn drop(&mut self) {
        // Warnings and failed generation must not leave an empty loading tab.
        if let Some(tab) = self.0.take() { let _ = tab.close(); }
    }
}

#[cfg(target_arch = "wasm32")]
fn release_pdf_when_closed(window: web_sys::Window, tab: web_sys::Window, url: String) {
    use eframe::wasm_bindgen::{JsCast, closure::Closure};
    // A download's short timeout is unsuitable for a viewer: keep its URL valid
    // for reload/save, then release the PDF bytes after the viewer tab closes.
    let timer_window = window.clone();
    let cleanup = Closure::once_into_js(move || {
        if tab.closed().unwrap_or(false) {
            let _ = web_sys::Url::revoke_object_url(&url);
        } else {
            release_pdf_when_closed(window, tab, url);
        }
    });
    let _ = timer_window
        .set_timeout_with_callback_and_timeout_and_arguments_0(cleanup.unchecked_ref(), 1_000);
}

#[cfg(not(target_arch = "wasm32"))]
fn open_pdf(path: &std::path::Path) -> Result<(), String> {
    // Use the OS file association, passing the absolute path as one argument.
    // No shell interpolation: spaces and metacharacters remain part of the name.
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("/usr/bin/open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    let mut child = command.arg(path).spawn().map_err(|e| e.to_string())?;
    // Some viewers stay running. Finish the export immediately and reap the
    // launcher separately rather than keeping the file-operation modal open.
    benchup_core::i18n::spawn_localized(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn import(
    ctx: &eframe::egui::Context,
    model: benchup_core::Model,
    extension: String,
) -> Pending {
    run(ctx, move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr!("project_files.import_geometry_stl_mm_obj_metres_dxf_declared"))
            .add_filter(tr!("project_files.geometry"), &[extension.as_str()])
            .pick_file()
        else {
            return Ok(Outcome::Cancelled);
        };
        if std::fs::metadata(&path).map_err(|e| e.to_string())?.len()
            > crate::interchange::MAX_SKP_BYTES as u64
        {
            return Err(tr!("project_files.geometry_import_is_limited_to_64_mib_per").into());
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or(tr!("project_files.sketchup_import"));
        (if extension == "skp" {
            crate::interchange::import_skp_with_recovery(&bytes, name)
        } else {
            crate::mesh_exchange::import(&bytes, &extension, name).map(|model| (model, false))
        })
        .and_then(|(imported, recovered)| prepare_import(model, &imported, recovered))
    })
}

#[cfg(target_arch = "wasm32")]
pub fn import(
    ctx: &eframe::egui::Context,
    model: benchup_core::Model,
    extension: String,
) -> Pending {
    let (sender, receiver) = mpsc::channel();
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = if let Some(file) = rfd::AsyncFileDialog::new()
            .set_title(tr!("project_files.import_geometry_stl_mm_obj_metres_dxf_declared"))
            .add_filter(tr!("project_files.geometry"), &[extension.as_str()])
            .pick_file()
            .await
        {
            if file.inner().size() > crate::interchange::MAX_SKP_BYTES as f64 {
                Err(tr!("project_files.geometry_import_is_limited_to_64_mib_per").into())
            } else {
                match wasm_bindgen_futures::JsFuture::from(file.inner().array_buffer()).await {
                    Ok(buffer) => {
                        let name = file.file_name();
                        let name = std::path::Path::new(&name)
                            .file_stem()
                            .and_then(|n| n.to_str())
                            .unwrap_or(tr!("project_files.sketchup_import"));
                        (if extension == "skp" {
                            crate::interchange::import_skp_async(
                                &js_sys::Uint8Array::new(&buffer).to_vec(),
                                name,
                            )
                            .await
                        } else {
                            crate::mesh_exchange::import(
                                &js_sys::Uint8Array::new(&buffer).to_vec(),
                                &extension,
                                name,
                            ).map(|model| (model, false))
                        })
                        .and_then(|(imported, recovered)| prepare_import(model, &imported, recovered))
                    }
                    Err(_) => Err(tr!("project_files.the_browser_could_not_read_this_geometry_file").into()),
                }
            }
        } else {
            Ok(Outcome::Cancelled)
        };
        let _ = sender.send(result);
        ctx.request_repaint();
    });
    receiver
}

#[cfg(not(target_arch = "wasm32"))]
pub fn export(ctx: &eframe::egui::Context, name: String, contents: Vec<u8>, stl: bool) -> Pending {
    run(ctx, move || {
        let extension = if stl { "stl" } else { "obj" };
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr!("project_files.export_geometry"))
            .add_filter(tr!("project_files.geometry"), &[extension])
            .set_file_name(exchange_name(&name, extension))
            .save_file()
        else {
            return Ok(Outcome::Cancelled);
        };
        write(&extension_path(path, extension)?, &contents)?;
        Ok(Outcome::Exported)
    })
}

#[cfg(target_arch = "wasm32")]
pub fn export(ctx: &eframe::egui::Context, name: String, contents: Vec<u8>, stl: bool) -> Pending {
    let (sender, receiver) = mpsc::channel();
    let result = download_bytes(
        &exchange_name(&name, if stl { "stl" } else { "obj" }),
        &contents,
        "application/octet-stream",
    )
    .map(|_| Outcome::Exported)
    .map_err(|e| {
        e.as_string()
            .unwrap_or_else(|| tr!("project_files.the_browser_could_not_download_this_obj_file").into())
    });
    let _ = sender.send(result);
    ctx.request_repaint();
    receiver
}

/// Prepare the complete next document on the file-operation thread on desktop;
/// committing its result on the UI thread only swaps owned models.
pub fn prepare_import(
    mut model: benchup_core::Model,
    imported: &benchup_core::Model,
    recovered: bool,
) -> Result<Outcome, String> {
    imported.validate().map_err(str::to_owned)?;
    let selected = model.paste_entities(imported, benchup_core::DVec3::ZERO);
    if selected.is_empty() {
        return Err(tr!("project_files.the_imported_geometry_cannot_be_inserted_in_this").into());
    }
    model.validate().map_err(str::to_owned)?;
    Ok(Outcome::Imported { model: Box::new(model), selected, recovered })
}

pub(crate) fn render_cut_report(
    report: CutReport,
    name: &str,
    model: &benchup_core::Model,
    selected: &std::collections::BTreeSet<u64>,
    layout_settings: crate::cutting_layout::Settings,
) -> Result<Vec<u8>, String> {
    if report == CutReport::CuttingPlan {
        layout_settings.validate()?;
    }
    let plan = CuttingPlan::collect_with_layout(model, selected, crate::cutting_layout::Settings {
        show_joinery: report == CutReport::CuttingPlan && layout_settings.show_joinery,
        ..layout_settings
    })?;
    let selection = !selected.is_empty();
    match report {
        CutReport::CuttingPlan => Ok(plan.pdf_with_layout(name, selection, true, layout_settings)),
        CutReport::WorkshopLabels => plan.workshop_labels_pdf(name, selection),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn export_screenshots(
    ctx: &eframe::egui::Context,
    name: String,
    settings: crate::screenshots::Settings,
    pages: Vec<image::RgbaImage>,
) -> Pending {
    run(ctx, move || {
        let export = crate::screenshots::encode_with_title(&settings, &pages, &name)?;
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr!("project_files.export_screenshot"))
            .add_filter(tr!("project_files.screenshot"), &[export.extension])
            .set_file_name(exchange_name(
                &tr!("project_files.screenshot_2", name = name),
                export.extension,
            ))
            .save_file()
        else {
            return Ok(Outcome::Cancelled);
        };
        write(&extension_path(path, export.extension)?, &export.bytes)?;
        Ok(Outcome::Exported)
    })
}

#[cfg(target_arch = "wasm32")]
pub fn export_screenshots(
    ctx: &eframe::egui::Context,
    name: String,
    settings: crate::screenshots::Settings,
    pages: Vec<image::RgbaImage>,
) -> Pending {
    let (sender, receiver) = mpsc::channel();
    let result = crate::screenshots::encode_with_title(&settings, &pages, &name).and_then(|export| {
        download_bytes(
            &exchange_name(&tr!("project_files.screenshot_2", name = name), export.extension),
            &export.bytes,
            export.mime,
        )
        .map(|_| Outcome::Exported)
        .map_err(|e| {
            e.as_string()
                .unwrap_or_else(|| tr!("project_files.the_browser_could_not_download_this_screenshot").into())
        })
    });
    let _ = sender.send(result);
    ctx.request_repaint();
    receiver
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save_pdf_contents(name: &str, report: Option<CutReport>, contents: &[u8]) -> Result<Outcome, String> {
    let (title, file_name) = match report {
        Some(report) => {
            let (title, suffix) = match report {
                CutReport::CuttingPlan => (tr!("project_files.export_woodworking_cutting_plan"), tr!("project_files.cutting_plan")),
                CutReport::WorkshopLabels => (tr!("project_files.export_workshop_labels"), tr!("project_files.workshop_labels")),
            };
            (title, tr!("project_files.report_name", name = name, suffix = suffix))
        }
        None => (tr!("project_files.export_time_and_cost_estimates"), tr!("project_files.estimates", name = name)),
    };
    let Some(path) = rfd::FileDialog::new().set_title(title)
        .add_filter(tr!("project_files.pdf_document"), &["pdf"])
        .set_file_name(exchange_name(&file_name, "pdf")).save_file()
    else { return Ok(Outcome::Cancelled) };
    let path = extension_path(path, "pdf")?;
    write(&path, contents)?;
    open_pdf(&path).map_err(|error| tr!("project_files.pdf_saved_to_but_its_viewer_could_not", path.display(), error = error))?;
    Ok(Outcome::Exported)
}

/// Optional PNG preview lives in portable files only, never in recovery/history.
/// The encoder moves this temporary field into a private PNG chunk.
pub(crate) fn with_preview(json: &str, image: image::RgbaImage) -> Result<String, String> {
    const MAX_PREVIEW_SIZE: u32 = 512;
    let image = image::DynamicImage::ImageRgba8(image);
    let image = if image.width() > MAX_PREVIEW_SIZE || image.height() > MAX_PREVIEW_SIZE {
        image.thumbnail(MAX_PREVIEW_SIZE, MAX_PREVIEW_SIZE)
    } else {
        image
    };
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let mut value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    value["viewport_preview_png"] = serde_json::to_value(png.into_inner()).map_err(|e| e.to_string())?;
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn extract_preview(path: &std::path::Path, output: &std::path::Path) -> Result<(), String> {
    let input = std::fs::read(path).map_err(|e| e.to_string())?;
    if !input.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("This project has no saved viewport preview. Save it in BenchUp first.".into());
    }
    let image = image::load_from_memory_with_format(&input, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    image.save_with_format(output, image::ImageFormat::Png).map_err(|e| e.to_string())
}

fn png_crc(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 { crc = (crc >> 1) ^ (0xedb88320u32 & 0u32.wrapping_sub(crc & 1)); }
    }
    !crc
}

fn project_payload(bytes: &[u8]) -> Result<&[u8], String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return Ok(bytes); }
    let mut position = 8usize;
    while let Some(header) = bytes.get(position..position.saturating_add(8)) {
        let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = position.checked_add(12).and_then(|v| v.checked_add(length)).ok_or("Invalid project chunk size")?;
        let chunk = bytes.get(position..end).ok_or("Truncated project chunk")?;
        if &header[4..8] == b"buPd" {
            let checksum = u32::from_be_bytes(chunk[8 + length..].try_into().unwrap());
            if png_crc(&chunk[4..8 + length]) != checksum { return Err("The project data checksum is invalid.".into()); }
            return Ok(&chunk[8..8 + length]);
        }
        position = end;
    }
    Err("This PNG does not contain a BenchUp project.".into())
}
