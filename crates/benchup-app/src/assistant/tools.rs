//! LLM-specific tool plumbing; the tools themselves live in `benchup_tools`.
use super::client::ToolCall;
use serde_json::{Value, json};

pub(crate) use benchup_tools::{ToolEnv, ToolOutcome, is_mutating, openai_tools as definitions};

/// Run one model-issued call: convert the OpenAI shape to the neutral call.
pub(crate) fn execute(model: &mut benchup_core::Model, call: &ToolCall, env: &mut ToolEnv) -> ToolOutcome {
    let call = benchup_tools::Call::from_text(call.id.as_str(), call.function.name.as_str(), &call.function.arguments);
    benchup_tools::execute(model, &call, env)
}

/// Some servers put a tool call in `content` as a fenced JSON block instead of
/// `tool_calls`. Accepted only when the whole reply is exactly one fenced block
/// or one bare JSON object, so JSON quoted inside prose is never executed.
pub(crate) fn fallback_tool_call(content: &str) -> Option<ToolCall> {
    let from_json = |text: &str| -> Option<ToolCall> {
        let value: Value = serde_json::from_str(text.trim()).ok()?;
        let call = value.get("function").filter(|f| f.is_object()).unwrap_or(&value);
        let name = call.get("name").or_else(|| call.get("tool"))?.as_str()?;
        if !benchup_tools::is_known(name) {
            return None;
        }
        let arguments = call.get("arguments").or_else(|| call.get("parameters")).cloned().unwrap_or_else(|| json!({}));
        let arguments = match arguments {
            Value::String(text) => serde_json::from_str(&text).ok()?,
            other => other,
        };
        Some(ToolCall::new("call_fallback", name, &arguments))
    };
    let content = content.trim();
    if let Some(inner) = content.strip_prefix("```").and_then(|rest| rest.strip_suffix("```")) {
        if inner.contains("```") {
            return None;
        }
        let body = inner.trim_start();
        let body = match body.get(..4) {
            Some(label) if label.eq_ignore_ascii_case("json") => &body[4..],
            _ => body,
        };
        return from_json(body);
    }
    (content.starts_with('{') && content.ends_with('}')).then(|| from_json(content)).flatten()
}


#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::Value;

    #[test]
    fn fenced_json_fallback_is_parsed() {
        let fenced = "```json\n{\"name\": \"create_board\", \"arguments\": {\"name\": \"A\", \"origin_mm\": [0,0,0], \"size_mm\": [1,1,1]}}\n```";
        let call = fallback_tool_call(fenced).unwrap();
        assert_eq!(call.function.name, "create_board");
        let arguments: Value = serde_json::from_str(&call.function.arguments).unwrap();
        assert_eq!(arguments["name"], "A");
        // Wrapped function form, string arguments, upper-case fence label.
        let wrapped = "```JSON\n{\"function\": {\"name\": \"get_scene\", \"arguments\": \"{}\"}}\n```";
        assert_eq!(fallback_tool_call(wrapped).unwrap().function.name, "get_scene");
        assert_eq!(fallback_tool_call("{\"tool\":\"exit_to_root\"}").unwrap().function.name, "exit_to_root");
        assert!(fallback_tool_call("```json\n{\"name\": \"launch_missiles\", \"arguments\": {}}\n```").is_none());
        assert!(fallback_tool_call("Just text, no tool.").is_none());
        assert!(fallback_tool_call("```json\nnot json\n```").is_none());
        assert!(fallback_tool_call("```rust\nfn main() {}\n```").is_none());
    }

    #[test]
    fn json_quoted_in_prose_is_never_executed() {
        let block = "```json\n{\"name\": \"get_scene\", \"arguments\": {}}\n```";
        for prose in [
            format!("I will add the board.\n{block}\nDone."),
            format!("To read the scene you could call:\n{block}"),
            format!("{block}\nand then\n{block}"),
            "Use {\"name\": \"get_scene\"} to read it.".to_owned(),
            "{\"name\": \"get_scene\"} is the call I would make.".to_owned(),
        ] {
            assert!(fallback_tool_call(&prose).is_none(), "{prose}");
        }
        // The whole reply being one block, with surrounding blanks, still runs.
        assert_eq!(fallback_tool_call(&format!("\n  {block}\n")).unwrap().function.name, "get_scene");
    }

    #[test]
    fn tool_definitions_sent_to_the_server_are_unchanged() {
        let expected: Value = serde_json::from_str(include_str!("tools_wire_fixture.json")).unwrap();
        assert_eq!(serde_json::to_string(&definitions()).unwrap(), serde_json::to_string(&expected).unwrap());
        assert_eq!(definitions().len(), 27);
    }
}

/// Test helpers shared by the assistant tests.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use benchup_core::Model;

    pub(crate) fn run(model: &mut Model, env: &mut ToolEnv, name: &str, args: Value) -> ToolOutcome {
        let call = benchup_tools::Call::new("t1", name, args);
        benchup_tools::execute(model, &call, env)
    }
    pub(crate) fn board(model: &mut Model, env: &mut ToolEnv, name: &str, origin: [f64; 3], size: [f64; 3]) -> u64 {
        let outcome = run(model, env, "create_board", json!({"name": name, "origin_mm": origin, "size_mm": size}));
        assert!(outcome.ok, "{}", outcome.result);
        outcome.created[0]
    }
}
