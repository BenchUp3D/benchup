//! Desktop shell entry points. Thumbnail extraction never starts a GPU or UI.
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};
static OPEN: OnceLock<Mutex<VecDeque<PathBuf>>> = OnceLock::new();
fn queue() -> &'static Mutex<VecDeque<PathBuf>> {
    OPEN.get_or_init(Default::default)
}
pub fn next_open() -> Option<PathBuf> {
    queue().lock().ok()?.pop_front()
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
pub fn initialize() {
    for arg in std::env::args_os().skip(1) {
        let path = PathBuf::from(arg);
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("bup"))
        {
            queue().lock().unwrap().push_back(path);
        }
    }
    #[cfg(target_os = "macos")]
    unsafe {
        macos::install();
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
    use std::ffi::{CStr, c_char, c_void};
    type Id = *mut c_void;
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Id;
        fn object_getClass(object: Id) -> Id;
        fn class_addMethod(
            class: Id,
            selector: Id,
            method: unsafe extern "C" fn(Id, Id, Id, Id),
            types: *const c_char,
        ) -> bool;
        fn objc_msgSend();
    }
    unsafe fn send(object: Id, name: &CStr) -> Id {
        let f: unsafe extern "C" fn(Id, Id) -> Id =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { f(object, sel_registerName(name.as_ptr())) }
    }
    unsafe extern "C" fn open_files(_: Id, _: Id, app: Id, files: Id) {
        unsafe {
            let count = send(files, c"count") as usize;
            let at: unsafe extern "C" fn(Id, Id, usize) -> Id =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            for index in 0..count {
                let file = at(files, sel_registerName(c"objectAtIndex:".as_ptr()), index);
                let bytes = send(file, c"UTF8String") as *const c_char;
                if !bytes.is_null() {
                    queue().lock().unwrap().push_back(PathBuf::from(
                        CStr::from_ptr(bytes).to_string_lossy().as_ref(),
                    ));
                }
            }
            let reply: unsafe extern "C" fn(Id, Id, usize) =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            reply(app, sel_registerName(c"replyToOpenOrPrint:".as_ptr()), 0);
        }
    }
    pub unsafe fn install() {
        unsafe {
            let app = send(
                objc_getClass(c"NSApplication".as_ptr()),
                c"sharedApplication",
            );
            let delegate = send(app, c"delegate");
            class_addMethod(
                object_getClass(delegate),
                sel_registerName(c"application:openFiles:".as_ptr()),
                open_files,
                c"v@:@@".as_ptr(),
            );
        }
    }
}
