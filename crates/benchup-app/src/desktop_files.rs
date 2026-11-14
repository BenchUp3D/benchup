//! Desktop shell entry points. Thumbnail extraction and conversion never start a GPU or UI.
use benchup_core::{DVec2, Document, Model, tr};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock, PoisonError},
};
static OPEN: OnceLock<Mutex<VecDeque<PathBuf>>> = OnceLock::new();
/// The queue only holds paths, so a panic elsewhere cannot leave it
/// inconsistent; recover instead of panicking inside AppKit callbacks.
fn queue() -> MutexGuard<'static, VecDeque<PathBuf>> {
    OPEN.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}
pub fn next_open() -> Option<PathBuf> {
    queue().pop_front()
}
/// Queues a project requested by the operating system or a launch argument.
fn enqueue(path: PathBuf) {
    if let Some(path) = project_path(path) {
        push_latest(&mut queue(), path);
    }
}
/// Accepts only BenchUp projects, so unrelated launch arguments such as share
/// links are not opened as files. Relative paths become absolute so repeated
/// requests for one file match.
fn project_path(path: PathBuf) -> Option<PathBuf> {
    is_project(&path).then(|| std::path::absolute(&path).unwrap_or(path))
}
fn is_project(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("bup"))
}
/// A path requested again moves behind the others: it opens once, and the
/// document left open is the one requested last.
fn push_latest(queue: &mut VecDeque<PathBuf>, path: PathBuf) {
    queue.retain(|queued| *queued != path);
    queue.push_back(path);
}
/// Prepares document-open handling that must exist before the desktop event
/// loop starts. Call on the main thread before `eframe::run_native`.
pub fn prepare_launch() {
    #[cfg(target_os = "macos")]
    macos::observe_launch();
}
pub fn thumbnail_command() -> Option<Result<(), String>> {
    let mut args = std::env::args_os().skip(1);
    if args.next()?.to_str()? != "--thumbnail" {
        return None;
    }
    Some((|| {
        let input = args.next().ok_or("Missing project path")?;
        let output = args.next().ok_or("Missing thumbnail output path")?;
        crate::project_files::extract_preview(
            std::path::Path::new(&input),
            std::path::Path::new(&output),
        )
    })())
}
pub fn convert_command() -> Option<Result<(), String>> {
    let mut args = std::env::args_os().skip(1);
    if args.next()?.to_str()? != "--convert" {
        return None;
    }
    Some((|| {
        let input = args.next().ok_or(tr!("desktop_files.missing_sketchup_file_path"))?;
        let output = args.next().ok_or(tr!("desktop_files.missing_project_output_path"))?;
        convert_skp(std::path::Path::new(&input), std::path::Path::new(&output))
    })())
}
/// Same document as File > Import into an empty model, then Save, without a preview.
fn convert_skp(input: &std::path::Path, output: &std::path::Path) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|e| e.to_string())?;
    let name = input
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or(tr!("project_files.sketchup_import"));
    let imported = crate::interchange::import_skp(&bytes, name)?;
    let crate::project_files::Outcome::Imported { model, selected, .. } =
        crate::project_files::prepare_import(Model::default(), &imported, false)?
    else {
        unreachable!("prepare_import always imports")
    };
    let mut doc = Document {
        model: *model,
        selection: selected,
        ..Document::default()
    };
    doc.history.record_applied_attributed(
        tr!("projects.import_sketchup"),
        Model::default(),
        &doc.model,
        None,
        None,
    );
    doc.camera.fit(doc.model.bounds(), DVec2::new(1440., 900.));
    crate::project_files::set_document_name(&mut doc, output);
    let json = serde_json::to_string(&doc).map_err(|e| e.to_string())?;
    crate::project_files::write(output, &crate::project_files::encode(&json)?)
}
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "desktop_file_tests.rs"]
mod tests;
pub fn initialize() {
    // Once the macOS launch handler is installed, AppKit delivers launch
    // arguments through `application:openFiles:`; reading them again here
    // would queue them twice.
    #[cfg(target_os = "macos")]
    let read_arguments = !macos::installed();
    #[cfg(not(target_os = "macos"))]
    let read_arguments = true;
    if read_arguments {
        for arg in std::env::args_os().skip(1) {
            enqueue(PathBuf::from(arg));
        }
        // Launch documents have already been dispatched at this point, but
        // documents opened while BenchUp runs still reach the delegate.
        #[cfg(target_os = "macos")]
        macos::install_on_main_thread();
    }
    // Register the installed path on launch, including after an updater move.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    std::thread::spawn(|| {
        let _ = register();
    });
}
#[cfg(target_os = "linux")]
fn register() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?);
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or(
            PathBuf::from(std::env::var_os("HOME").ok_or("Missing home directory")?)
                .join(".local/share"),
        );
    if !data.is_absolute() {
        return Err("XDG_DATA_HOME must be absolute".into());
    }
    let path = exe.to_string_lossy();
    if path.contains(['\n', '\r', '%']) {
        return Err("Unsupported desktop executable path".into());
    }
    let quoted = format!(
        "\"{}\"",
        path.replace('\\', "\\\\\\\\")
            .replace('"', "\\\\\"")
            .replace('`', "\\\\`")
            .replace('$', "\\\\$")
    );
    let files = [
        (
            "applications/io.github.BenchUp3D.BenchUp.desktop",
            format!(
                "[Desktop Entry]\nType=Application\nName=BenchUp\nExec={quoted} %f\nIcon=io.github.BenchUp3D.BenchUp\nMimeType=application/x-benchup;\nCategories=Graphics;3DGraphics;\nTerminal=false\nStartupWMClass=io.github.BenchUp3D.BenchUp\n"
            ),
        ),
        (
            "mime/packages/io.github.BenchUp3D.BenchUp.xml",
            include_str!("../../../packaging/linux/io.github.BenchUp3D.BenchUp.xml").to_owned(),
        ),
        (
            "thumbnailers/benchup.thumbnailer",
            format!(
                "[Thumbnailer Entry]\nExec={quoted} --thumbnail %i %o\nMimeType=application/x-benchup;\n"
            ),
        ),
    ];
    let mut changed = false;
    for (relative, contents) in files {
        let target = data.join(relative);
        if std::fs::read_to_string(&target).ok().as_deref() != Some(&contents) {
            std::fs::create_dir_all(target.parent().unwrap())?;
            std::fs::write(target, contents)?;
            changed = true;
        }
    }
    // Wayland shells resolve the taskbar icon through the desktop entry's
    // app ID; the window icon alone is insufficient. Install into the standard
    // fallback theme so this also works when launching an unpackaged binary.
    let icon = data.join("icons/hicolor/256x256/apps/io.github.BenchUp3D.BenchUp.png");
    let icon_bytes = include_bytes!("../../../assets/branding/icon-256.png");
    if std::fs::read(&icon).ok().as_deref() != Some(icon_bytes.as_slice()) {
        std::fs::create_dir_all(icon.parent().unwrap())?;
        std::fs::write(&icon, icon_bytes)?;
        changed = true;
        let _ = std::process::Command::new("gtk-update-icon-cache")
            .args(["--force", "--ignore-theme-index"])
            .arg(data.join("icons/hicolor"))
            .status();
    }
    if changed {
        for (command, directory) in [
            ("update-mime-database", "mime"),
            ("update-desktop-database", "applications"),
        ] {
            let _ = std::process::Command::new(command)
                .arg(data.join(directory))
                .status();
        }
    }
    Ok(())
}
#[cfg(target_os = "windows")]
fn register() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::windows::process::CommandExt;
    let exe = std::env::current_exe()?;
    let values = [
        (
            r"Software\Classes\.bup\OpenWithProgids",
            "BenchUp.Project",
            "".to_owned(),
        ),
        (
            r"Software\Classes\BenchUp.Project",
            "",
            "BenchUp project".to_owned(),
        ),
        (
            r"Software\Classes\BenchUp.Project\shell\open\command",
            "",
            format!("\"{}\" \"%1\"", exe.display()),
        ),
        (
            r"Software\Classes\.bup",
            "PerceivedType",
            "image".to_owned(),
        ),
        (
            r"Software\Classes\.bup\shellex\{e357fccd-a995-4576-b01f-234630154e96}",
            "",
            "{C7657C4A-9F68-40fa-A4DF-96BC08EB3551}".to_owned(),
        ),
        (
            r"Software\Classes\BenchUp.Project\DefaultIcon",
            "",
            format!("\"{}\",0", exe.display()),
        ),
    ];
    for (key, name, value) in values {
        let mut cmd = std::process::Command::new("reg.exe");
        cmd.creation_flags(0x08000000)
            .args(["add", &format!("HKCU\\{key}"), "/f", "/t", "REG_SZ"]);
        if name.is_empty() {
            cmd.arg("/ve");
        } else {
            cmd.args(["/v", name]);
        }
        if !cmd.args(["/d", &value]).status()?.success() {
            return Err("Could not register BenchUp project files".into());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2::{
        MainThreadMarker, MainThreadOnly, define_class, ffi, msg_send,
        rc::Retained,
        runtime::{AnyObject, Imp, NSObject, Sel},
        sel,
    };
    use objc2_app_kit::{
        NSApplication, NSApplicationDelegateReply, NSApplicationWillFinishLaunchingNotification,
    };
    use objc2_foundation::{NSArray, NSNotification, NSNotificationCenter, NSString};
    use std::sync::atomic::{AtomicBool, Ordering};

    static INSTALLED: AtomicBool = AtomicBool::new(false);
    /// Whether `application:openFiles:` was added to the app delegate.
    pub fn installed() -> bool {
        INSTALLED.load(Ordering::Relaxed)
    }

    type OpenFiles = unsafe extern "C-unwind" fn(
        *mut AnyObject,
        Sel,
        *mut NSApplication,
        *mut NSArray<NSString>,
    );
    unsafe extern "C-unwind" fn open_files(
        _: *mut AnyObject,
        _: Sel,
        app: *mut NSApplication,
        files: *mut NSArray<NSString>,
    ) {
        // SAFETY: AppKit passes the application and an array of path strings.
        if let Some(files) = unsafe { files.as_ref() } {
            for file in files.to_vec() {
                enqueue(PathBuf::from(file.to_string()));
            }
        }
        if let Some(app) = unsafe { app.as_ref() } {
            app.replyToOpenOrPrint(NSApplicationDelegateReply::Success);
        }
    }

    /// Adds `application:openFiles:` to winit's application delegate class.
    fn install(mtm: MainThreadMarker) {
        let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() else {
            return;
        };
        let object: &AnyObject = delegate.as_ref();
        let class = object.class();
        // SAFETY: the implementation matches the `v@:@@` encoding of
        // `application:openFiles:`; the runtime requires an untyped `Imp`.
        let added = unsafe {
            ffi::class_addMethod(
                class as *const _ as *mut _,
                sel!(application:openFiles:),
                std::mem::transmute::<OpenFiles, Imp>(open_files),
                c"v@:@@".as_ptr(),
            )
        };
        // Fails when the delegate already implements the method, in which case
        // launch arguments are read directly instead.
        if added.as_bool() {
            INSTALLED.store(true, Ordering::Relaxed);
        }
    }
    pub fn install_on_main_thread() {
        if let Some(mtm) = MainThreadMarker::new()
            && !installed()
        {
            install(mtm);
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "BenchUpLaunchObserver"]
        struct LaunchObserver;

        impl LaunchObserver {
            #[unsafe(method(benchUpWillFinishLaunching:))]
            fn will_finish_launching(&self, _notification: &NSNotification) {
                install(self.mtm());
            }
        }
    );

    /// When macOS launches BenchUp to open a document, AppKit delivers the
    /// open-documents event during `finishLaunching`, before eframe creates the
    /// app. winit only sets its delegate when the event loop is built, so add the
    /// handler from the will-finish-launching notification: it fires after the
    /// delegate exists and before AppKit dispatches the launch documents.
    pub fn observe_launch() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let observer: Retained<LaunchObserver> =
            unsafe { msg_send![LaunchObserver::alloc(mtm), init] };
        // SAFETY: the observer implements the selector with a notification argument.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &observer,
                sel!(benchUpWillFinishLaunching:),
                Some(NSApplicationWillFinishLaunchingNotification),
                None,
            );
        }
        // The notification center does not retain observers.
        std::mem::forget(observer);
    }
}
