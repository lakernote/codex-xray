use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_stream::stream;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::provider::read_credential;

pub const CHAT_BRIDGE_PORT: u16 = 32_198;
const CHAT_BRIDGE_VERSION: u32 = 1;
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
static EVENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatBridgeProvider {
    pub upstream_base_url: String,
    pub credential_mode: String,
    pub env_key: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_context_window")]
    pub context_window: u64,
}

fn default_context_window() -> u64 {
    128_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatBridgeFile {
    version: u32,
    providers: BTreeMap<String, ChatBridgeProvider>,
}

impl Default for ChatBridgeFile {
    fn default() -> Self {
        Self {
            version: CHAT_BRIDGE_VERSION,
            providers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatBridgeStatus {
    pub running: bool,
    pub base_url: String,
    pub configured_providers: usize,
    pub last_error: Option<String>,
}

#[derive(Debug, Default)]
struct RuntimeStatus {
    running: bool,
    last_error: Option<String>,
}

#[derive(Clone)]
pub struct ChatBridgeState {
    config_path: PathBuf,
    providers: Arc<RwLock<BTreeMap<String, ChatBridgeProvider>>>,
    runtime: Arc<RwLock<RuntimeStatus>>,
    client: reqwest::Client,
}

pub struct ChatBridgeRollback {
    provider_id: String,
    previous: Option<ChatBridgeProvider>,
    changed: bool,
}

impl ChatBridgeState {
    pub fn load(config_path: PathBuf) -> Result<Self, String> {
        let providers = if config_path.is_file() {
            let content = fs::read(&config_path)
                .map_err(|error| format!("无法读取 Chat 兼容桥配置：{error}"))?;
            let parsed: ChatBridgeFile = serde_json::from_slice(&content)
                .map_err(|error| format!("Chat 兼容桥配置无法解析：{error}"))?;
            if parsed.version != CHAT_BRIDGE_VERSION {
                return Err(format!("Chat 兼容桥配置版本 {} 不受支持。", parsed.version));
            }
            parsed.providers
        } else {
            BTreeMap::new()
        };
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(10 * 60))
            .build()
            .map_err(|error| format!("无法初始化 Chat 兼容桥 HTTP 客户端：{error}"))?;
        Ok(Self {
            config_path,
            providers: Arc::new(RwLock::new(providers)),
            runtime: Arc::new(RwLock::new(RuntimeStatus::default())),
            client,
        })
    }

    pub fn status(&self) -> ChatBridgeStatus {
        let runtime = self.runtime.read().expect("chat bridge runtime lock");
        let configured_providers = self
            .providers
            .read()
            .expect("chat bridge providers lock")
            .len();
        ChatBridgeStatus {
            running: runtime.running,
            base_url: bridge_root_url(),
            configured_providers,
            last_error: runtime.last_error.clone(),
        }
    }

    pub fn provider(&self, provider_id: &str) -> Option<ChatBridgeProvider> {
        self.providers
            .read()
            .ok()
            .and_then(|providers| providers.get(provider_id).cloned())
    }

    pub fn persist_provider(
        &self,
        provider_id: &str,
        provider: ChatBridgeProvider,
    ) -> Result<ChatBridgeRollback, String> {
        let mut providers = self
            .providers
            .write()
            .map_err(|_| "Chat 兼容桥 Provider 状态已损坏".to_string())?;
        let previous = providers.insert(provider_id.to_string(), provider.clone());
        if previous.as_ref() == Some(&provider) {
            return Ok(ChatBridgeRollback {
                provider_id: provider_id.to_string(),
                previous,
                changed: false,
            });
        }
        if let Err(error) = save_file(&self.config_path, &providers) {
            if let Some(previous) = previous.clone() {
                providers.insert(provider_id.to_string(), previous);
            } else {
                providers.remove(provider_id);
            }
            return Err(error);
        }
        Ok(ChatBridgeRollback {
            provider_id: provider_id.to_string(),
            previous,
            changed: true,
        })
    }

    pub fn rollback_provider(&self, rollback: ChatBridgeRollback) {
        if !rollback.changed {
            return;
        }
        if let Ok(mut providers) = self.providers.write() {
            if let Some(previous) = rollback.previous {
                providers.insert(rollback.provider_id, previous);
            } else {
                providers.remove(&rollback.provider_id);
            }
            let _ = save_file(&self.config_path, &providers);
        }
    }
}

fn save_file(path: &Path, providers: &BTreeMap<String, ChatBridgeProvider>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 Chat 兼容桥配置目录：{error}"))?;
    }
    let content = serde_json::to_vec_pretty(&ChatBridgeFile {
        version: CHAT_BRIDGE_VERSION,
        providers: providers.clone(),
    })
    .map_err(|error| format!("无法序列化 Chat 兼容桥配置：{error}"))?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| format!("无法保存 Chat 兼容桥配置：{error}"))
}

pub fn bridge_root_url() -> String {
    format!("http://127.0.0.1:{CHAT_BRIDGE_PORT}")
}

pub fn provider_base_url(provider_id: &str) -> String {
    format!("{}/v1/chat/{provider_id}", bridge_root_url())
}

pub fn is_provider_bridge_url(provider_id: &str, base_url: &str) -> bool {
    base_url.trim_end_matches('/') == provider_base_url(provider_id)
}

pub async fn serve(state: ChatBridgeState) {
    let router = bridge_router(state.clone());
    let address = format!("127.0.0.1:{CHAT_BRIDGE_PORT}");
    match tokio::net::TcpListener::bind(&address).await {
        Ok(listener) => {
            if let Ok(mut runtime) = state.runtime.write() {
                runtime.running = true;
                runtime.last_error = None;
            }
            if let Err(error) = axum::serve(listener, router).await
                && let Ok(mut runtime) = state.runtime.write()
            {
                runtime.running = false;
                runtime.last_error = Some(format!("Chat 兼容桥已停止：{error}"));
            }
        }
        Err(error) => {
            if let Ok(mut runtime) = state.runtime.write() {
                runtime.running = false;
                runtime.last_error = Some(format!("无法监听 {address}：{error}"));
            }
        }
    }
}

fn bridge_router(state: ChatBridgeState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/chat/{provider_id}/models", get(models))
        .route("/v1/chat/{provider_id}/responses", post(responses))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(state)
}

async fn health(State(state): State<ChatBridgeState>) -> Json<ChatBridgeStatus> {
    Json(state.status())
}

async fn models(
    State(state): State<ChatBridgeState>,
    AxumPath(provider_id): AxumPath<String>,
) -> Response {
    let Some(provider) = state.provider(&provider_id) else {
        return json_error(
            StatusCode::NOT_FOUND,
            format!("Chat 兼容桥中没有 Provider {provider_id}。"),
        );
    };
    if provider.model.trim().is_empty() {
        return json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Chat Provider {provider_id} 没有配置模型 ID。"),
        );
    }
    Json(json!({
        "models": [bridge_model_info(&provider)]
    }))
    .into_response()
}

fn bridge_model_info(provider: &ChatBridgeProvider) -> Value {
    let context_window = provider.context_window.max(8_192);
    json!({
        "slug": provider.model,
        "display_name": provider.model,
        "description": "Chat Completions via Codex X-Ray",
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": [
            {"effort": "low", "description": "Faster responses with lighter reasoning"},
            {"effort": "medium", "description": "Balanced reasoning"},
            {"effort": "high", "description": "More reasoning for complex work"}
        ],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1,
        "additional_speed_tiers": [],
        "service_tiers": [],
        "default_service_tier": null,
        "availability_nux": null,
        "upgrade": null,
        "base_instructions": "You are Codex, an agentic coding assistant. Work carefully, use the provided tools when needed, and give concise, accurate answers.",
        "model_messages": null,
        "include_skills_usage_instructions": true,
        "default_reasoning_summary": "none",
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "web_search_tool_type": "text",
        "truncation_policy": {"mode": "tokens", "limit": 10_000},
        "supports_parallel_tool_calls": true,
        "supports_image_detail_original": false,
        "context_window": context_window,
        "max_context_window": context_window,
        "comp_hash": null,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text"],
        "supports_search_tool": false,
        "use_responses_lite": false,
        "tool_mode": "direct",
        "multi_agent_version": null
    })
}

async fn responses(
    State(state): State<ChatBridgeState>,
    AxumPath(provider_id): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let Some(provider) = state.provider(&provider_id) else {
        return json_error(
            StatusCode::NOT_FOUND,
            format!("Chat 兼容桥中没有 Provider {provider_id}。"),
        );
    };
    let chat_request = match responses_to_chat(&request) {
        Ok(request) => request,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let endpoint = chat_endpoint(&provider.upstream_base_url);
    let mut upstream = state
        .client
        .post(&endpoint)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "text/event-stream")
        .json(&chat_request);
    match bridge_credential(&provider_id, &provider, &headers).await {
        Ok(Some(credential)) => upstream = upstream.bearer_auth(credential),
        Ok(None) => {}
        Err(error) => return json_error(StatusCode::UNAUTHORIZED, error),
    }
    let upstream = match upstream.send().await {
        Ok(response) => response,
        Err(error) => {
            return json_error(
                StatusCode::BAD_GATEWAY,
                format!("Chat 上游连接失败：{error}"),
            );
        }
    };
    let status = upstream.status();
    if !status.is_success() {
        let body = upstream
            .text()
            .await
            .unwrap_or_else(|error| format!("无法读取上游错误：{error}"));
        return (
            status,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            body,
        )
            .into_response();
    }

    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("chat-model")
        .to_string();
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.contains("text/event-stream") {
        return match upstream.json::<Value>().await {
            Ok(response) => {
                let mut stream = ResponseStream::new(model);
                let mut frames = stream.start();
                frames.extend(stream.consume_chat_response(&response));
                frames.extend(stream.finish());
                sse_response(Body::from(frames.concat()))
            }
            Err(error) => json_error(
                StatusCode::BAD_GATEWAY,
                format!("Chat 上游没有返回有效 JSON：{error}"),
            ),
        };
    }

    let output = stream! {
        let mut events = upstream.bytes_stream().eventsource();
        let mut response_stream = ResponseStream::new(model);
        for frame in response_stream.start() {
            yield Ok::<Bytes, Infallible>(Bytes::from(frame));
        }
        let mut finished = false;
        while let Some(event) = events.next().await {
            match event {
                Ok(event) if event.data.trim() == "[DONE]" => {
                    for frame in response_stream.finish() {
                        yield Ok(Bytes::from(frame));
                    }
                    finished = true;
                    break;
                }
                Ok(event) => match serde_json::from_str::<Value>(&event.data) {
                    Ok(chunk) => {
                        for frame in response_stream.consume_chat_chunk(&chunk) {
                            yield Ok(Bytes::from(frame));
                        }
                    }
                    Err(error) => {
                        for frame in response_stream.fail(format!("Chat 流事件无法解析：{error}")) {
                            yield Ok(Bytes::from(frame));
                        }
                        finished = true;
                        break;
                    }
                },
                Err(error) => {
                    for frame in response_stream.fail(format!("Chat 流已中断：{error}")) {
                        yield Ok(Bytes::from(frame));
                    }
                    finished = true;
                    break;
                }
            }
        }
        if !finished {
            for frame in response_stream.finish() {
                yield Ok(Bytes::from(frame));
            }
        }
    };
    sse_response(Body::from_stream(output))
}

fn sse_response(body: Body) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/event-stream; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
            (header::CONNECTION, "keep-alive"),
        ],
        body,
    )
        .into_response()
}

fn json_error(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "message": message,
                "type": "codex_xray_chat_bridge_error"
            }
        })),
    )
        .into_response()
}

async fn bridge_credential(
    provider_id: &str,
    provider: &ChatBridgeProvider,
    headers: &HeaderMap,
) -> Result<Option<String>, String> {
    let inbound = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match provider.credential_mode.as_str() {
        "keychain" => {
            let provider_id = provider_id.to_string();
            let credential = tokio::task::spawn_blocking(move || read_credential(&provider_id))
                .await
                .map_err(|error| format!("读取系统凭据任务失败：{error}"))??;
            if inbound != Some(credential.as_str()) {
                return Err("本地兼容桥认证失败，请重新保存这个 Provider 的 API Key。".to_string());
            }
            Ok(Some(credential))
        }
        "environment" => {
            if let Some(env_key) = provider.env_key.as_deref()
                && let Ok(value) = std::env::var(env_key)
                && !value.trim().is_empty()
            {
                if inbound != Some(value.as_str()) {
                    return Err(format!("本地兼容桥没有收到与 {env_key} 匹配的认证信息。"));
                }
                return Ok(Some(value));
            }
            Ok(inbound.map(ToOwned::to_owned))
        }
        "none" => Ok(None),
        other => Err(format!("Chat 兼容桥不支持凭据方式 {other}。")),
    }
}

fn chat_endpoint(base_url: &str) -> String {
    let base_url = base_url.trim_end_matches('/');
    if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{base_url}/chat/completions")
    }
}

pub fn responses_to_chat(request: &Value) -> Result<Value, String> {
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Responses 请求缺少 model。".to_string())?;
    let mut messages = Vec::new();
    if let Some(instructions) = request
        .get("instructions")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        messages.push(json!({"role": "system", "content": instructions}));
    }
    match request.get("input") {
        Some(Value::String(content)) => {
            messages.push(json!({"role": "user", "content": content}));
        }
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(message) = response_input_to_chat_message(item)? {
                    push_chat_message(&mut messages, message);
                }
            }
        }
        Some(_) => return Err("Responses input 必须是字符串或数组。".to_string()),
        None => {}
    }
    let tools = request
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(response_tool_to_chat)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if tools.len() > 128 {
        return Err(format!(
            "当前任务提供了 {} 个函数工具，超过 Chat 兼容桥的 128 个上限。请在 Codex 中停用不需要的 MCP、插件或 Skill 后重试。",
            tools.len()
        ));
    }
    let mut output = Map::from_iter([
        ("model".to_string(), json!(model)),
        ("messages".to_string(), Value::Array(messages)),
        ("stream".to_string(), json!(true)),
        ("stream_options".to_string(), json!({"include_usage": true})),
    ]);
    if !tools.is_empty() {
        output.insert("tools".to_string(), Value::Array(tools));
        if let Some(tool_choice) = request.get("tool_choice") {
            output.insert(
                "tool_choice".to_string(),
                response_tool_choice_to_chat(tool_choice),
            );
        }
        if let Some(parallel) = request.get("parallel_tool_calls") {
            output.insert("parallel_tool_calls".to_string(), parallel.clone());
        }
    }
    for (source, target) in [
        ("max_output_tokens", "max_tokens"),
        ("temperature", "temperature"),
        ("top_p", "top_p"),
    ] {
        if let Some(value) = request.get(source).filter(|value| !value.is_null()) {
            output.insert(target.to_string(), value.clone());
        }
    }
    Ok(Value::Object(output))
}

fn push_chat_message(messages: &mut Vec<Value>, mut message: Value) {
    if message.get("role").and_then(Value::as_str) == Some("assistant")
        && let Some(tool_calls) = message.get_mut("tool_calls").and_then(Value::as_array_mut)
        && let Some(previous) = messages.last_mut()
        && previous.get("role").and_then(Value::as_str) == Some("assistant")
        && let Some(previous_calls) = previous.get_mut("tool_calls").and_then(Value::as_array_mut)
    {
        previous_calls.append(tool_calls);
        return;
    }
    messages.push(message);
}

fn response_input_to_chat_message(item: &Value) -> Result<Option<Value>, String> {
    match item.get("type").and_then(Value::as_str) {
        Some("message") | None => {
            let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
            let content = response_content_to_chat(item.get("content"));
            Ok(Some(json!({"role": role, "content": content})))
        }
        Some("function_call") => {
            let call_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("call_unknown");
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let arguments = item
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            Ok(Some(json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments}
                }]
            })))
        }
        Some("function_call_output") => {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or("call_unknown");
            let output = item.get("output").map(value_to_text).unwrap_or_default();
            Ok(Some(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": output
            })))
        }
        Some("reasoning") | Some("compaction") => Ok(None),
        Some(other) => Err(format!(
            "Chat 兼容桥暂不支持 Responses input 类型 {other}。"
        )),
    }
}

fn response_content_to_chat(content: Option<&Value>) -> Value {
    match content {
        Some(Value::String(content)) => json!(content),
        Some(Value::Array(parts)) => {
            let converted = parts
                .iter()
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("input_text") | Some("output_text") | Some("text") => Some(json!({
                        "type": "text",
                        "text": part.get("text").and_then(Value::as_str).unwrap_or_default()
                    })),
                    Some("input_image") => part.get("image_url").and_then(Value::as_str).map(
                        |image_url| json!({"type": "image_url", "image_url": {"url": image_url}}),
                    ),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if converted.len() == 1
                && converted[0].get("type").and_then(Value::as_str) == Some("text")
            {
                converted[0].get("text").cloned().unwrap_or(Value::Null)
            } else {
                Value::Array(converted)
            }
        }
        Some(other) => json!(value_to_text(other)),
        None => Value::Null,
    }
}

fn value_to_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn response_tool_to_chat(tool: &Value) -> Option<Value> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return None;
    }
    let mut function = Map::new();
    for key in ["name", "description", "parameters", "strict"] {
        if let Some(value) = tool.get(key) {
            function.insert(key.to_string(), value.clone());
        }
    }
    Some(json!({"type": "function", "function": function}))
}

fn response_tool_choice_to_chat(tool_choice: &Value) -> Value {
    if let Some(choice) = tool_choice.as_str() {
        return json!(choice);
    }
    if tool_choice.get("type").and_then(Value::as_str) == Some("function")
        && let Some(name) = tool_choice.get("name")
    {
        return json!({"type": "function", "function": {"name": name}});
    }
    tool_choice.clone()
}

#[derive(Debug)]
struct MessageOutput {
    output_index: usize,
    id: String,
    text: String,
    opened: bool,
    closed: bool,
}

#[derive(Debug)]
struct ToolOutput {
    output_index: usize,
    id: String,
    call_id: String,
    name: String,
    arguments: String,
    opened: bool,
    closed: bool,
}

struct ResponseStream {
    response_id: String,
    created_at: u64,
    model: String,
    sequence: u64,
    next_output_index: usize,
    message: Option<MessageOutput>,
    tools: HashMap<usize, ToolOutput>,
    tool_order: Vec<usize>,
    usage: Value,
    done: bool,
}

impl ResponseStream {
    fn new(model: String) -> Self {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let unique = EVENT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            response_id: format!("resp_xray_{created_at}_{unique}"),
            created_at,
            model,
            sequence: 0,
            next_output_index: 0,
            message: None,
            tools: HashMap::new(),
            tool_order: Vec::new(),
            usage: empty_usage(),
            done: false,
        }
    }

    fn start(&mut self) -> Vec<String> {
        let response = self.response_value("in_progress", Vec::new());
        vec![self.frame(json!({
            "type": "response.created",
            "response": response
        }))]
    }

    fn consume_chat_chunk(&mut self, chunk: &Value) -> Vec<String> {
        if let Some(model) = chunk.get("model").and_then(Value::as_str) {
            self.model = model.to_string();
        }
        if let Some(usage) = chunk.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = chat_usage_to_responses(usage);
        }
        let mut frames = Vec::new();
        for choice in chunk
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let delta = choice.get("delta").unwrap_or(&Value::Null);
            if let Some(content) = delta.get("content").and_then(Value::as_str)
                && !content.is_empty()
            {
                self.push_text(content, &mut frames);
            }
            for tool in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                self.push_tool_delta(tool, &mut frames);
            }
        }
        frames
    }

    fn consume_chat_response(&mut self, response: &Value) -> Vec<String> {
        if let Some(model) = response.get("model").and_then(Value::as_str) {
            self.model = model.to_string();
        }
        if let Some(usage) = response.get("usage") {
            self.usage = chat_usage_to_responses(usage);
        }
        let mut frames = Vec::new();
        for choice in response
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let message = choice.get("message").unwrap_or(&Value::Null);
            if let Some(content) = message.get("content").and_then(Value::as_str)
                && !content.is_empty()
            {
                self.push_text(content, &mut frames);
            }
            for tool in message
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                self.push_tool_delta(tool, &mut frames);
            }
        }
        frames
    }

    fn push_text(&mut self, delta: &str, frames: &mut Vec<String>) {
        if self.message.is_none() {
            let output_index = self.next_output_index;
            self.next_output_index += 1;
            self.message = Some(MessageOutput {
                output_index,
                id: format!("msg_{}", self.response_id),
                text: String::new(),
                opened: false,
                closed: false,
            });
        }
        let mut events = Vec::new();
        {
            let message = self.message.as_mut().expect("message initialized");
            if !message.opened {
                message.opened = true;
                events.push(json!({
                    "type": "response.output_item.added",
                    "output_index": message.output_index,
                    "item": {
                        "id": message.id,
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": []
                    }
                }));
                events.push(json!({
                    "type": "response.content_part.added",
                    "item_id": message.id,
                    "output_index": message.output_index,
                    "content_index": 0,
                    "part": {
                        "type": "output_text",
                        "text": "",
                        "annotations": [],
                        "logprobs": []
                    }
                }));
            }
            message.text.push_str(delta);
            events.push(json!({
                "type": "response.output_text.delta",
                "item_id": message.id,
                "output_index": message.output_index,
                "content_index": 0,
                "delta": delta,
                "logprobs": []
            }));
        }
        frames.extend(events.into_iter().map(|event| self.frame(event)));
    }

    fn push_tool_delta(&mut self, tool: &Value, frames: &mut Vec<String>) {
        let index = tool
            .get("index")
            .and_then(Value::as_u64)
            .unwrap_or(self.tools.len() as u64) as usize;
        let function = tool.get("function").unwrap_or(&Value::Null);
        let call_id = tool
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let arguments = function
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !self.tools.contains_key(&index) {
            let output_index = self.next_output_index;
            self.next_output_index += 1;
            self.tool_order.push(index);
            self.tools.insert(
                index,
                ToolOutput {
                    output_index,
                    id: format!("fc_{}_{}", self.response_id, index),
                    call_id: if call_id.is_empty() {
                        format!("call_{}_{}", self.response_id, index)
                    } else {
                        call_id.clone()
                    },
                    name: String::new(),
                    arguments: String::new(),
                    opened: false,
                    closed: false,
                },
            );
        }
        let mut events = Vec::new();
        {
            let tool = self.tools.get_mut(&index).expect("tool initialized");
            if !call_id.is_empty() {
                tool.call_id = call_id;
            }
            if !name.is_empty() {
                if tool.name.is_empty() || name.starts_with(&tool.name) {
                    tool.name = name;
                } else if !tool.name.ends_with(&name) {
                    tool.name.push_str(&name);
                }
            }
            if !tool.opened && !tool.name.is_empty() {
                tool.opened = true;
                events.push(json!({
                    "type": "response.output_item.added",
                    "output_index": tool.output_index,
                    "item": {
                        "id": tool.id,
                        "type": "function_call",
                        "status": "in_progress",
                        "call_id": tool.call_id,
                        "name": tool.name,
                        "arguments": ""
                    }
                }));
            }
            if !arguments.is_empty() {
                tool.arguments.push_str(&arguments);
                if tool.opened {
                    events.push(json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": tool.id,
                        "output_index": tool.output_index,
                        "delta": arguments
                    }));
                }
            }
        }
        frames.extend(events.into_iter().map(|event| self.frame(event)));
    }

    fn finish(&mut self) -> Vec<String> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut events = Vec::new();
        if let Some(message) = self.message.as_mut()
            && message.opened
            && !message.closed
        {
            message.closed = true;
            let part = json!({
                "type": "output_text",
                "text": message.text,
                "annotations": [],
                "logprobs": []
            });
            events.push(json!({
                "type": "response.output_text.done",
                "item_id": message.id,
                "output_index": message.output_index,
                "content_index": 0,
                "text": message.text,
                "logprobs": []
            }));
            events.push(json!({
                "type": "response.content_part.done",
                "item_id": message.id,
                "output_index": message.output_index,
                "content_index": 0,
                "part": part
            }));
            events.push(json!({
                "type": "response.output_item.done",
                "output_index": message.output_index,
                "item": message_item(message, "completed")
            }));
        }
        for index in self.tool_order.clone() {
            if let Some(tool) = self.tools.get_mut(&index)
                && !tool.closed
            {
                tool.closed = true;
                if !tool.opened {
                    tool.opened = true;
                    events.push(json!({
                        "type": "response.output_item.added",
                        "output_index": tool.output_index,
                        "item": tool_item(tool, "in_progress")
                    }));
                }
                events.push(json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": tool.id,
                    "output_index": tool.output_index,
                    "arguments": tool.arguments
                }));
                events.push(json!({
                    "type": "response.output_item.done",
                    "output_index": tool.output_index,
                    "item": tool_item(tool, "completed")
                }));
            }
        }
        let output = self.completed_output();
        let response = self.response_value("completed", output);
        events.push(json!({
            "type": "response.completed",
            "response": response
        }));
        events.into_iter().map(|event| self.frame(event)).collect()
    }

    fn fail(&mut self, message: String) -> Vec<String> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut response = self.response_value("failed", self.completed_output());
        response["error"] = json!({
            "code": "chat_bridge_stream_error",
            "message": message
        });
        vec![self.frame(json!({
            "type": "response.failed",
            "response": response
        }))]
    }

    fn completed_output(&self) -> Vec<Value> {
        let mut output = Vec::new();
        if let Some(message) = self.message.as_ref().filter(|message| message.opened) {
            output.push(message_item(message, "completed"));
        }
        for index in &self.tool_order {
            if let Some(tool) = self.tools.get(index) {
                output.push(tool_item(tool, "completed"));
            }
        }
        output.sort_by_key(|item| {
            item.get("_output_index")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        });
        for item in &mut output {
            item.as_object_mut()
                .expect("output item object")
                .remove("_output_index");
        }
        output
    }

    fn response_value(&self, status: &str, output: Vec<Value>) -> Value {
        json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "background": false,
            "error": Value::Null,
            "incomplete_details": Value::Null,
            "instructions": Value::Null,
            "max_output_tokens": Value::Null,
            "model": self.model,
            "output": output,
            "parallel_tool_calls": true,
            "previous_response_id": Value::Null,
            "reasoning": {"effort": Value::Null, "summary": Value::Null},
            "store": false,
            "temperature": Value::Null,
            "text": {"format": {"type": "text"}},
            "tool_choice": "auto",
            "tools": [],
            "top_p": Value::Null,
            "truncation": "disabled",
            "usage": if status == "in_progress" { Value::Null } else { self.usage.clone() },
            "user": Value::Null,
            "metadata": {}
        })
    }

    fn frame(&mut self, mut event: Value) -> String {
        event["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");
        format!("event: {event_type}\ndata: {event}\n\n")
    }
}

fn message_item(message: &MessageOutput, status: &str) -> Value {
    json!({
        "_output_index": message.output_index,
        "id": message.id,
        "type": "message",
        "status": status,
        "role": "assistant",
        "content": [{
            "type": "output_text",
            "text": message.text,
            "annotations": [],
            "logprobs": []
        }]
    })
}

fn tool_item(tool: &ToolOutput, status: &str) -> Value {
    json!({
        "_output_index": tool.output_index,
        "id": tool.id,
        "type": "function_call",
        "status": status,
        "call_id": tool.call_id,
        "name": tool.name,
        "arguments": tool.arguments
    })
}

fn empty_usage() -> Value {
    json!({
        "input_tokens": 0,
        "input_tokens_details": {"cached_tokens": 0},
        "output_tokens": 0,
        "output_tokens_details": {"reasoning_tokens": 0},
        "total_tokens": 0
    })
}

fn chat_usage_to_responses(usage: &Value) -> Value {
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let cached_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("prompt_cache_hit_tokens").and_then(Value::as_u64))
        .or_else(|| usage.get("cached_tokens").and_then(Value::as_u64))
        .unwrap_or_default();
    let output_tokens = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let reasoning_tokens = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input_tokens + output_tokens);
    json!({
        "input_tokens": input_tokens,
        "input_tokens_details": {"cached_tokens": cached_tokens},
        "output_tokens": output_tokens,
        "output_tokens_details": {"reasoning_tokens": reasoning_tokens},
        "total_tokens": total_tokens
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;

    #[test]
    fn converts_responses_messages_and_tools_to_chat() {
        let request = json!({
            "model": "deepseek-chat",
            "instructions": "Be concise.",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "List files"}]},
                {"type": "function_call", "call_id": "call_1", "name": "exec", "arguments": "{\"cmd\":\"ls\"}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "README.md"}
            ],
            "tools": [
                {"type": "function", "name": "exec", "description": "Run a command", "parameters": {"type": "object"}},
                {"type": "web_search"}
            ],
            "tool_choice": "auto",
            "parallel_tool_calls": false
        });
        let chat = responses_to_chat(&request).expect("convert request");
        assert_eq!(chat["model"], "deepseek-chat");
        assert_eq!(chat["messages"].as_array().expect("messages").len(), 4);
        assert_eq!(chat["tools"].as_array().expect("tools").len(), 1);
        assert_eq!(chat["messages"][2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(chat["messages"][3]["role"], "tool");
    }

    #[test]
    fn groups_parallel_function_calls_into_one_assistant_message() {
        let chat = responses_to_chat(&json!({
            "model": "chat-model",
            "input": [
                {"type": "message", "role": "user", "content": "Inspect both"},
                {"type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{\"path\":\"a\"}"},
                {"type": "function_call", "call_id": "call_2", "name": "read_file", "arguments": "{\"path\":\"b\"}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "A"},
                {"type": "function_call_output", "call_id": "call_2", "output": "B"}
            ]
        }))
        .expect("convert parallel calls");
        let messages = chat["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 4);
        assert_eq!(
            messages[1]["tool_calls"].as_array().expect("calls").len(),
            2
        );
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[3]["role"], "tool");
    }

    #[test]
    fn converts_chat_text_and_tool_stream_to_responses_events() {
        let mut stream = ResponseStream::new("deepseek-chat".to_string());
        let mut frames = stream.start();
        frames.extend(stream.consume_chat_chunk(&json!({
            "model": "deepseek-chat",
            "choices": [{"delta": {"content": "Checking "}}]
        })));
        frames.extend(stream.consume_chat_chunk(&json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "id": "call_1",
                "type": "function",
                "function": {"name": "exec", "arguments": "{\"cmd\":"}
            }]}}]
        })));
        frames.extend(stream.consume_chat_chunk(&json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "function": {"name": "exec", "arguments": "\"ls\"}"}
            }]}}],
            "usage": {
                "prompt_tokens": 20,
                "prompt_cache_hit_tokens": 12,
                "completion_tokens": 4,
                "total_tokens": 24
            }
        })));
        frames.extend(stream.finish());
        let output = frames.concat();
        assert!(output.contains("response.output_text.delta"));
        assert!(output.contains("response.function_call_arguments.delta"));
        assert!(output.contains("\\\"cmd\\\":\\\"ls\\\""));
        assert!(!output.contains("\"name\":\"execexec\""));
        assert!(output.contains("\"input_tokens\":20"));
        assert!(output.contains("\"cached_tokens\":12"));
        assert!(output.contains("response.completed"));
    }

    #[test]
    fn bridge_url_is_stable_and_provider_scoped() {
        assert_eq!(
            provider_base_url("deepseek"),
            "http://127.0.0.1:32198/v1/chat/deepseek"
        );
        assert!(is_provider_bridge_url(
            "deepseek",
            "http://127.0.0.1:32198/v1/chat/deepseek/"
        ));
    }

    #[tokio::test]
    async fn translates_an_http_chat_stream_end_to_end() {
        let upstream = Router::new().route(
            "/chat/completions",
            post(|| async {
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    concat!(
                        "data: {\"model\":\"chat-test\",\"choices\":[{\"delta\":{\"content\":\"Ready \"}}]}\n\n",
                        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"exec\",\"arguments\":\"{\\\"cmd\\\":\\\"pwd\\\"}\"}}]}}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3,\"total_tokens\":15}}\n\n",
                        "data: [DONE]\n\n"
                    ),
                )
            }),
        );
        let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("upstream listener");
        let upstream_address = upstream_listener.local_addr().expect("upstream address");
        let upstream_task = tokio::spawn(async move {
            axum::serve(upstream_listener, upstream)
                .await
                .expect("serve upstream");
        });

        let config_path = std::env::temp_dir().join(format!(
            "codex-xray-chat-bridge-test-{}.json",
            EVENT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let state = ChatBridgeState::load(config_path.clone()).expect("bridge state");
        state
            .persist_provider(
                "chat-test",
                ChatBridgeProvider {
                    upstream_base_url: format!("http://{upstream_address}"),
                    credential_mode: "none".to_string(),
                    env_key: None,
                    model: "chat-test".to_string(),
                    context_window: 128_000,
                },
            )
            .expect("persist test provider");
        let bridge_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bridge listener");
        let bridge_address = bridge_listener.local_addr().expect("bridge address");
        let bridge_task = tokio::spawn(async move {
            axum::serve(bridge_listener, bridge_router(state))
                .await
                .expect("serve bridge");
        });

        let response = reqwest::Client::new()
            .post(format!(
                "http://{bridge_address}/v1/chat/chat-test/responses"
            ))
            .json(&json!({
                "model": "chat-test",
                "input": "Call pwd",
                "tools": [{
                    "type": "function",
                    "name": "exec",
                    "parameters": {"type": "object"}
                }],
                "stream": true
            }))
            .send()
            .await
            .expect("bridge request");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.expect("bridge response");
        assert!(body.contains("response.output_text.delta"));
        assert!(body.contains("response.function_call_arguments.done"));
        assert!(body.contains("\"name\":\"exec\""));
        assert!(!body.contains("execexec"));
        assert!(body.contains("\"input_tokens\":12"));
        assert!(body.contains("response.completed"));

        let catalog = reqwest::Client::new()
            .get(format!(
                "http://{bridge_address}/v1/chat/chat-test/models?client_version=0.0.0"
            ))
            .send()
            .await
            .expect("model catalog request");
        assert_eq!(catalog.status(), StatusCode::OK);
        let catalog = catalog.json::<Value>().await.expect("model catalog");
        assert_eq!(catalog["models"][0]["slug"], "chat-test");
        assert_eq!(catalog["models"][0]["context_window"], 128_000);

        bridge_task.abort();
        upstream_task.abort();
        let _ = fs::remove_file(config_path);
    }

    #[tokio::test]
    #[ignore = "requires a live OpenAI-compatible Chat Completions provider"]
    async fn translates_a_live_chat_tool_round_trip() {
        let upstream_base_url =
            std::env::var("CODEX_XRAY_LIVE_CHAT_BASE_URL").expect("live Chat base URL");
        let api_key = std::env::var("CODEX_XRAY_LIVE_CHAT_API_KEY").expect("live Chat API key");
        let model = std::env::var("CODEX_XRAY_LIVE_CHAT_MODEL").expect("live Chat model");
        let config_path = std::env::temp_dir().join(format!(
            "codex-xray-live-chat-bridge-test-{}.json",
            EVENT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let state = ChatBridgeState::load(config_path.clone()).expect("bridge state");
        state
            .persist_provider(
                "live-chat",
                ChatBridgeProvider {
                    upstream_base_url,
                    credential_mode: "environment".to_string(),
                    env_key: Some("CODEX_XRAY_LIVE_CHAT_API_KEY".to_string()),
                    model: model.clone(),
                    context_window: 128_000,
                },
            )
            .expect("persist live provider");
        let bridge_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bridge listener");
        let bridge_address = bridge_listener.local_addr().expect("bridge address");
        let bridge_task = tokio::spawn(async move {
            axum::serve(bridge_listener, bridge_router(state))
                .await
                .expect("serve bridge");
        });
        let client = reqwest::Client::new();
        let tools = json!([{
            "type": "function",
            "name": "get_test_value",
            "description": "Return a test value",
            "parameters": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"]
            }
        }]);
        let first = client
            .post(format!(
                "http://{bridge_address}/v1/chat/live-chat/responses"
            ))
            .bearer_auth(&api_key)
            .json(&json!({
                "model": model,
                "input": "You must call get_test_value with name xray. Do not answer directly.",
                "tools": tools,
                "tool_choice": "auto",
                "stream": true
            }))
            .send()
            .await
            .expect("first bridge request");
        assert_eq!(first.status(), StatusCode::OK);
        let first_body = first.text().await.expect("first bridge response");
        let first_events = sse_json_events(&first_body);
        let function_call = first_events
            .iter()
            .find(|event| {
                event.get("type").and_then(Value::as_str) == Some("response.output_item.done")
                    && event.pointer("/item/type").and_then(Value::as_str) == Some("function_call")
            })
            .and_then(|event| event.get("item"))
            .expect("function call output");
        assert_eq!(function_call["name"], "get_test_value");
        assert!(
            function_call["arguments"]
                .as_str()
                .is_some_and(|arguments| arguments.contains("xray"))
        );

        let call_id = function_call["call_id"].as_str().expect("function call id");
        let arguments = function_call["arguments"]
            .as_str()
            .expect("function arguments");
        let second = client
            .post(format!(
                "http://{bridge_address}/v1/chat/live-chat/responses"
            ))
            .bearer_auth(&api_key)
            .json(&json!({
                "model": model,
                "input": [
                    {
                        "type": "message",
                        "role": "user",
                        "content": "Call get_test_value, then reply with exactly the returned value."
                    },
                    {
                        "type": "function_call",
                        "call_id": call_id,
                        "name": "get_test_value",
                        "arguments": arguments
                    },
                    {
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": "XRAY_TOOL_OK"
                    }
                ],
                "tools": tools,
                "tool_choice": "auto",
                "stream": true
            }))
            .send()
            .await
            .expect("second bridge request");
        assert_eq!(second.status(), StatusCode::OK);
        let second_body = second.text().await.expect("second bridge response");
        let second_events = sse_json_events(&second_body);
        let output = second_events
            .iter()
            .filter(|event| {
                event.get("type").and_then(Value::as_str) == Some("response.output_text.delta")
            })
            .filter_map(|event| event.get("delta").and_then(Value::as_str))
            .collect::<String>();
        assert_eq!(output.trim(), "XRAY_TOOL_OK");
        assert!(second_events.iter().any(|event| {
            event.get("type").and_then(Value::as_str) == Some("response.completed")
                && event
                    .pointer("/response/usage/total_tokens")
                    .and_then(Value::as_u64)
                    .is_some_and(|tokens| tokens > 0)
        }));

        bridge_task.abort();
        let _ = fs::remove_file(config_path);
    }

    fn sse_json_events(body: &str) -> Vec<Value> {
        body.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|data| *data != "[DONE]")
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .collect()
    }
}
