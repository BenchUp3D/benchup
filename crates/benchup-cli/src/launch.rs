//! `benchup open`: hand a project to the BenchUp desktop app.
//!
//! The CLI does not link the app. It asks the operating system to open the
//! file with the installed app (macOS by bundle id, Linux and Windows through
//! the `.bup` file association), or starts a given executable directly. No
//! shell parses the path, so names with `&`, `%` or `^` open as they are. If the
//! app is already running, macOS hands it the file; the app reads the file
//! from disk again, asking first if the open project has unsaved changes.
//!
//! On macOS, a document handed to an app that is still starting arrives
//! before BenchUp can take it, and macOS reports that BenchUp "cannot open
//! files in the BenchUp project format". So when the app is not running yet,
//! the file is passed as a launch argument instead, which the app reads at
//! start-up.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const DOWNLOAD_URL: &str = "https://github.com/BenchUp3D/editor/releases/latest";

#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "io.github.BenchUp3D.BenchUp";

/// An absolute path to an existing `.bup` file, which is the only kind of
/// argument the app opens.
fn project_path(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("bup")) {
        return Err(format!("{} is not a .bup project; BenchUp only opens .bup files", path.display()));
    }
    let absolute = path.canonicalize().map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    if !absolute.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    Ok(plain_path(absolute))
}

/// Windows `canonicalize` gives a verbatim `\\?\C:\...` path, which file
/// associations and many apps do not accept; this is the usual form of it.
#[cfg(windows)]
fn plain_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy().into_owned();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{share}"))
    } else if let Some(local) = text.strip_prefix(r"\\?\").filter(|rest| rest.as_bytes().get(1) == Some(&b':')) {
        PathBuf::from(local)
    } else {
        path
    }
}

#[cfg(not(windows))]
fn plain_path(path: PathBuf) -> PathBuf {
    path
}

/// Whether the installed BenchUp is running (macOS lists it by bundle id).
#[cfg(target_os = "macos")]
fn app_running() -> bool {
    Command::new("lsappinfo")
        .args(["find", &format!("bundleid={BUNDLE_ID}")])
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
}

/// `open` arguments: hand the file to a running app, or start the app with
/// the file as its argument.
#[cfg(target_os = "macos")]
fn open_arguments(path: &Path, running: bool) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec!["-b".into(), BUNDLE_ID.into()];
    if !running {
        args.push("--args".into());
    }
    args.push(path.into());
    args
}

/// The command the operating system uses to open `path` in the installed app.
fn system_command(path: &Path) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.args(open_arguments(path, app_running()));
        command
    }
    // Explorer opens the file with its association without a shell parsing
    // the path (`cmd /C start` would split it at `&` and expand `%NAME%`).
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("explorer.exe");
        command.arg(path);
        command
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    }
}

/// Opens `path` in BenchUp: in `app` (an executable) if given, otherwise in
/// the installed app.
pub fn open(path: &str, app: Option<&str>) -> Result<Value, String> {
    let path = project_path(path)?;
    let opened = path.display().to_string();
    if let Some(app) = app {
        // Started detached: the app keeps running after this command exits,
        // and its output does not mix with this command's JSON.
        Command::new(app)
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start {app}: {e}"))?;
        return Ok(json!({"ok": true, "opened": opened, "app": app}));
    }
    let mut command = system_command(&path);
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {program}: {e}. Is BenchUp installed? {DOWNLOAD_URL}"))?;
    // Explorer exits with 1 even when it opened the file.
    if !output.status.success() && !cfg!(target_os = "windows") {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!(
            "could not open {opened} in BenchUp ({program}: {detail}). \
             Install the desktop app from {DOWNLOAD_URL}, or pass --app <executable>"
        ));
    }
    Ok(json!({"ok": true, "opened": opened, "app": "BenchUp"}))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn a_running_app_gets_the_file_and_a_starting_one_gets_it_as_an_argument() {
        let path = Path::new("/tmp/a b&c.bup");
        assert_eq!(open_arguments(path, true), ["-b", BUNDLE_ID, "/tmp/a b&c.bup"]);
        assert_eq!(open_arguments(path, false), ["-b", BUNDLE_ID, "--args", "/tmp/a b&c.bup"]);
    }
}
