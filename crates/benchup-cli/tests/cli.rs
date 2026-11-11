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
