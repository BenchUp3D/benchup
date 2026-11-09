//! Newline-delimited JSON-RPC 2.0 for MCP over stdio.
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const SUPPORTED: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "Edits a BenchUp 3D woodworking project. Lengths are millimetres, Z is up. \
Call get_scene first to see the model. Each mutating call is one undo step in BenchUp.";

/// Where tool calls go. Returns the tool result and whether it succeeded.
pub trait Backend {
    fn call(&mut self, client: &str, name: &str, arguments: Value) -> (Value, bool);
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
                "serverInfo": {"name": "benchup-mcp", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({"tools": benchup_tools::mcp_tools()})),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(id, -32602, "tools/call needs a tool name"));
            };
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let (result, success) = backend.call(client, name, arguments);
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
