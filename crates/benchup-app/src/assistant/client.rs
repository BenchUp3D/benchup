//! OpenAI-compatible Chat Completions client. Requests run off the UI thread
//! (a thread natively, `fetch` on the web) and report through a channel, so the
//! egui loop never blocks. Tests substitute a scripted transport.
use benchup_core::tr;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::sync::mpsc::{self, Receiver};

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.endlesstokens.ai/v1";
pub(crate) const KEY_PORTAL_URL: &str = "https://portal.endlesstokens.ai/key";
#[cfg(any(not(target_arch = "wasm32"), test))]
const TIMEOUT_SECS: u64 = 120;
#[cfg(any(not(target_arch = "wasm32"), test))]
const RESPONSE_LIMIT: u64 = 4 * 1024 * 1024;

/// Server error text shown to the user is cut to this many characters.
const MAX_ERROR_CHARS: usize = 300;

/// Strip surrounding blanks and trailing slashes so one server has one key.
pub(crate) fn normalize_base(base: &str) -> String {
    base.trim().trim_end_matches('/').to_owned()
}

/// Credentials leave the device only over TLS, with loopback HTTP for local
/// servers. Mirrors the account server guard.
pub(crate) fn valid_base_url(base: &str) -> bool {
    let authority = base
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(""));
    !base.contains(['?', '#', '@', ' '])
        && (base.starts_with("https://") && authority.is_some_and(|a| !a.is_empty())
            || base.starts_with("http://")
                && authority.is_some_and(|a| {
                    a == "localhost"
                        || a == "127.0.0.1"
                        || a.starts_with("localhost:")
                        || a.starts_with("127.0.0.1:")
                }))
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ChatError {
    InvalidUrl,
    InvalidKey,
    ToolsUnsupported,
    Network(String),
    Http { status: u16, message: String },
    BadResponse(String),
}

impl ChatError {
    /// Text shown to the user.
    pub(crate) fn user_message(&self) -> String {
        match self {
            Self::InvalidUrl => tr!("assistant.error.invalid_server_address").into(),
            Self::InvalidKey => tr!("assistant.error.invalid_api_key").into(),
            Self::ToolsUnsupported => tr!("assistant.error.tools_unsupported").into(),
            Self::Network(e) => tr!("assistant.error.network", e = e),
            Self::Http { status, message } => {
                tr!("assistant.error.http", status = status, message = message)
            }
            Self::BadResponse(e) => tr!("assistant.error.bad_response", e = e),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Role {
    System,
    User,
    Assistant,
    Tool,
}

fn arguments_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    // Some servers send arguments as an object instead of a JSON string.
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => text,
        Value::Null => "{}".into(),
        other => other.to_string(),
    })
}
fn function_kind() -> String {
    "function".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct FunctionCall {
    pub name: String,
    #[serde(default, deserialize_with = "arguments_text")]
    pub arguments: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(default = "function_kind", rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}
impl ToolCall {
    pub(crate) fn new(id: &str, name: &str, arguments: &Value) -> Self {
        Self {
            id: id.into(),
            kind: function_kind(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Message {
    pub role: Role,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}
impl Message {
    pub(crate) fn text(role: Role, content: impl Into<String>) -> Self {
        Self { role, content: content.into(), tool_calls: Vec::new(), tool_call_id: None }
    }
    pub(crate) fn tool(call_id: &str, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<&'static str>,
    pub stream: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ChatResponse {
    #[serde(default)]
    pub choices: Vec<Choice>,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Choice {
    #[serde(default)]
    pub message: ResponseMessage,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ResponseMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
}

fn error_message(body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|value| {
        let error = value.get("error")?;
        error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .map(str::to_owned)
    });
    message.unwrap_or_else(|| body.to_owned()).chars().take(MAX_ERROR_CHARS).collect()
}

/// True when a 400 message says the model or server rejects tool calling,
/// as opposed to a bad argument that merely mentions a tool or function.
fn says_tools_unsupported(message: &str) -> bool {
    let lower = message.to_lowercase();
    const REFUSALS: [&str; 6] = ["not support", "unsupported", "does not allow", "doesn't allow", "not allowed", "no support"];
    const SUBJECTS: [&str; 3] = ["tool", "function calling", "function call"];
    const NEAR: usize = 40;
    let positions = |needle: &str| -> Vec<usize> { lower.match_indices(needle).map(|(i, _)| i).collect() };
    REFUSALS.iter().flat_map(|r| positions(r)).any(|refusal| {
        SUBJECTS.iter().flat_map(|s| positions(s)).any(|subject| subject.abs_diff(refusal) <= NEAR)
    })
}

/// Turn an HTTP status and body into JSON, or the matching typed error.
pub(crate) fn check_status(status: u16, body: &str) -> Result<Value, ChatError> {
    if (200..300).contains(&status) {
        return serde_json::from_str(body).map_err(|e| ChatError::BadResponse(e.to_string()));
    }
    let message = error_message(body);
    Err(match status {
        401 => ChatError::InvalidKey,
        400 if says_tools_unsupported(&message) => ChatError::ToolsUnsupported,
        _ => ChatError::Http { status, message },
    })
}

pub(crate) fn parse_models(status: u16, body: &str) -> Result<Vec<String>, ChatError> {
    let value = check_status(status, body)?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ChatError::BadResponse(tr!("assistant.error.no_model_list").into()))?;
    let mut ids: Vec<String> = data
        .iter()
        .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub(crate) fn parse_chat(status: u16, body: &str) -> Result<ChatResponse, ChatError> {
    let value = check_status(status, body)?;
    let response: ChatResponse =
        serde_json::from_value(value).map_err(|e| ChatError::BadResponse(e.to_string()))?;
    if response.choices.is_empty() {
        return Err(ChatError::BadResponse(tr!("assistant.error.no_choices").into()));
    }
    Ok(response)
}

/// Replies arrive on a channel the caller polls once per frame.
pub(crate) trait ChatTransport {
    fn models(&self, ctx: &eframe::egui::Context, base: &str, key: &str)
    -> Receiver<Result<Vec<String>, ChatError>>;
    fn chat(
        &self,
        ctx: &eframe::egui::Context,
        base: &str,
        key: &str,
        request: &ChatRequest,
    ) -> Receiver<Result<ChatResponse, ChatError>>;
}

pub(crate) struct HttpTransport;

impl ChatTransport for HttpTransport {
    fn models(&self, ctx: &eframe::egui::Context, base: &str, key: &str)
    -> Receiver<Result<Vec<String>, ChatError>> {
        spawn(ctx, format!("{}/models", normalize_base(base)), base, key, None, parse_models)
    }
    fn chat(
        &self,
        ctx: &eframe::egui::Context,
        base: &str,
        key: &str,
        request: &ChatRequest,
    ) -> Receiver<Result<ChatResponse, ChatError>> {
        let body = serde_json::to_string(request).unwrap_or_default();
        spawn(
            ctx,
            format!("{}/chat/completions", normalize_base(base)),
            base,
            key,
            Some(body),
            parse_chat,
        )
    }
}

fn spawn<T: Send + 'static>(
    ctx: &eframe::egui::Context,
    url: String,
    base: &str,
    key: &str,
    body: Option<String>,
    parse: fn(u16, &str) -> Result<T, ChatError>,
) -> Receiver<Result<T, ChatError>> {
    let (sender, receiver) = mpsc::channel();
    if !valid_base_url(&normalize_base(base)) {
        let _ = sender.send(Err(ChatError::InvalidUrl));
        return receiver;
    }
    let ctx = ctx.clone();
    let key = key.to_owned();
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        let result = (|| {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(TIMEOUT_SECS)))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .new_agent();
            let bearer = format!("Bearer {key}");
            let mut response = match &body {
                Some(body) => agent
                    .post(&url)
                    .header("Authorization", &bearer)
                    .header("Content-Type", "application/json")
                    .send(body.as_bytes()),
                None => agent.get(&url).header("Authorization", &bearer).call(),
            }
            .map_err(|e| ChatError::Network(e.to_string()))?;
            let status = response.status().as_u16();
            let bytes = response
                .body_mut()
                .with_config()
                .limit(RESPONSE_LIMIT)
                .read_to_vec()
                .map_err(|e| ChatError::Network(e.to_string()))?;
            parse(status, &String::from_utf8_lossy(&bytes))
        })();
        let _ = sender.send(result);
        ctx.request_repaint();
    });
    #[cfg(target_arch = "wasm32")]
    {
        use eframe::wasm_bindgen::JsValue;
        // The server answers CORS with a wildcard origin, which browsers only
        // accept for requests that send no cookies or other ambient credentials.
        let fetch = js_sys::Function::new_with_args(
            "args",
            r#"
            return (async () => {
                const a = JSON.parse(args);
                const headers = {'Authorization': 'Bearer ' + a.key};
                if (a.body !== null) headers['Content-Type'] = 'application/json';
                const r = await fetch(a.url, {method: a.body === null ? 'GET' : 'POST', body: a.body ?? undefined,
                    headers, credentials: 'omit', redirect: 'error', cache: 'no-store', signal: AbortSignal.timeout(120000)});
                const declared = r.headers.get('content-length');
                if (declared !== null && Number(declared) > 4194304) throw Error('Response too large');
                const text = await r.text();
                if (text.length > 4194304) throw Error('Response too large');
                return JSON.stringify({status: r.status, body: text});
            })();"#,
        );
        let args = serde_json::json!({"url": url, "key": key, "body": body}).to_string();
        let promise = fetch.call1(&JsValue::NULL, &JsValue::from_str(&args));
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let network = || ChatError::Network(tr!("assistant.error.cannot_reach_server").into());
                let value = promise.map_err(|_| network())?;
                let value = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&value))
                    .await
                    .map_err(|_| network())?;
                let reply: Value = serde_json::from_str(&value.as_string().ok_or_else(network)?)
                    .map_err(|e| ChatError::BadResponse(e.to_string()))?;
                parse(
                    reply["status"].as_u64().unwrap_or(0) as u16,
                    reply["body"].as_str().unwrap_or(""),
                )
            }
            .await;
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    receiver
}

/// Scripted transport: replies come from a queue; every request is recorded.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    #[derive(Clone, Default)]
    pub(crate) struct FakeTransport {
        pub replies: Arc<Mutex<VecDeque<Result<ChatResponse, ChatError>>>>,
        pub requests: Arc<Mutex<Vec<Value>>>,
        pub models: Arc<Mutex<Option<Result<Vec<String>, ChatError>>>>,
    }
    impl FakeTransport {
        pub(crate) fn push_tool_calls(&self, calls: Vec<ToolCall>) {
            self.replies.lock().unwrap().push_back(Ok(ChatResponse {
                choices: vec![Choice {
                    message: ResponseMessage { content: None, tool_calls: Some(calls) },
                }],
            }));
        }
        pub(crate) fn push_text(&self, text: &str) {
            self.replies.lock().unwrap().push_back(Ok(ChatResponse {
                choices: vec![Choice {
                    message: ResponseMessage { content: Some(text.into()), tool_calls: None },
                }],
            }));
        }
    }
    impl ChatTransport for FakeTransport {
        fn models(&self, _: &eframe::egui::Context, _: &str, _: &str)
        -> Receiver<Result<Vec<String>, ChatError>> {
            let (sender, receiver) = mpsc::channel();
            let _ = sender.send(self.models.lock().unwrap().clone().unwrap_or(Ok(Vec::new())));
            receiver
        }
        fn chat(
            &self,
            _: &eframe::egui::Context,
            _: &str,
            _: &str,
            request: &ChatRequest,
        ) -> Receiver<Result<ChatResponse, ChatError>> {
            self.requests.lock().unwrap().push(serde_json::to_value(request).unwrap());
            let (sender, receiver) = mpsc::channel();
            let reply = self.replies.lock().unwrap().pop_front().unwrap_or_else(|| {
                Err(ChatError::Network("no scripted reply".into()))
            });
            let _ = sender.send(reply);
            receiver
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_guard_requires_tls_except_loopback() {
        for ok in [
            DEFAULT_BASE_URL,
            "https://example.com",
            "http://localhost:8080/v1",
            "http://127.0.0.1/v1",
        ] {
            assert!(valid_base_url(ok), "{ok}");
        }
        for bad in [
            "http://example.com/v1",
            "http://localhost.evil.com/v1",
            "ftp://example.com",
            "https://",
            "https://user@example.com",
            "https://example.com/v1?x=1",
            "example.com",
            "",
        ] {
            assert!(!valid_base_url(bad), "{bad}");
        }
        assert_eq!(normalize_base(" https://a.b/v1/ "), "https://a.b/v1");
    }

    #[test]
    fn statuses_map_to_typed_errors() {
        let unauthorized = r#"{"error":{"message":"bad","code":"invalid_api_key"}}"#;
        assert_eq!(parse_models(401, unauthorized), Err(ChatError::InvalidKey));
        assert_eq!(
            parse_chat(400, r#"{"error":{"message":"This model does not support tools"}}"#)
                .unwrap_err(),
            ChatError::ToolsUnsupported
        );
        assert_eq!(
            parse_chat(500, r#"{"error":{"message":"boom"}}"#).unwrap_err(),
            ChatError::Http { status: 500, message: "boom".into() }
        );
        assert!(matches!(parse_chat(200, "not json"), Err(ChatError::BadResponse(_))));
        assert!(matches!(parse_chat(200, r#"{"choices":[]}"#), Err(ChatError::BadResponse(_))));
        assert!(!ChatError::InvalidKey.user_message().is_empty());
    }

    #[test]
    fn error_messages_are_always_truncated() {
        let long = "x".repeat(2000);
        let json = format!(r#"{{"error":{{"message":"{long}"}}}}"#);
        for body in [json.as_str(), long.as_str()] {
            let Err(ChatError::Http { message, .. }) = parse_chat(500, body) else { panic!() };
            assert_eq!(message.chars().count(), 300);
        }
    }

    #[test]
    fn bad_request_maps_to_tools_unsupported_only_when_it_says_so() {
        let body = |m: &str| serde_json::json!({"error": {"message": m}}).to_string();
        for message in [
            "This model does not support tools",
            "Tool calling is not supported by this model",
            "function calling is unsupported for qwen",
            "Tools are not allowed with this endpoint",
        ] {
            assert_eq!(parse_chat(400, &body(message)).unwrap_err(), ChatError::ToolsUnsupported, "{message}");
        }
        for message in [
            "Invalid value for function parameter 'name'",
            "messages.3: tool_call_id is required",
            "max_tokens is not supported with this model",
            "Unsupported parameter: temperature",
        ] {
            assert_eq!(
                parse_chat(400, &body(message)).unwrap_err(),
                ChatError::Http { status: 400, message: message.into() },
                "{message}"
            );
        }
    }

    #[test]
    fn parses_models_and_tool_calls_with_object_arguments() {
        let models = parse_models(200, r#"{"data":[{"id":"b"},{"id":"a"},{"id":"a"}]}"#).unwrap();
        assert_eq!(models, ["a", "b"]);
        let body = r#"{"choices":[{"message":{"content":null,"tool_calls":[
            {"id":"c1","type":"function","function":{"name":"get_scene","arguments":{"x":1}}},
            {"function":{"name":"select","arguments":"{\"ids\":[1]}"}}]}}]}"#;
        let response = parse_chat(200, body).unwrap();
        let calls = response.choices[0].message.tool_calls.clone().unwrap();
        assert_eq!(calls[0].function.arguments, r#"{"x":1}"#);
        assert_eq!(calls[1].function.arguments, r#"{"ids":[1]}"#);
        assert_eq!(calls[1].kind, "function");
    }

    #[test]
    fn messages_serialize_in_wire_format() {
        let assistant = Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: vec![ToolCall::new("c1", "get_scene", &serde_json::json!({}))],
            tool_call_id: None,
        };
        let value = serde_json::to_value([&assistant, &Message::tool("c1", "{}")]).unwrap();
        assert_eq!(value[0]["role"], "assistant");
        assert_eq!(value[0]["tool_calls"][0]["function"]["name"], "get_scene");
        assert_eq!(value[1]["role"], "tool");
        assert_eq!(value[1]["tool_call_id"], "c1");
        assert!(value[1].get("tool_calls").is_none());
    }
}
