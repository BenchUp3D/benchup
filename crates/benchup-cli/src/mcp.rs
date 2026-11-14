//! Newline-delimited JSON-RPC 2.0 for MCP over stdio.
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const SUPPORTED: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "Edits a BenchUp 3D woodworking project. Lengths are millimetres, Z is up. \
Call get_scene first to see the model. Each mutating call is one undo step in BenchUp. \
The session starts at the top level; a group opened with enter_object stays open until exit_to_root. \
Call get_guide once for conventions (front is -y), joint roles and woodworking recipes. \
Use run_batch for several calls as one undo step (with dry_run to try them first) and undo to go back.";

/// Where tool calls go. Returns the tool result and whether it succeeded.
pub trait Backend {
    fn call(&mut self, client: &str, name: &str, arguments: Value) -> (Value, bool);
    /// Several calls as one undo step, all or nothing; `dry_run` changes nothing (`run_batch`).
    fn batch(&mut self, calls: Vec<(String, Value)>, dry_run: bool) -> (Value, bool);
    /// Undo or redo the last change to the file (`undo`).
    fn history(&mut self, redo: bool) -> (Value, bool);
    /// A PNG of the model from a view, and what it shows (`render_view`).
    fn render(&mut self, view: &str, width: u32, height: u32) -> Result<(Vec<u8>, Value), String>;
}

/// `render_view` is served here rather than by the shared tools: its result
/// is an image, which only MCP clients can take.
fn render_view_tool() -> Value {
    json!({
        "name": "render_view",
        "description": "A PNG image of the model from a standard view (iso, front, back, left, right, top, bottom), with joints and router profiles cut, edges and dimensions with their values, to check a result without opening BenchUp. The text part reports dimensions whose labels overlap or that point along the view and cannot be read.",
        "inputSchema": {"type": "object", "properties": {
            "view": {"type": "string", "enum": benchup_tools::render::VIEWS},
            "width": {"type": "integer", "minimum": benchup_tools::render::MIN_SIZE, "maximum": benchup_tools::render::MAX_SIZE},
            "height": {"type": "integer", "minimum": benchup_tools::render::MIN_SIZE, "maximum": benchup_tools::render::MAX_SIZE}},
            "additionalProperties": false},
    })
}

/// Tools that only make sense for a file served over MCP: batches with dry
/// runs and `$N` references, and undo. The shared tools act one call at a time.
fn session_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "run_batch",
            "description": "Run several tool calls as ONE undo step, all or nothing, like `benchup call --batch`. In a call's arguments, \"$N.field\" is a value from call N's result (counting from 1), e.g. \"$1.created[0]\" or \"$3.joint_id\"; a reference to a list of ids inside a list is spliced in. With dry_run: true nothing is saved: every result is reported and the file stays as it was (end a dry run with check_model {\"only_new\": true}). When a call fails, nothing is changed and the answer names the failed call, its error and the results of the calls before it.",
            "inputSchema": {"type": "object", "properties": {
                "calls": {"type": "array", "minItems": 1, "items": {"type": "object", "properties": {
                    "tool": {"type": "string"}, "arguments": {"type": "object"}}, "required": ["tool"], "additionalProperties": false}},
                "dry_run": {"type": "boolean"}}, "required": ["calls"], "additionalProperties": false},
        }),
        json!({
            "name": "undo",
            "description": "Undo the last change to the project (one call or one run_batch), or redo it with redo: true. The same history as BenchUp's Edit menu.",
            "inputSchema": {"type": "object", "properties": {"redo": {"type": "boolean"}}, "additionalProperties": false},
        }),
    ]
}

fn session_call(backend: &mut dyn Backend, name: &str, arguments: &Value) -> (Value, bool) {
    if name == "undo" {
        return backend.history(arguments.get("redo").and_then(Value::as_bool).unwrap_or(false));
    }
    let Some(list) = arguments.get("calls").and_then(Value::as_array).filter(|l| !l.is_empty()) else {
        return (json!({"ok": false, "error": "calls must be a non-empty list of {\"tool\", \"arguments\"}"}), false);
    };
    let mut calls = Vec::with_capacity(list.len());
    for (n, call) in list.iter().enumerate() {
        let Some(tool) = call.get("tool").and_then(Value::as_str) else {
            return (json!({"ok": false, "error": format!("call {} has no \"tool\"", n + 1)}), false);
        };
        calls.push((tool.to_owned(), call.get("arguments").cloned().unwrap_or_else(|| json!({}))));
    }
    backend.batch(calls, arguments.get("dry_run").and_then(Value::as_bool).unwrap_or(false))
}

fn render_view(backend: &mut dyn Backend, arguments: &Value) -> Value {
    let view = arguments.get("view").and_then(Value::as_str).unwrap_or("iso");
    let size = |key: &str, default: u32| arguments.get(key).and_then(Value::as_u64).map_or(default, |v| v.min(u64::from(u32::MAX)) as u32);
    match backend.render(view, size("width", 1000), size("height", 750)) {
        Ok((png, info)) => json!({
            "content": [
                {"type": "image", "data": benchup_tools::render::base64(&png), "mimeType": "image/png"},
                {"type": "text", "text": info.to_string()},
            ],
            "isError": false,
        }),
        Err(error) => json!({"content": [{"type": "text", "text": json!({"ok": false, "error": error}).to_string()}], "isError": true}),
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Handle one parsed message; `None` means no reply (notification).
fn handle(message: &Value, backend: &mut dyn Backend, client: &mut String) -> Option<Value> {
    if !message.is_object() {
        return Some(error(Value::Null, -32600, "invalid request"));
    }
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str);
    let Some(method) = method else {
        // A response or malformed message: only malformed ones with an id get an error.
        return id
            .filter(|_| message.get("result").is_none() && message.get("error").is_none())
            .map(|id| error(id, -32600, "invalid request"));
    };
    let id = id?; // notifications never get replies
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let ok = |result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
    Some(match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = SUPPORTED.iter().find(|v| **v == requested).unwrap_or(&SUPPORTED[0]);
            if let Some(name) = params.pointer("/clientInfo/name").and_then(Value::as_str) {
                *client = name.to_owned();
            }
            ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "benchup", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => ok(json!({})),
        "tools/list" => {
            let mut tools = benchup_tools::mcp_tools();
            tools.push(render_view_tool());
            tools.extend(session_tools());
            ok(json!({"tools": tools}))
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(id, -32602, "tools/call needs a tool name"));
            };
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            if name == "render_view" {
                return Some(ok(render_view(backend, &arguments)));
            }
            let (result, success) = if matches!(name, "run_batch" | "undo") {
                session_call(backend, name, &arguments)
            } else {
                backend.call(client, name, arguments)
            };
            let mut out = json!({
                "content": [{"type": "text", "text": result.to_string()}],
                "isError": !success,
            });
            if result.is_object() {
                out["structuredContent"] = result;
            }
            ok(out)
        }
        _ => error(id, -32601, &format!("method not found: {method}")),
    })
}

/// Run the stdio loop until the reader ends.
pub fn serve<R: BufRead, W: Write>(reader: R, mut writer: W, backend: &mut dyn Backend) -> std::io::Result<()> {
    let mut client = String::from("agent");
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(_) => Some(error(Value::Null, -32700, "parse error")),
            Ok(Value::Array(_)) => Some(error(Value::Null, -32600, "batch requests are not supported")),
            Ok(message) => handle(&message, backend, &mut client),
        };
        if let Some(reply) = reply {
            writeln!(writer, "{reply}")?;
            writer.flush()?;
        }
    }
    Ok(())
}
