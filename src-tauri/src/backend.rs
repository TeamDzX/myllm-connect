use std::{collections::BTreeMap, time::Duration};

use reqwest::{Client, RequestBuilder, Url};
use serde::Serialize;
use serde_json::{json, Map, Value};

const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_LLAMA_CPP_URLS: &[&str] = &["http://127.0.0.1:8080", "http://127.0.0.1:8081"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Ollama,
    LlamaCpp,
}

#[derive(Clone, Debug)]
pub struct Backend {
    pub kind: BackendKind,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
}

impl Backend {
    pub fn request(&self, builder: RequestBuilder) -> RequestBuilder {
        match &self.api_key {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        }
    }
}

pub fn client() -> Client {
    Client::builder()
        // Bound failed connections without imposing a deadline on long-running
        // generation streams.
        .connect_timeout(Duration::from_secs(3))
        .build()
        .expect("valid HTTP client")
}

pub async fn detect(client: &Client) -> Option<Backend> {
    let configured_llama = std::env::var("MYLLM_LLAMA_CPP_URL").ok();
    let mut configured_url = None;

    if let Some(raw_url) = configured_llama {
        if let Ok(url) = normalize_loopback_url(&raw_url) {
            configured_url = Some(url.clone());
            if let Some(backend) = detect_llama_cpp(client, url).await {
                return Some(backend);
            }
        }
    }

    if let Some(backend) = detect_ollama(client).await {
        return Some(backend);
    }

    for url in DEFAULT_LLAMA_CPP_URLS {
        if configured_url.as_deref() == Some(*url) {
            continue;
        }
        if let Some(backend) = detect_llama_cpp(client, (*url).to_string()).await {
            return Some(backend);
        }
    }

    None
}

async fn detect_ollama(client: &Client) -> Option<Backend> {
    let response = client
        .get(format!("{DEFAULT_OLLAMA_URL}/api/tags"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: Value = response.json().await.ok()?;
    let model = body
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| models.first())
        .and_then(|model| model.get("name").or_else(|| model.get("model")))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    Some(Backend {
        kind: BackendKind::Ollama,
        base_url: DEFAULT_OLLAMA_URL.to_string(),
        api_key: None,
        model,
    })
}

async fn detect_llama_cpp(client: &Client, base_url: String) -> Option<Backend> {
    let api_key = std::env::var("MYLLM_LLAMA_CPP_API_KEY").ok();
    let request = client
        .get(format!("{base_url}/v1/models"))
        .timeout(Duration::from_secs(5));
    let request = match &api_key {
        Some(key) => request.bearer_auth(key),
        None => request,
    };
    let response = request.send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: Value = response.json().await.ok()?;
    let model = body
        .get("data")
        .and_then(Value::as_array)
        .and_then(|models| models.first())
        .and_then(|model| model.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    Some(Backend {
        kind: BackendKind::LlamaCpp,
        base_url,
        api_key,
        model,
    })
}

fn normalize_loopback_url(raw: &str) -> Result<String, String> {
    let mut url = Url::parse(raw).map_err(|_| "invalid llama.cpp URL".to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("llama.cpp URL must use http or https".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "llama.cpp URL is missing a host".to_string())?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
    if !is_loopback {
        return Err("llama.cpp URL must point to this computer".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("credentials are not allowed in the llama.cpp URL".to_string());
    }
    url.set_query(None);
    url.set_fragment(None);
    let normalized = url.as_str().trim_end_matches('/').to_string();
    Ok(normalized)
}

pub fn llama_models_to_ollama(value: Value, context_window: Option<u64>) -> Value {
    let models = value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .map(|id| {
            let mut model = json!({
                "name": id,
                "model": id,
                "modified_at": "1970-01-01T00:00:00Z",
                "size": 0,
                "digest": "",
                "details": {
                    "format": "gguf",
                    "family": "llama.cpp",
                    "families": ["llama.cpp"],
                    "parameter_size": "unknown",
                    "quantization_level": "unknown"
                }
            });
            if let Some(context_window) = context_window {
                model["context_window"] = json!(context_window);
            }
            model
        })
        .collect::<Vec<_>>();
    json!({ "models": models })
}

pub fn llama_context_window(value: &Value) -> Option<u64> {
    value
        .get("default_generation_settings")
        .and_then(|settings| settings.get("n_ctx"))
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
}

pub fn ollama_chat_to_openai(mut value: Value) -> Result<(Value, bool), String> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| "chat request must be a JSON object".to_string())?;
    let stream = object
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let messages = object
        .remove("messages")
        .unwrap_or_else(|| Value::Array(Vec::new()));
    object.insert("messages".to_string(), convert_messages(messages)?);

    if let Some(format) = object.remove("format") {
        let response_format = match format {
            Value::String(value) if value == "json" => Some(json!({ "type": "json_object" })),
            Value::Object(schema) => Some(json!({ "type": "json_object", "schema": schema })),
            _ => None,
        };
        if let Some(response_format) = response_format {
            object.insert("response_format".to_string(), response_format);
        }
    }

    if let Some(Value::Object(options)) = object.remove("options") {
        const PASSTHROUGH: &[&str] = &[
            "temperature",
            "top_p",
            "top_k",
            "seed",
            "stop",
            "frequency_penalty",
            "presence_penalty",
        ];
        for key in PASSTHROUGH {
            if let Some(value) = options.get(*key) {
                object.insert((*key).to_string(), value.clone());
            }
        }
        if let Some(value) = options.get("repeat_penalty") {
            object.insert("repeat_penalty".to_string(), value.clone());
        }
        if let Some(value) = options.get("num_predict") {
            object.insert("max_tokens".to_string(), value.clone());
        }
    }

    object.remove("keep_alive");
    object.remove("think");
    object.insert("stream".to_string(), Value::Bool(stream));
    if stream {
        object.insert(
            "stream_options".to_string(),
            json!({ "include_usage": true }),
        );
    }

    Ok((value, stream))
}

fn convert_messages(messages: Value) -> Result<Value, String> {
    let messages = messages
        .as_array()
        .ok_or_else(|| "messages must be an array".to_string())?;
    let mut converted = Vec::with_capacity(messages.len());
    let mut known_calls = Vec::<(String, String)>::new();

    for (message_index, source) in messages.iter().enumerate() {
        let mut message = source
            .as_object()
            .cloned()
            .ok_or_else(|| format!("message {message_index} must be an object"))?;

        if let Some(images) = message
            .remove("images")
            .and_then(|value| value.as_array().cloned())
        {
            let mut content = Vec::new();
            if let Some(text) = message.remove("content") {
                if !text.as_str().unwrap_or_default().is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
            }
            for image in images {
                if let Some(data) = image.as_str() {
                    let url = if data.starts_with("data:") {
                        data.to_string()
                    } else {
                        format!("data:image/jpeg;base64,{data}")
                    };
                    content.push(json!({ "type": "image_url", "image_url": { "url": url } }));
                }
            }
            message.insert("content".to_string(), Value::Array(content));
        }

        if let Some(Value::Array(tool_calls)) = message.get_mut("tool_calls") {
            for (call_index, call) in tool_calls.iter_mut().enumerate() {
                let call_object = call
                    .as_object_mut()
                    .ok_or_else(|| "tool call must be an object".to_string())?;
                let id = call_object
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| format!("call_{message_index}_{call_index}"));
                let function = call_object
                    .get_mut("function")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| "tool call is missing a function".to_string())?;
                let name = function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if let Some(arguments) = function.get_mut("arguments") {
                    if !arguments.is_string() {
                        *arguments = Value::String(
                            serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string()),
                        );
                    }
                }
                call_object.insert("id".to_string(), Value::String(id.clone()));
                call_object.insert("type".to_string(), Value::String("function".to_string()));
                known_calls.push((name, id));
            }
        }

        if message.get("role").and_then(Value::as_str) == Some("tool")
            && !message.contains_key("tool_call_id")
        {
            let named_call = message
                .get("tool_name")
                .or_else(|| message.get("name"))
                .and_then(Value::as_str)
                .and_then(|name| {
                    known_calls
                        .iter()
                        .rev()
                        .find(|(known_name, _)| known_name == name)
                        .map(|(_, id)| id.clone())
                });
            let call_id = named_call.or_else(|| known_calls.last().map(|(_, id)| id.clone()));
            if let Some(call_id) = call_id {
                message.insert("tool_call_id".to_string(), Value::String(call_id));
            }
            message.remove("tool_name");
        }

        converted.push(Value::Object(message));
    }

    Ok(Value::Array(converted))
}

pub fn openai_chat_to_ollama(value: Value, fallback_model: &str) -> Result<Value, String> {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| "llama.cpp returned no chat choice".to_string())?;
    let message = choice
        .get("message")
        .cloned()
        .unwrap_or_else(|| json!({ "role": "assistant", "content": "" }));
    let message = openai_message_to_ollama(message);
    let usage = value.get("usage").cloned().unwrap_or_else(|| json!({}));

    Ok(json!({
        "model": value.get("model").and_then(Value::as_str).unwrap_or(fallback_model),
        "created_at": "1970-01-01T00:00:00Z",
        "message": message,
        "done": true,
        "done_reason": choice.get("finish_reason").cloned().unwrap_or(Value::Null),
        "prompt_eval_count": usage.get("prompt_tokens").cloned().unwrap_or(Value::Null),
        "eval_count": usage.get("completion_tokens").cloned().unwrap_or(Value::Null)
    }))
}

fn openai_message_to_ollama(message: Value) -> Value {
    let mut output = Map::new();
    output.insert("role".to_string(), json!("assistant"));
    output.insert("content".to_string(), json!(""));
    if let Some(source) = message.as_object() {
        if let Some(role) = source.get("role") {
            output.insert("role".to_string(), role.clone());
        }
        if let Some(content) = source.get("content") {
            output.insert("content".to_string(), content.clone());
        }
        if let Some(thinking) = source.get("reasoning_content") {
            output.insert("thinking".to_string(), thinking.clone());
        }
        if let Some(tool_calls) = source.get("tool_calls").and_then(Value::as_array) {
            output.insert(
                "tool_calls".to_string(),
                Value::Array(tool_calls.iter().map(openai_tool_call_to_ollama).collect()),
            );
        }
    }
    Value::Object(output)
}

fn openai_tool_call_to_ollama(call: &Value) -> Value {
    let function = call.get("function").cloned().unwrap_or_else(|| json!({}));
    let name = function.get("name").cloned().unwrap_or_else(|| json!(""));
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|value| serde_json::from_str(value).ok())
        .or_else(|| function.get("arguments").cloned())
        .unwrap_or_else(|| json!({}));
    json!({ "function": { "name": name, "arguments": arguments } })
}

#[derive(Default)]
struct ToolCallBuffer {
    name: String,
    arguments: String,
}

pub struct LlamaStreamTranslator {
    buffer: Vec<u8>,
    model: String,
    finish_reason: Option<String>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    tool_calls: BTreeMap<usize, ToolCallBuffer>,
    finished: bool,
}

impl LlamaStreamTranslator {
    pub fn new(model: String) -> Self {
        Self {
            buffer: Vec::new(),
            model,
            finish_reason: None,
            prompt_tokens: None,
            completion_tokens: None,
            tool_calls: BTreeMap::new(),
            finished: false,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        let mut output = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=end).collect::<Vec<_>>();
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            if let Ok(line) = std::str::from_utf8(&line) {
                output.extend(self.process_line(line));
            }
        }
        output
    }

    pub fn finish(&mut self) -> Vec<String> {
        let mut output = Vec::new();
        if !self.buffer.is_empty() {
            let line = String::from_utf8_lossy(&self.buffer).to_string();
            self.buffer.clear();
            output.extend(self.process_line(line.trim_end_matches(['\r', '\n'])));
        }
        if !self.finished {
            output.extend(self.final_chunks());
        }
        output
    }

    fn process_line(&mut self, line: &str) -> Vec<String> {
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            return Vec::new();
        };
        if data == "[DONE]" {
            return self.final_chunks();
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        if let Some(error) = value.get("error") {
            return vec![format!("{}\n", json!({ "error": error }))];
        }
        if let Some(model) = value.get("model").and_then(Value::as_str) {
            self.model = model.to_string();
        }
        if let Some(usage) = value.get("usage") {
            self.prompt_tokens = usage.get("prompt_tokens").and_then(Value::as_u64);
            self.completion_tokens = usage.get("completion_tokens").and_then(Value::as_u64);
        }
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return Vec::new();
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta") else {
            return Vec::new();
        };

        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let target = self.tool_calls.entry(index).or_default();
                if let Some(name) = call
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                {
                    target.name.push_str(name);
                }
                if let Some(arguments) = call
                    .get("function")
                    .and_then(|function| function.get("arguments"))
                    .and_then(Value::as_str)
                {
                    target.arguments.push_str(arguments);
                }
            }
        }

        let content = delta.get("content").and_then(Value::as_str);
        let thinking = delta.get("reasoning_content").and_then(Value::as_str);
        if content.is_none() && thinking.is_none() {
            return Vec::new();
        }
        let mut message = json!({
            "role": delta.get("role").and_then(Value::as_str).unwrap_or("assistant"),
            "content": content.unwrap_or("")
        });
        if let Some(thinking) = thinking {
            message["thinking"] = json!(thinking);
        }
        vec![format!(
            "{}\n",
            json!({
                "model": self.model,
                "created_at": "1970-01-01T00:00:00Z",
                "message": message,
                "done": false
            })
        )]
    }

    fn final_chunks(&mut self) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let mut output = Vec::new();
        if !self.tool_calls.is_empty() {
            let tool_calls = self
                .tool_calls
                .values()
                .map(|call| {
                    let arguments = serde_json::from_str::<Value>(&call.arguments)
                        .unwrap_or_else(|_| Value::String(call.arguments.clone()));
                    json!({ "function": { "name": call.name, "arguments": arguments } })
                })
                .collect::<Vec<_>>();
            output.push(format!(
                "{}\n",
                json!({
                    "model": self.model,
                    "created_at": "1970-01-01T00:00:00Z",
                    "message": { "role": "assistant", "content": "", "tool_calls": tool_calls },
                    "done": false
                })
            ));
        }
        output.push(format!(
            "{}\n",
            json!({
                "model": self.model,
                "created_at": "1970-01-01T00:00:00Z",
                "message": { "role": "assistant", "content": "" },
                "done": true,
                "done_reason": self.finish_reason.clone().unwrap_or_else(|| "stop".to_string()),
                "prompt_eval_count": self.prompt_tokens,
                "eval_count": self.completion_tokens
            })
        ));
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepts_loopback_backend_urls() {
        assert!(normalize_loopback_url("http://localhost:8080/").is_ok());
        assert!(normalize_loopback_url("http://127.0.0.1:8080").is_ok());
        assert!(normalize_loopback_url("http://[::1]:8080").is_ok());
        assert!(normalize_loopback_url("http://192.168.1.20:8080").is_err());
        assert!(normalize_loopback_url("https://example.com").is_err());
    }

    #[test]
    fn converts_ollama_tools_images_and_options() {
        let request = json!({
            "model": "local-model",
            "messages": [
                { "role": "user", "content": "look", "images": ["abc123"] },
                { "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": "weather", "arguments": { "city": "Seattle" } } }] },
                { "role": "tool", "tool_name": "weather", "content": "Rain" }
            ],
            "stream": true,
            "options": { "temperature": 0.2, "num_predict": 42 },
            "format": "json"
        });
        let (converted, stream) = ollama_chat_to_openai(request).unwrap();
        assert!(stream);
        assert_eq!(converted["temperature"], json!(0.2));
        assert_eq!(converted["max_tokens"], json!(42));
        assert_eq!(converted["response_format"]["type"], "json_object");
        assert_eq!(
            converted["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/jpeg;base64,abc123"
        );
        assert_eq!(
            converted["messages"][1]["tool_calls"][0]["function"]["arguments"],
            "{\"city\":\"Seattle\"}"
        );
        assert_eq!(converted["messages"][2]["tool_call_id"], "call_1_0");
    }

    #[test]
    fn translates_fragmented_sse_and_tool_calls() {
        let mut translator = LlamaStreamTranslator::new("fallback".to_string());
        let first = b"data: {\"model\":\"model-a\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Hel";
        assert!(translator.push(first).is_empty());
        let second = b"lo\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"weather\",\"arguments\":\"{\\\"city\\\":\"}}]},\"finish_reason\":null}]}\n";
        let output = translator.push(second);
        assert_eq!(output.len(), 1);
        assert!(output[0].contains("Hello"));
        let third = b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"Seattle\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":3}}\n\ndata: [DONE]\n\n";
        let output = translator.push(third);
        assert_eq!(output.len(), 2);
        assert!(output[0].contains("weather"));
        assert!(output[0].contains("Seattle"));
        assert!(output[1].contains("\"done\":true"));
        assert!(output[1].contains("\"prompt_eval_count\":9"));
    }

    #[test]
    fn translates_non_streaming_response() {
        let response = json!({
            "model": "model-a",
            "choices": [{
                "message": { "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": "weather", "arguments": "{\"city\":\"Seattle\"}" } }] },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 4, "completion_tokens": 2 }
        });
        let converted = openai_chat_to_ollama(response, "fallback").unwrap();
        assert_eq!(converted["done"], true);
        assert_eq!(
            converted["message"]["tool_calls"][0]["function"]["arguments"]["city"],
            "Seattle"
        );
        assert_eq!(converted["prompt_eval_count"], 4);
    }

    #[test]
    fn adds_llama_context_window_to_model_discovery() {
        let models = json!({ "data": [{ "id": "local-model" }] });
        let converted = llama_models_to_ollama(models, Some(262_144));

        assert_eq!(converted["models"][0]["context_window"], 262_144);
    }

    #[test]
    fn reads_context_window_from_llama_props() {
        let props = json!({ "default_generation_settings": { "n_ctx": 262144 } });

        assert_eq!(llama_context_window(&props), Some(262_144));
    }
}
