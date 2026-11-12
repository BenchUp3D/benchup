//! End-to-end tests of the `benchup` binary against real project files.
use benchup_tools::files;
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn benchup(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_benchup"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run benchup");
    let mut input = child.stdin.take().unwrap();
    if let Some(text) = stdin {
        input.write_all(text.as_bytes()).unwrap();
    }
    drop(input);
    child.wait_with_output().unwrap()
}

fn json_out(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!("stdout is not JSON: {}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
    })
}

fn new_project(dir: &Path) -> String {
    let path = dir.join("shelf.bup").display().to_string();
    let output = benchup(&["new", &path, "--name", "Shelf"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    path
}

fn board(name: &str, origin: [f64; 3], size: [f64; 3]) -> Value {
    json!({"name": name, "origin_mm": origin, "size_mm": size})
}

fn object_count(path: &str) -> u64 {
    json_out(&benchup(&["scene", path], None))["object_count"].as_u64().unwrap()
}

#[test]
fn a_call_edits_the_file_as_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_project(dir.path());
    let args = board("Side", [0., 0., 0.], [18., 300., 900.]).to_string();
    let output = benchup(&["call", &path, "create_board", &args], None);
    assert!(output.status.success());
    let result = json_out(&output);
    assert_eq!(result["ok"], true);
    assert_eq!(result["objects"][0]["size_mm"], json!([18., 300., 900.]));

    let doc = files::decode(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(doc.name, "Shelf");
    doc.model.validate().unwrap();
    assert_eq!(doc.history.labels().collect::<Vec<_>>(), ["Agent: create_board"]);
    assert_eq!(object_count(&path), 1);
}

#[test]
fn a_batch_is_all_or_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_project(dir.path());
    let good = format!(
        "{}\n# comment\n\n{}\n",
        json!({"tool": "create_board", "arguments": board("Left", [0., 0., 0.], [18., 300., 900.])}),
        json!({"tool": "create_board", "arguments": board("Right", [782., 0., 0.], [18., 300., 900.])}),
    );
    let output = benchup(&["call", &path, "--batch", "-"], Some(&good));
    assert!(output.status.success());
    assert_eq!(json_out(&output)["results"].as_array().unwrap().len(), 2);
    let doc = files::decode(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(doc.history.labels().collect::<Vec<_>>(), ["Agent: 2 changes"]);

    let before = std::fs::read(&path).unwrap();
    let bad = format!(
        "{}\n{}\n",
        json!({"tool": "create_board", "arguments": board("Top", [0., 0., 900.], [800., 300., 18.])}),
        json!({"tool": "move", "arguments": {"ids": [999_999], "delta_mm": [0, 0, 1]}}),
    );
    let output = benchup(&["call", &path, "--batch", "-"], Some(&bad));
    assert_eq!(output.status.code(), Some(1));
    let result = json_out(&output);
    assert_eq!(result["failed_call"], 2);
    assert_eq!(result["tool"], "move");
    assert_eq!(std::fs::read(&path).unwrap(), before, "a failed batch must not touch the file");
}

#[test]
fn tool_errors_exit_1_and_leave_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_project(dir.path());
    let before = std::fs::read(&path).unwrap();
    let output = benchup(&["call", &path, "create_board", r#"{"name": "No size"}"#], None);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json_out(&output)["ok"], false);
    assert_eq!(std::fs::read(&path).unwrap(), before);

    let output = benchup(&["call", &path, "create_board", "{not json"], None);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn undo_and_redo_step_through_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_project(dir.path());
    let args = board("Side", [0., 0., 0.], [18., 300., 900.]).to_string();
    benchup(&["call", &path, "create_board", &args], None);

    let undo = benchup(&["undo", &path], None);
    assert_eq!(json_out(&undo), json!({"ok": true, "undone": "Agent: create_board"}));
    assert_eq!(object_count(&path), 0);
    assert_eq!(benchup(&["undo", &path], None).status.code(), Some(1));

    let redo = benchup(&["redo", &path], None);
    assert_eq!(json_out(&redo)["ok"], true);
    assert_eq!(object_count(&path), 1);
}

#[test]
fn new_refuses_to_replace_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_project(dir.path());
    assert_eq!(benchup(&["new", &path], None).status.code(), Some(2));
    assert!(benchup(&["new", &path, "--force"], None).status.success());
}

#[test]
fn tools_lists_schemas() {
    let list = json_out(&benchup(&["tools"], None));
    let names: Vec<_> = list.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"create_board") && names.contains(&"get_scene"));
    let one = json_out(&benchup(&["tools", "create_board"], None));
    assert_eq!(one["inputSchema"]["required"], json!(["name", "origin_mm", "size_mm"]));
    assert_eq!(benchup(&["tools", "nope"], None).status.code(), Some(2));
}

#[test]
fn mcp_serves_tools_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_project(dir.path());
    let messages = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-06-18", "clientInfo": {"name": "test"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "create_board", "arguments": board("Side", [0., 0., 0.], [18., 300., 900.])}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
               "params": {"name": "move", "arguments": {"ids": [999_999], "delta_mm": [1, 0, 0]}}}),
    ];
    let input: String = messages.iter().map(|m| format!("{m}\n")).collect();
    let output = benchup(&["mcp", &path], Some(&input));
    assert!(output.status.success());
    let replies: Vec<Value> =
        String::from_utf8(output.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(replies.len(), 4, "notifications get no reply");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert!(replies[1]["result"]["tools"].as_array().unwrap().len() > 20);
    assert_eq!(replies[2]["result"]["isError"], false);
    assert_eq!(replies[3]["result"]["isError"], true);
    assert_eq!(object_count(&path), 1);
}

/// A stand-in for the desktop app that records the path it was asked to open.
#[cfg(unix)]
fn fake_app(dir: &Path) -> (String, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let record = dir.join("opened.txt");
    let app = dir.join("fake-benchup-app");
    std::fs::write(&app, format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}.tmp'\nmv '{0}.tmp' '{0}'\n", record.display()))
        .unwrap();
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
    (app.display().to_string(), record)
}

/// The app is started detached, so wait for it to record its argument.
#[cfg(unix)]
fn opened_by(record: &Path) -> String {
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(record) {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("the app was not started");
}

#[cfg(unix)]
#[test]
fn open_starts_the_app_with_the_absolute_project_path() {
    let dir = tempfile::tempdir().unwrap();
    new_project(dir.path());
    let (app, record) = fake_app(dir.path());
    // A relative path, as typed in a shell, reaches the app as an absolute one.
    let output = Command::new(env!("CARGO_BIN_EXE_benchup"))
        .args(["open", "shelf.bup", "--app", &app])
        .current_dir(dir.path())
        .env_remove("BENCHUP_APP")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = dir.path().canonicalize().unwrap().join("shelf.bup").display().to_string();
    let result = json_out(&output);
    assert_eq!(result["ok"], true);
    assert_eq!(result["opened"], expected);
    assert_eq!(opened_by(&record), expected);
}

#[cfg(unix)]
#[test]
fn open_uses_the_benchup_app_variable() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_project(dir.path());
    let (app, record) = fake_app(dir.path());
    let output = Command::new(env!("CARGO_BIN_EXE_benchup")).args(["open", &path]).env("BENCHUP_APP", &app).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(json_out(&output)["app"], app);
    assert!(opened_by(&record).ends_with("shelf.bup"));
}

#[test]
fn open_refuses_missing_files_and_other_extensions() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.bup").display().to_string();
    let output = benchup(&["open", &missing, "--app", "/nonexistent/app"], None);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot open"));

    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "not a project").unwrap();
    let output = benchup(&["open", &text.display().to_string(), "--app", "/nonexistent/app"], None);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a .bup project"));

    let output = benchup(&["open"], None);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn open_reports_an_app_that_cannot_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = new_project(dir.path());
    let output = benchup(&["open", &path, "--app", "/nonexistent/benchup-app"], None);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot start /nonexistent/benchup-app"));
}

/// A project with one group "Pair" holding two boards; returns (group, inner board).
fn grouped_project(dir: &Path) -> (String, u64, u64) {
    let path = new_project(dir);
    let batch = [
        json!({"tool": "create_board", "arguments": board("A", [0., 0., 0.], [100., 100., 20.])}),
        json!({"tool": "create_board", "arguments": board("B", [200., 0., 0.], [100., 100., 20.])}),
    ];
    let input: String = batch.iter().map(|c| format!("{c}\n")).collect();
    let created = json_out(&benchup(&["call", &path, "--batch", "-"], Some(&input)));
    let ids: Vec<u64> = created["results"].as_array().unwrap().iter().map(|r| r["created"][0].as_u64().unwrap()).collect();
    let grouped = json_out(&benchup(&["call", &path, "group", &json!({"ids": ids, "name": "Pair", "component": false}).to_string()], None));
    let group = grouped["created"][0].as_u64().unwrap();
    let inner = grouped["members"][0]["id"].as_u64().unwrap();
    (path, group, inner)
}

#[test]
fn an_open_group_lasts_for_one_command() {
    let dir = tempfile::tempdir().unwrap();
    let (path, group, inner) = grouped_project(dir.path());
    let rename = json!({"id": inner, "name": "Inner"}).to_string();
    // Opened in one command, the group is closed again in the next.
    assert!(benchup(&["call", &path, "enter_object", &json!({"id": group}).to_string()], None).status.success());
    let output = benchup(&["call", &path, "rename", &rename], None);
    assert_eq!(output.status.code(), Some(1));
    let hint = json_out(&output)["hint"].as_str().unwrap().to_owned();
    assert!(hint.contains(&format!("enter_object{{id:{group}}}")) && hint.contains("--batch"), "{hint}");
    // In one batch it works, and the file is left at the top level.
    let batch = format!(
        "{}\n{}\n",
        json!({"tool": "enter_object", "arguments": {"id": group}}),
        json!({"tool": "rename", "arguments": {"id": inner, "name": "Inner"}})
    );
    assert!(benchup(&["call", &path, "--batch", "-"], Some(&batch)).status.success());
    let doc = files::decode(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(doc.model.objects[&inner].name, "Inner");
    assert_eq!(doc.model.edit_context, None);
}

