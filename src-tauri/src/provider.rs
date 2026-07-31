use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use url::Url;

use crate::chat_bridge::{ChatBridgeState, is_provider_bridge_url, provider_base_url};

const CREDENTIAL_HELPER_ARG: &str = "--codex-xray-credential";
const FILE_CREDENTIAL_HELPER_ARG: &str = "--codex-xray-file-credential";
const PROFILE_FILE_VERSION: u32 = 1;

const BUILTIN_PROVIDERS: &[(&str, &str, Option<&str>)] = &[
    ("openai", "OpenAI", None),
    ("ollama", "Ollama", Some("http://localhost:11434/v1")),
    ("lmstudio", "LM Studio", Some("http://localhost:1234/v1")),
    ("amazon-bedrock", "Amazon Bedrock", None),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDefinition {
    pub id: String,
    pub name: String,
    pub base_url: Option<String>,
    pub env_key: Option<String>,
    pub env_available: bool,
    pub credential_source: String,
    pub credential_available: bool,
    pub credential_path: Option<String>,
    pub auth_command: Option<String>,
    pub auth_args: Vec<String>,
    pub wire_api: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    pub context_window: Option<u64>,
    pub builtin: bool,
    pub compatibility: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub fetched_at: String,
    pub config_path: String,
    pub version: Option<String>,
    pub active_provider: String,
    pub active_model: Option<String>,
    pub models: Vec<ModelOption>,
    pub providers: Vec<ProviderDefinition>,
    pub profiles: Vec<ProviderProfile>,
    pub restore_available: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub base_url: Option<String>,
    pub env_key: Option<String>,
    pub credential_mode: String,
    pub credential_available: bool,
    #[serde(default)]
    pub credential_path: Option<String>,
    pub protocol: String,
    pub context_window: u64,
    pub builtin: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderProfileFile {
    version: u32,
    profiles: BTreeMap<String, ProviderProfile>,
}

impl Default for ProviderProfileFile {
    fn default() -> Self {
        Self {
            version: PROFILE_FILE_VERSION,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub display_name: String,
    pub default_reasoning_effort: Option<String>,
    pub supported_reasoning_efforts: Vec<String>,
    pub supports_personality: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderApplyRequest {
    pub profile_name: Option<String>,
    pub provider_id: String,
    pub model: String,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub env_key: Option<String>,
    pub credential_mode: Option<String>,
    pub api_key: Option<String>,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default = "default_context_window")]
    pub context_window: u64,
    pub expected_version: Option<String>,
}

fn default_context_window() -> u64 {
    128_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderTestResult {
    pub success: bool,
    pub check_kind: String,
    pub provider_id: String,
    pub model: String,
    pub endpoint: Option<String>,
    pub latency_ms: u128,
    pub http_status: Option<u16>,
    pub message: String,
    pub capabilities: Vec<ProviderCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapability {
    pub capability: String,
    pub status: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRestorePoint {
    provider_id: String,
    model: Option<String>,
    definition: Option<ProviderDefinition>,
}

fn codex_home() -> PathBuf {
    env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn is_builtin(id: &str) -> bool {
    BUILTIN_PROVIDERS
        .iter()
        .any(|(builtin_id, _, _)| *builtin_id == id)
}

fn default_protocol() -> String {
    "responses".to_string()
}

fn credential_file_path(provider_id: &str) -> Result<PathBuf, String> {
    let provider_id = validate_identifier(provider_id, "Provider ID")?;
    Ok(codex_home()
        .join("codex-xray")
        .join("credentials")
        .join(format!("{provider_id}.key")))
}

fn credential_file_display_path(provider_id: &str) -> Option<String> {
    credential_file_path(provider_id)
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

pub(crate) fn read_file_credential(provider_id: &str) -> Result<String, String> {
    let path = credential_file_path(provider_id)?;
    let value = fs::read_to_string(&path)
        .map_err(|error| format!("无法读取凭据文件 {}：{error}", path.display()))?;
    validate_api_key(value)
}

fn file_credential_exists(provider_id: &str) -> bool {
    credential_file_path(provider_id)
        .ok()
        .is_some_and(|path| path.is_file())
}

fn save_file_credential(provider_id: &str, api_key: &str) -> Result<(), String> {
    let api_key = validate_api_key(api_key.to_string())?;
    let path = credential_file_path(provider_id)?;
    let parent = path
        .parent()
        .ok_or_else(|| "无法确定凭据文件目录。".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("无法创建凭据文件目录 {}：{error}", parent.display()))?;
    #[cfg(unix)]
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("无法限制凭据目录权限：{error}"))?;

    let temporary = path.with_extension("key.tmp");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("无法创建临时凭据文件：{error}"))?;
    file.write_all(api_key.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("无法写入凭据文件：{error}"))?;
    fs::rename(&temporary, &path).map_err(|error| format!("无法保存凭据文件：{error}"))?;
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("无法限制凭据文件权限：{error}"))?;
    Ok(())
}

fn delete_file_credential(provider_id: &str) -> Result<(), String> {
    let path = credential_file_path(provider_id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("无法删除凭据文件 {}：{error}", path.display())),
    }
}

pub fn delete_credential(provider_id: &str, mode: Option<&str>) -> Result<(), String> {
    let provider_id = validate_identifier(provider_id, "Provider ID")?;
    if mode != Some("file") {
        return Err("当前只支持删除 X-Ray 本机保存的 API Key。".to_string());
    }
    delete_file_credential(&provider_id)
}

fn validate_api_key(value: String) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err("API Key 不能为空。".to_string());
    }
    if value.len() > 32 * 1024 || value.contains(['\r', '\n', '\0']) {
        return Err("API Key 过长或包含无效控制字符。".to_string());
    }
    Ok(value)
}

fn credential_helper_definition(provider_id: &str) -> Result<(String, Vec<String>), String> {
    let executable =
        env::current_exe().map_err(|error| format!("无法定位 Codex X-Ray 可执行文件：{error}"))?;
    Ok((
        executable.to_string_lossy().into_owned(),
        vec![
            FILE_CREDENTIAL_HELPER_ARG.to_string(),
            provider_id.to_string(),
        ],
    ))
}

fn auth_definition(value: &Value) -> (Option<String>, Vec<String>) {
    let command = value
        .get("auth")
        .and_then(|auth| string_field(auth, "command"));
    let args = value
        .pointer("/auth/args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    (command, args)
}

fn xray_credential_helper_mode(args: &[String], provider_id: &str) -> Option<&'static str> {
    match args {
        [helper, id] if helper == CREDENTIAL_HELPER_ARG && id == provider_id => Some("keychain"),
        [helper, id] if helper == FILE_CREDENTIAL_HELPER_ARG && id == provider_id => Some("file"),
        _ => None,
    }
}

fn compatibility_for(base_url: Option<&str>, wire_api: &str) -> String {
    if base_url.is_some_and(is_known_chat_only_url) {
        return "chat_only".to_string();
    }
    if wire_api != "responses" {
        return "unsupported_wire_api".to_string();
    }
    "responses".to_string()
}

fn definition_from_value(id: &str, value: &Value) -> ProviderDefinition {
    let name = string_field(value, "name").unwrap_or_else(|| id.to_string());
    let base_url = string_field(value, "base_url");
    let env_key = string_field(value, "env_key");
    let env_available = env_key
        .as_deref()
        .is_some_and(|key| env::var_os(key).is_some());
    let (auth_command, auth_args) = auth_definition(value);
    let xray_credential_mode = auth_command
        .as_ref()
        .and_then(|_| xray_credential_helper_mode(&auth_args, id));
    let credential_source = if let Some(mode) = xray_credential_mode {
        mode
    } else if auth_command.is_some() {
        "command"
    } else if env_key.is_some() {
        "environment"
    } else {
        "none"
    }
    .to_string();
    let credential_available = if xray_credential_mode == Some("keychain") {
        false
    } else if xray_credential_mode == Some("file") {
        file_credential_exists(id)
    } else if auth_command.is_some() {
        true
    } else if env_key.is_some() {
        env_available
    } else {
        true
    };
    let wire_api = string_field(value, "wire_api").unwrap_or_else(|| "responses".to_string());
    ProviderDefinition {
        id: id.to_string(),
        name,
        compatibility: compatibility_for(base_url.as_deref(), &wire_api),
        base_url,
        env_key,
        env_available,
        credential_source,
        credential_available,
        credential_path: (xray_credential_mode == Some("file"))
            .then(|| credential_file_display_path(id))
            .flatten(),
        auth_command,
        auth_args,
        wire_api,
        protocol: "responses".to_string(),
        context_window: None,
        builtin: is_builtin(id),
    }
}

fn provider_map(config: &Value) -> Map<String, Value> {
    config
        .get("model_providers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn user_layer(raw: &Value) -> Option<&Value> {
    raw.get("layers")
        .and_then(Value::as_array)?
        .iter()
        .find(|layer| {
            layer
                .pointer("/name/type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "user")
                && layer.pointer("/name/profile").is_none_or(Value::is_null)
        })
}

pub fn build_provider_snapshot(
    raw: &Value,
    restore_path: &Path,
) -> Result<ProviderSnapshot, String> {
    let config = raw
        .get("config")
        .ok_or_else(|| "config/read 缺少 config 字段".to_string())?;
    let active_provider =
        string_field(config, "model_provider").unwrap_or_else(|| "openai".to_string());
    let active_model = string_field(config, "model");
    let configured = provider_map(config);
    let mut providers = Vec::new();

    for (id, name, base_url) in BUILTIN_PROVIDERS {
        let mut definition = configured
            .get(*id)
            .map(|value| definition_from_value(id, value))
            .unwrap_or_else(|| ProviderDefinition {
                id: (*id).to_string(),
                name: (*name).to_string(),
                base_url: base_url.map(ToOwned::to_owned),
                env_key: None,
                env_available: true,
                credential_source: "builtin".to_string(),
                credential_available: true,
                credential_path: None,
                auth_command: None,
                auth_args: Vec::new(),
                wire_api: "responses".to_string(),
                protocol: "responses".to_string(),
                context_window: None,
                builtin: true,
                compatibility: "responses".to_string(),
            });
        definition.builtin = true;
        providers.push(definition);
    }
    for (id, value) in configured {
        if !is_builtin(&id) {
            providers.push(definition_from_value(&id, &value));
        }
    }
    providers.sort_by(|left, right| {
        right
            .id
            .eq(&active_provider)
            .cmp(&left.id.eq(&active_provider))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    let layer = user_layer(raw);
    let config_path = layer
        .and_then(|layer| layer.pointer("/name/file"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            codex_home()
                .join("config.toml")
                .to_string_lossy()
                .into_owned()
        });
    let version = layer
        .and_then(|layer| layer.get("version"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    let mut warnings = Vec::new();
    if let Some(active) = providers.iter().find(|item| item.id == active_provider) {
        if active.compatibility == "chat_only" {
            warnings.push(
                "当前地址只公布了 Chat Completions；Codex 自定义 Provider 需要 Responses API。"
                    .to_string(),
            );
        } else if active.compatibility != "responses" {
            warnings.push("当前 Provider 不是 Codex 支持的 Responses API。".to_string());
        }
        if !active.credential_available {
            warnings.push(format!(
                "当前 Provider 的{}凭据不可用。",
                if active.credential_source == "keychain" {
                    "旧版"
                } else if active.credential_source == "file" {
                    "本机"
                } else {
                    "环境变量"
                }
            ));
        }
    } else {
        warnings.push(format!(
            "当前 Provider {active_provider} 没有可读取的定义。"
        ));
    }

    Ok(ProviderSnapshot {
        fetched_at: Utc::now().to_rfc3339(),
        config_path,
        version,
        active_provider,
        active_model,
        models: Vec::new(),
        providers,
        profiles: Vec::new(),
        restore_available: restore_path.is_file(),
        warnings,
    })
}

pub fn enrich_provider_snapshot_with_bridge(
    snapshot: &mut ProviderSnapshot,
    bridge: &ChatBridgeState,
) {
    for provider in &mut snapshot.providers {
        let Some(mapping) = bridge.provider(&provider.id) else {
            continue;
        };
        if provider
            .base_url
            .as_deref()
            .is_some_and(|base_url| is_provider_bridge_url(&provider.id, base_url))
        {
            provider.base_url = Some(mapping.upstream_base_url);
            provider.protocol = "chat_completions".to_string();
            provider.context_window = Some(mapping.context_window);
            provider.compatibility = "chat_bridge".to_string();
        }
    }
    if let Some(active) = snapshot
        .providers
        .iter()
        .find(|provider| provider.id == snapshot.active_provider)
        && active.protocol == "chat_completions"
        && !bridge.status().running
    {
        snapshot.warnings.push(
            "当前 Provider 使用 Chat 兼容桥，但本地桥没有运行；请保持 Codex X-Ray 打开。"
                .to_string(),
        );
    }
}

fn validate_identifier(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
    {
        return Err(format!(
            "{label} 只能包含字母、数字、连字符和下划线，且长度不超过 64。"
        ));
    }
    Ok(value.to_string())
}

fn validate_model(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 || value.contains(['\r', '\n', '\0']) {
        return Err("模型名称不能为空，且不能包含换行或控制字符。".to_string());
    }
    Ok(value.to_string())
}

fn validate_context_window(value: u64) -> Result<u64, String> {
    if !(8_192..=4_000_000).contains(&value) {
        return Err("上下文窗口必须在 8,192 到 4,000,000 Token 之间。".to_string());
    }
    Ok(value)
}

fn validate_provider_url(value: &str, allow_chat_only: bool) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    let parsed = Url::parse(value).map_err(|_| "Provider URL 不是有效 URL。".to_string())?;
    let local_http = parsed.scheme() == "http"
        && parsed
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
    if parsed.scheme() != "https" && !local_http {
        return Err("远程 Provider 必须使用 HTTPS；HTTP 只允许 localhost。".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Provider URL 不能内嵌用户名或密码。".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("Provider URL 不能包含查询参数或片段。".to_string());
    }
    if !allow_chat_only && is_known_chat_only_url(value) {
        return Err(
            "这个厂商直连地址目前只公布了 Chat Completions，不能直接用于只接受 Responses API 的 Codex。请选择支持 Responses 的云平台入口，或填写 Responses 适配网关。"
                .to_string(),
        );
    }
    Ok(value.to_string())
}

fn validate_base_url(value: &str) -> Result<String, String> {
    validate_provider_url(value, false)
}

fn is_known_chat_only_url(value: &str) -> bool {
    Url::parse(value).ok().is_some_and(|url| {
        url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("open.bigmodel.cn")
                || host.eq_ignore_ascii_case("api.z.ai")
                || host.eq_ignore_ascii_case("api.deepseek.com")
                || host.eq_ignore_ascii_case("api.moonshot.cn")
                || host.eq_ignore_ascii_case("api.minimax.chat")
                || host.eq_ignore_ascii_case("api.hunyuan.cloud.tencent.com")
                || host.eq_ignore_ascii_case("api.siliconflow.cn")
        })
    })
}

fn credential_mode(request: &ProviderApplyRequest) -> Result<&str, String> {
    let mode = request.credential_mode.as_deref().unwrap_or_else(|| {
        if request.env_key.is_some() {
            "environment"
        } else {
            "none"
        }
    });
    if !["file", "environment", "none"].contains(&mode) {
        return Err("凭据方式不是 Codex X-Ray 支持的值。".to_string());
    }
    Ok(mode)
}

fn provider_protocol(request: &ProviderApplyRequest) -> Result<&str, String> {
    let protocol = request.protocol.trim();
    if !["responses", "chat_completions"].contains(&protocol) {
        return Err("接口协议必须是 Responses 或 Chat Completions。".to_string());
    }
    Ok(protocol)
}

fn custom_definition(request: &ProviderApplyRequest) -> Result<ProviderDefinition, String> {
    let id = validate_identifier(&request.provider_id, "Provider ID")?;
    let name = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&id)
        .to_string();
    if name.len() > 96 || name.contains(['\r', '\n', '\0']) {
        return Err("Provider 名称过长或包含控制字符。".to_string());
    }
    let protocol = provider_protocol(request)?;
    let context_window = if protocol == "chat_completions" {
        Some(validate_context_window(request.context_window)?)
    } else {
        None
    };
    let upstream_base_url = request
        .base_url
        .as_deref()
        .ok_or_else(|| "自定义 Provider 必须填写 API Base URL。".to_string())
        .and_then(|value| validate_provider_url(value, protocol == "chat_completions"))?;
    let base_url = if protocol == "chat_completions" {
        provider_base_url(&id)
    } else {
        upstream_base_url
    };
    let mode = credential_mode(request)?;
    let env_key = if mode == "environment" {
        Some(
            request
                .env_key
                .as_deref()
                .ok_or_else(|| "使用环境变量时必须填写变量名。".to_string())
                .and_then(|value| validate_identifier(value, "环境变量名"))?,
        )
    } else {
        None
    };
    let (auth_command, auth_args) = if mode == "file" {
        let (command, args) = credential_helper_definition(&id)?;
        (Some(command), args)
    } else {
        (None, Vec::new())
    };
    let credential_available = match mode {
        "file" => {
            request
                .api_key
                .as_deref()
                .is_some_and(|value| validate_api_key(value.to_string()).is_ok())
                || file_credential_exists(&id)
        }
        "environment" => env_key
            .as_deref()
            .is_some_and(|key| env::var_os(key).is_some()),
        _ => true,
    };
    let credential_path = if mode == "file" {
        credential_file_display_path(&id)
    } else {
        None
    };
    Ok(ProviderDefinition {
        id,
        name,
        base_url: Some(base_url),
        env_available: env_key
            .as_deref()
            .is_some_and(|key| env::var_os(key).is_some()),
        env_key,
        credential_source: mode.to_string(),
        credential_available,
        credential_path,
        auth_command,
        auth_args,
        wire_api: "responses".to_string(),
        protocol: protocol.to_string(),
        context_window,
        builtin: false,
        compatibility: if protocol == "chat_completions" {
            "chat_bridge".to_string()
        } else {
            "responses".to_string()
        },
    })
}

fn edit(key_path: impl Into<String>, value: Value, merge_strategy: &str) -> Value {
    json!({
        "keyPath": key_path.into(),
        "value": value,
        "mergeStrategy": merge_strategy
    })
}

fn definition_edits(definition: &ProviderDefinition) -> Vec<Value> {
    let prefix = format!("model_providers.{}", definition.id);
    let mut value = Map::from_iter([
        ("name".to_string(), json!(definition.name)),
        ("wire_api".to_string(), json!("responses")),
    ]);
    if let Some(base_url) = &definition.base_url {
        value.insert("base_url".to_string(), json!(base_url));
    }
    if let Some(env_key) = &definition.env_key {
        value.insert("env_key".to_string(), json!(env_key));
    }
    if let Some(command) = &definition.auth_command {
        value.insert(
            "auth".to_string(),
            json!({
                "command": command,
                "args": definition.auth_args,
            }),
        );
    }
    vec![edit(prefix, Value::Object(value), "replace")]
}

pub fn build_apply_edits(
    request: &ProviderApplyRequest,
) -> Result<(Vec<Value>, Option<ProviderDefinition>), String> {
    let provider_id = validate_identifier(&request.provider_id, "Provider ID")?;
    let model = validate_model(&request.model)?;
    if is_builtin(&provider_id) && provider_protocol(request)? != "responses" {
        return Err("Codex 内置 Provider 不能改为 Chat Completions。".to_string());
    }
    let definition = if is_builtin(&provider_id) {
        None
    } else {
        Some(custom_definition(request)?)
    };
    let mut edits = vec![
        edit("model_provider", json!(provider_id), "replace"),
        edit("model", json!(model), "replace"),
    ];
    if let Some(definition) = &definition {
        edits.extend(definition_edits(definition));
    }
    Ok((edits, definition))
}

fn validate_profile_name(value: Option<&str>, fallback: &str) -> Result<String, String> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback);
    if value.len() > 96 || value.contains(['\r', '\n', '\0']) {
        return Err("方案名称过长或包含控制字符。".to_string());
    }
    Ok(value.to_string())
}

fn load_profile_file(path: &Path) -> Result<ProviderProfileFile, String> {
    if !path.is_file() {
        return Ok(ProviderProfileFile::default());
    }
    let content = fs::read(path).map_err(|error| format!("无法读取 Provider 方案：{error}"))?;
    let file = serde_json::from_slice::<ProviderProfileFile>(&content)
        .map_err(|error| format!("Provider 方案无法解析：{error}"))?;
    if file.version != PROFILE_FILE_VERSION {
        return Err(format!("Provider 方案版本 {} 不受支持。", file.version));
    }
    Ok(file)
}

fn save_profile_file(path: &Path, file: &ProviderProfileFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 Provider 方案目录：{error}"))?;
    }
    let content = serde_json::to_vec_pretty(file)
        .map_err(|error| format!("无法序列化 Provider 方案：{error}"))?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| format!("无法保存 Provider 方案：{error}"))
}

fn request_profile(request: &ProviderApplyRequest) -> Result<ProviderProfile, String> {
    let provider_id = validate_identifier(&request.provider_id, "Provider ID")?;
    let model = validate_model(&request.model)?;
    let builtin = is_builtin(&provider_id);
    let definition = if builtin {
        None
    } else {
        Some(custom_definition(request)?)
    };
    let provider_name = definition
        .as_ref()
        .map(|definition| definition.name.clone())
        .or_else(|| {
            BUILTIN_PROVIDERS
                .iter()
                .find(|(id, _, _)| *id == provider_id)
                .map(|(_, name, _)| (*name).to_string())
        })
        .unwrap_or_else(|| provider_id.clone());
    let name = validate_profile_name(request.profile_name.as_deref(), &provider_name)?;
    let credential_mode = if builtin {
        "builtin".to_string()
    } else {
        credential_mode(request)?.to_string()
    };
    let credential_available = match credential_mode.as_str() {
        "builtin" | "none" => true,
        "keychain" => false,
        "file" => {
            request
                .api_key
                .as_deref()
                .is_some_and(|value| validate_api_key(value.to_string()).is_ok())
                || file_credential_exists(&provider_id)
        }
        "environment" => request
            .env_key
            .as_deref()
            .is_some_and(|key| env::var_os(key).is_some()),
        _ => false,
    };
    let credential_path = if credential_mode == "file" {
        credential_file_display_path(&provider_id)
    } else {
        None
    };
    Ok(ProviderProfile {
        id: provider_id.clone(),
        name,
        provider_id,
        provider_name,
        model,
        base_url: if builtin {
            None
        } else {
            request.base_url.clone()
        },
        env_key: if credential_mode == "environment" {
            request.env_key.clone()
        } else {
            None
        },
        credential_mode,
        credential_available,
        credential_path,
        protocol: if builtin {
            "responses".to_string()
        } else {
            provider_protocol(request)?.to_string()
        },
        context_window: if request.protocol == "chat_completions" {
            validate_context_window(request.context_window)?
        } else {
            request.context_window
        },
        builtin,
        updated_at: Utc::now().to_rfc3339(),
    })
}

fn snapshot_profile(
    snapshot: &ProviderSnapshot,
    previous: Option<&ProviderProfile>,
) -> Option<ProviderProfile> {
    let model = snapshot.active_model.as_deref()?.trim();
    if model.is_empty() {
        return None;
    }
    let provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.id == snapshot.active_provider)?;
    Some(ProviderProfile {
        id: provider.id.clone(),
        name: previous
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| provider.name.clone()),
        provider_id: provider.id.clone(),
        provider_name: provider.name.clone(),
        model: model.to_string(),
        base_url: provider.base_url.clone(),
        env_key: provider.env_key.clone(),
        credential_mode: provider.credential_source.clone(),
        credential_available: provider.credential_available,
        credential_path: provider.credential_path.clone(),
        protocol: provider.protocol.clone(),
        context_window: provider.context_window.unwrap_or(128_000),
        builtin: provider.builtin,
        updated_at: Utc::now().to_rfc3339(),
    })
}

pub struct ProfileRollback {
    profile_id: String,
    previous: Option<ProviderProfile>,
    changed: bool,
}

pub fn persist_request_profile(
    path: &Path,
    request: &ProviderApplyRequest,
) -> Result<ProfileRollback, String> {
    let profile = request_profile(request)?;
    let mut file = load_profile_file(path)?;
    let previous = file.profiles.insert(profile.id.clone(), profile.clone());
    let changed = previous.as_ref() != Some(&profile);
    if changed && let Err(error) = save_profile_file(path, &file) {
        if let Some(previous) = previous.clone() {
            file.profiles.insert(profile.id.clone(), previous);
        } else {
            file.profiles.remove(&profile.id);
        }
        return Err(error);
    }
    Ok(ProfileRollback {
        profile_id: profile.id,
        previous,
        changed,
    })
}

pub fn persist_active_profile(path: &Path, snapshot: &ProviderSnapshot) -> Result<(), String> {
    let mut file = load_profile_file(path)?;
    let previous = file.profiles.get(&snapshot.active_provider);
    let Some(mut profile) = snapshot_profile(snapshot, previous) else {
        return Ok(());
    };
    if let Some(previous) = previous
        && previous.name == profile.name
        && previous.provider_name == profile.provider_name
        && previous.model == profile.model
        && previous.base_url == profile.base_url
        && previous.env_key == profile.env_key
        && previous.credential_mode == profile.credential_mode
        && previous.protocol == profile.protocol
        && previous.context_window == profile.context_window
        && previous.builtin == profile.builtin
    {
        return Ok(());
    }
    profile.updated_at = Utc::now().to_rfc3339();
    file.profiles.insert(profile.id.clone(), profile);
    save_profile_file(path, &file)
}

pub fn rollback_profile(path: &Path, update: ProfileRollback) {
    if !update.changed {
        return;
    }
    let Ok(mut file) = load_profile_file(path) else {
        return;
    };
    if let Some(previous) = update.previous {
        file.profiles.insert(update.profile_id, previous);
    } else {
        file.profiles.remove(&update.profile_id);
    }
    let _ = save_profile_file(path, &file);
}

pub fn enrich_provider_snapshot_with_profiles(
    snapshot: &mut ProviderSnapshot,
    path: &Path,
) -> Result<(), String> {
    let file = load_profile_file(path)?;
    let mut profiles = file.profiles.into_values().collect::<Vec<_>>();
    if !profiles
        .iter()
        .any(|profile| profile.provider_id == snapshot.active_provider)
        && let Some(profile) = snapshot_profile(snapshot, None)
    {
        profiles.push(profile);
    }
    for profile in &mut profiles {
        profile.credential_available = match profile.credential_mode.as_str() {
            "builtin" | "none" => true,
            "keychain" => false,
            "file" => file_credential_exists(&profile.provider_id),
            "environment" => profile
                .env_key
                .as_deref()
                .is_some_and(|key| env::var_os(key).is_some()),
            _ => false,
        };
    }
    profiles.sort_by(|left, right| {
        right
            .provider_id
            .eq(&snapshot.active_provider)
            .cmp(&left.provider_id.eq(&snapshot.active_provider))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    snapshot.profiles = profiles;
    Ok(())
}

pub fn probe_provider(request: &ProviderApplyRequest) -> Result<ProviderTestResult, String> {
    let provider_id = validate_identifier(&request.provider_id, "Provider ID")?;
    let model = validate_model(&request.model)?;
    let protocol = provider_protocol(request)?;
    let definition = if is_builtin(&provider_id) {
        let (_, name, default_base_url) = BUILTIN_PROVIDERS
            .iter()
            .find(|(id, _, _)| *id == provider_id)
            .ok_or_else(|| format!("无法识别内置 Provider：{provider_id}"))?;
        let base_url = request
            .base_url
            .as_deref()
            .map(validate_base_url)
            .transpose()?
            .or_else(|| default_base_url.map(ToOwned::to_owned))
            .ok_or_else(|| {
                format!(
                    "{name} 的凭据由 Codex 内部管理，无法从外部发送探测请求。请改用 Codex 官方模型目录验证。"
                )
            })?;
        ProviderDefinition {
            id: provider_id.clone(),
            name: (*name).to_string(),
            base_url: Some(base_url),
            env_key: request
                .env_key
                .as_deref()
                .map(|value| validate_identifier(value, "环境变量名"))
                .transpose()?,
            env_available: true,
            credential_source: credential_mode(request)?.to_string(),
            credential_available: true,
            credential_path: None,
            auth_command: None,
            auth_args: Vec::new(),
            wire_api: "responses".to_string(),
            protocol: "responses".to_string(),
            context_window: None,
            builtin: true,
            compatibility: "responses".to_string(),
        }
    } else {
        custom_definition(request)?
    };

    let base_url = if protocol == "chat_completions" {
        request
            .base_url
            .as_deref()
            .ok_or_else(|| "没有可探测的 Chat Completions 地址。".to_string())
            .and_then(|value| validate_provider_url(value, true))?
    } else {
        definition
            .base_url
            .clone()
            .ok_or_else(|| "没有可探测的 Responses API 地址。".to_string())?
    };
    let endpoint = if protocol == "chat_completions" {
        let base_url = base_url.trim_end_matches('/');
        if base_url.ends_with("/chat/completions") {
            base_url.to_string()
        } else {
            format!("{base_url}/chat/completions")
        }
    } else {
        format!("{}/responses", base_url.trim_end_matches('/'))
    };
    let credential = if let Some(api_key) = request
        .api_key
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        Some(validate_api_key(api_key.clone())?)
    } else if definition.credential_source == "keychain" {
        return Err(
            "这套配置仍使用旧钥匙串认证。请在“模型接入”中重新填写并保存 API Key。".to_string(),
        );
    } else if definition.credential_source == "file" {
        Some(read_file_credential(&provider_id)?)
    } else {
        definition
            .env_key
            .as_deref()
            .map(|key| {
                env::var(key)
                    .map_err(|_| format!("当前应用进程没有检测到环境变量 {key}。"))
                    .and_then(|value| {
                        if value.is_empty() || value.contains(['\r', '\n', '\0']) {
                            Err(format!("环境变量 {key} 为空或包含无效字符。"))
                        } else {
                            Ok(value)
                        }
                    })
            })
            .transpose()?
    };
    let request_body = if protocol == "chat_completions" {
        json!({
            "model": model,
            "messages": [{"role": "user", "content": "Return only OK."}],
            "max_tokens": 16,
            "stream": false
        })
    } else {
        json!({
            "model": model,
            "input": "Return only OK.",
            "max_output_tokens": 16,
            "store": false,
            "stream": false
        })
    };
    let body = serde_json::to_string(&request_body)
        .map_err(|error| format!("无法生成探测请求：{error}"))?;
    let marker = "\n__CODEX_XRAY_HTTP_STATUS__:";
    let started = Instant::now();
    let mut command = Command::new("curl");
    command
        .arg("--silent")
        .arg("--show-error")
        .arg("--max-time")
        .arg("20")
        .arg("--request")
        .arg("POST")
        .arg("--url")
        .arg(&endpoint)
        .arg("--header")
        .arg("@-")
        .arg("--data-binary")
        .arg(body)
        .arg("--write-out")
        .arg(format!("{marker}%{{http_code}}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动系统 curl 完成连接测试：{error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "Content-Type: application/json")
            .map_err(|error| format!("无法写入探测请求头：{error}"))?;
        if let Some(credential) = credential.as_deref() {
            writeln!(stdin, "Authorization: Bearer {credential}")
                .map_err(|error| format!("无法写入探测鉴权头：{error}"))?;
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Provider 连接测试异常：{error}"))?;
    let latency_ms = started.elapsed().as_millis();
    let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if let Some(credential) = credential.as_deref() {
        stdout = stdout.replace(credential, "[REDACTED]");
    }
    if !output.status.success() {
        return Ok(ProviderTestResult {
            success: false,
            check_kind: if protocol == "chat_completions" {
                "chat_completions_request".to_string()
            } else {
                "responses_request".to_string()
            },
            provider_id,
            model: request.model.trim().to_string(),
            endpoint: Some(endpoint),
            latency_ms,
            http_status: None,
            message: if stderr.is_empty() {
                format!("curl 退出状态 {}", output.status)
            } else {
                truncate_message(&stderr)
            },
            capabilities: provider_test_capabilities(protocol, request.context_window, false),
        });
    }
    let (response, status) = stdout
        .rsplit_once(marker)
        .ok_or_else(|| "Provider 返回中缺少 HTTP 状态。".to_string())?;
    let http_status = status
        .trim()
        .parse::<u16>()
        .map_err(|_| format!("Provider 返回了无效 HTTP 状态：{status}"))?;
    let success = (200..300).contains(&http_status);
    let message = if success {
        if protocol == "chat_completions" {
            "真实 /chat/completions 请求成功；保存后 Codex 将通过本地兼容桥使用它。".to_string()
        } else {
            "真实 /responses 请求成功；模型、地址和凭据已通过本次探测。".to_string()
        }
    } else {
        response_error_message(response)
    };
    Ok(ProviderTestResult {
        success,
        check_kind: if protocol == "chat_completions" {
            "chat_completions_request".to_string()
        } else {
            "responses_request".to_string()
        },
        provider_id,
        model: request.model.trim().to_string(),
        endpoint: Some(endpoint),
        latency_ms,
        http_status: Some(http_status),
        message,
        capabilities: provider_test_capabilities(protocol, request.context_window, success),
    })
}

pub fn provider_test_capabilities(
    protocol: &str,
    context_window: u64,
    generation_verified: bool,
) -> Vec<ProviderCapability> {
    let generation_status = if generation_verified {
        "verified"
    } else {
        "failed"
    };
    if protocol == "chat_completions" {
        return vec![
            ProviderCapability {
                capability: "text_generation".to_string(),
                status: generation_status.to_string(),
                value: None,
            },
            ProviderCapability {
                capability: "function_tools".to_string(),
                status: "bridge_supported".to_string(),
                value: None,
            },
            ProviderCapability {
                capability: "image_input".to_string(),
                status: "limited".to_string(),
                value: None,
            },
            ProviderCapability {
                capability: "builtin_tools".to_string(),
                status: "limited".to_string(),
                value: None,
            },
            ProviderCapability {
                capability: "reasoning_compaction".to_string(),
                status: "limited".to_string(),
                value: None,
            },
            ProviderCapability {
                capability: "context_window".to_string(),
                status: "configured".to_string(),
                value: Some(context_window.to_string()),
            },
        ];
    }

    vec![
        ProviderCapability {
            capability: "text_generation".to_string(),
            status: generation_status.to_string(),
            value: None,
        },
        ProviderCapability {
            capability: "function_tools".to_string(),
            status: "unverified".to_string(),
            value: None,
        },
        ProviderCapability {
            capability: "image_input".to_string(),
            status: "unverified".to_string(),
            value: None,
        },
        ProviderCapability {
            capability: "builtin_tools".to_string(),
            status: "unverified".to_string(),
            value: None,
        },
        ProviderCapability {
            capability: "reasoning_compaction".to_string(),
            status: "unverified".to_string(),
            value: None,
        },
        ProviderCapability {
            capability: "context_window".to_string(),
            status: "unverified".to_string(),
            value: None,
        },
    ]
}

pub fn codex_catalog_capabilities(found: bool) -> Vec<ProviderCapability> {
    vec![
        ProviderCapability {
            capability: "model_catalog".to_string(),
            status: if found { "verified" } else { "failed" }.to_string(),
            value: None,
        },
        ProviderCapability {
            capability: "text_generation".to_string(),
            status: "unverified".to_string(),
            value: None,
        },
    ]
}

fn response_error_message(response: &str) -> String {
    let trimmed = response.trim();
    let parsed = serde_json::from_str::<Value>(trimmed).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
        })
        .unwrap_or(trimmed);
    if message.is_empty() {
        "Provider 拒绝了探测请求，但没有返回错误正文。".to_string()
    } else {
        truncate_message(message)
    }
}

fn truncate_message(value: &str) -> String {
    const MAX_CHARS: usize = 2_000;
    let mut result = value.chars().take(MAX_CHARS).collect::<String>();
    if value.chars().count() > MAX_CHARS {
        result.push('…');
    }
    result
}

pub fn restore_point(snapshot: &ProviderSnapshot) -> ProviderRestorePoint {
    ProviderRestorePoint {
        provider_id: snapshot.active_provider.clone(),
        model: snapshot.active_model.clone(),
        definition: snapshot
            .providers
            .iter()
            .find(|item| item.id == snapshot.active_provider && !item.builtin)
            .cloned(),
    }
}

pub fn restore_edits(point: &ProviderRestorePoint) -> Result<Vec<Value>, String> {
    let provider_id = validate_identifier(&point.provider_id, "Provider ID")?;
    let mut edits = vec![edit("model_provider", json!(provider_id), "replace")];
    if let Some(model) = &point.model {
        edits.push(edit("model", json!(validate_model(model)?), "replace"));
    }
    if let Some(definition) = &point.definition {
        edits.extend(definition_edits(definition));
    }
    Ok(edits)
}

pub struct CredentialRollback {
    provider_id: String,
    mode: String,
    previous: Option<String>,
    changed: bool,
}

pub fn persist_request_credential(
    request: &ProviderApplyRequest,
) -> Result<CredentialRollback, String> {
    let provider_id = validate_identifier(&request.provider_id, "Provider ID")?;
    let mode = credential_mode(request)?;
    if is_builtin(&provider_id) || mode != "file" {
        return Ok(CredentialRollback {
            provider_id,
            mode: mode.to_string(),
            previous: None,
            changed: false,
        });
    }
    if let Some(api_key) = request
        .api_key
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        let previous = read_file_credential(&provider_id).ok();
        save_file_credential(&provider_id, api_key)?;
        return Ok(CredentialRollback {
            provider_id,
            mode: mode.to_string(),
            previous,
            changed: true,
        });
    }
    let available = file_credential_exists(&provider_id);
    if !available {
        return Err("请先填写 API Key；保存后它不会写入 Codex 配置。".to_string());
    }
    Ok(CredentialRollback {
        provider_id,
        mode: mode.to_string(),
        previous: None,
        changed: false,
    })
}

pub fn rollback_credential(update: CredentialRollback) {
    if !update.changed {
        return;
    }
    if let Some(previous) = update.previous {
        let _ = save_file_credential(&update.provider_id, &previous);
    } else if update.mode == "file" {
        let _ = delete_file_credential(&update.provider_id);
    }
}

pub fn credential_helper_exit_code() -> Option<i32> {
    let mut args = env::args_os();
    let _executable = args.next();
    let flag = args.next()?;
    if flag == CREDENTIAL_HELPER_ARG {
        eprintln!(
            "Codex X-Ray credential helper: keychain authentication was removed; save the API key again in Model access"
        );
        return Some(2);
    }
    if flag != FILE_CREDENTIAL_HELPER_ARG {
        return None;
    }
    let Some(provider_id) = args.next().and_then(|value| value.into_string().ok()) else {
        eprintln!("Codex X-Ray credential helper: missing Provider ID");
        return Some(2);
    };
    if args.next().is_some() {
        eprintln!("Codex X-Ray credential helper: unexpected arguments");
        return Some(2);
    }
    let credential = validate_identifier(&provider_id, "Provider ID")
        .and_then(|provider_id| read_file_credential(&provider_id));
    match credential {
        Ok(secret) => {
            if io::stdout()
                .write_all(secret.as_bytes())
                .and_then(|_| io::stdout().flush())
                .is_ok()
            {
                Some(0)
            } else {
                Some(1)
            }
        }
        Err(error) => {
            eprintln!("Codex X-Ray credential helper: {error}");
            Some(1)
        }
    }
}

pub fn save_restore_point(path: &Path, point: &ProviderRestorePoint) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 Provider 恢复点目录：{error}"))?;
    }
    let temporary = path.with_extension("tmp");
    let content = serde_json::to_vec_pretty(point)
        .map_err(|error| format!("无法序列化 Provider 恢复点：{error}"))?;
    fs::write(&temporary, content)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| format!("无法保存 Provider 恢复点：{error}"))
}

pub fn read_restore_point(path: &Path) -> Result<ProviderRestorePoint, String> {
    let content = fs::read(path).map_err(|error| format!("无法读取 Provider 恢复点：{error}"))?;
    serde_json::from_slice(&content).map_err(|error| format!("Provider 恢复点无法解析：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_glm_chat_completion_endpoint() {
        let request = ProviderApplyRequest {
            profile_name: Some("GLM".to_string()),
            provider_id: "glm".to_string(),
            model: "glm-5.2".to_string(),
            name: Some("GLM".to_string()),
            base_url: Some("https://open.bigmodel.cn/api/coding/paas/v4".to_string()),
            env_key: Some("ZHIPUAI_API_KEY".to_string()),
            credential_mode: Some("environment".to_string()),
            api_key: None,
            protocol: "responses".to_string(),
            context_window: 128_000,
            expected_version: None,
        };
        let error = build_apply_edits(&request).expect_err("direct GLM should be rejected");
        assert!(error.contains("Responses API"));
    }

    #[test]
    fn accepts_qianfan_responses_for_glm() {
        let request = ProviderApplyRequest {
            profile_name: Some("Qianfan GLM".to_string()),
            provider_id: "qianfan-glm".to_string(),
            model: "glm-5".to_string(),
            name: Some("Baidu Qianfan · GLM".to_string()),
            base_url: Some("https://qianfan.baidubce.com/v2".to_string()),
            env_key: Some("QIANFAN_API_KEY".to_string()),
            credential_mode: Some("environment".to_string()),
            api_key: None,
            protocol: "responses".to_string(),
            context_window: 128_000,
            expected_version: None,
        };
        let (edits, definition) =
            build_apply_edits(&request).expect("Qianfan exposes Responses API");
        assert_eq!(edits.len(), 3);
        assert_eq!(
            definition.and_then(|item| item.base_url),
            Some("https://qianfan.baidubce.com/v2".to_string())
        );
    }

    #[test]
    fn accepts_native_domestic_responses_endpoints() {
        for base_url in [
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "https://ark.cn-beijing.volces.com/api/v3",
            "https://api.minimaxi.com/v1",
            "https://api.xiaomimimo.com/v1",
        ] {
            assert_eq!(
                validate_base_url(base_url).expect("Responses endpoint"),
                base_url
            );
        }
    }

    #[test]
    fn builds_safe_responses_provider_edits() {
        let request = ProviderApplyRequest {
            profile_name: Some("GLM Gateway".to_string()),
            provider_id: "glm-gateway".to_string(),
            model: "glm-5.2".to_string(),
            name: Some("GLM via Responses gateway".to_string()),
            base_url: Some("https://gateway.example.com/v1".to_string()),
            env_key: Some("ZHIPUAI_API_KEY".to_string()),
            credential_mode: Some("environment".to_string()),
            api_key: None,
            protocol: "responses".to_string(),
            context_window: 128_000,
            expected_version: Some("v1".to_string()),
        };
        let (edits, definition) = build_apply_edits(&request).expect("valid provider");
        assert_eq!(edits.len(), 3);
        assert_eq!(
            definition.and_then(|item| item.base_url),
            Some("https://gateway.example.com/v1".to_string())
        );
        assert!(!edits.iter().any(|item| item.to_string().contains("secret")));
    }

    #[test]
    fn writes_file_auth_without_serializing_api_key() {
        let request = ProviderApplyRequest {
            profile_name: Some("Qwen file".to_string()),
            provider_id: "dashscope-file".to_string(),
            model: "qwen3-coder-plus".to_string(),
            name: Some("Alibaba Model Studio".to_string()),
            base_url: Some("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()),
            env_key: None,
            credential_mode: Some("file".to_string()),
            api_key: Some("test-secret-value".to_string()),
            protocol: "responses".to_string(),
            context_window: 128_000,
            expected_version: None,
        };
        let (edits, definition) = build_apply_edits(&request).expect("file provider");
        let serialized = serde_json::to_string(&edits).expect("serialize edits");
        assert!(!serialized.contains("test-secret-value"));
        assert!(serialized.contains(FILE_CREDENTIAL_HELPER_ARG));
        assert!(definition.is_some_and(|provider| {
            provider.credential_source == "file"
                && provider
                    .credential_path
                    .is_some_and(|path| path.ends_with("dashscope-file.key"))
        }));
    }

    #[test]
    fn provider_profile_never_serializes_api_key() {
        let request = ProviderApplyRequest {
            profile_name: Some("Kimi Work".to_string()),
            provider_id: "kimi-work".to_string(),
            model: "kimi-k2.6".to_string(),
            name: Some("Kimi".to_string()),
            base_url: Some("https://api.moonshot.cn/v1".to_string()),
            env_key: None,
            credential_mode: Some("file".to_string()),
            api_key: Some("test-secret-value".to_string()),
            protocol: "chat_completions".to_string(),
            context_window: 256_000,
            expected_version: None,
        };
        let profile = request_profile(&request).expect("profile");
        let serialized = serde_json::to_string(&profile).expect("serialize profile");
        assert_eq!(profile.provider_id, "kimi-work");
        assert_eq!(profile.protocol, "chat_completions");
        assert!(!serialized.contains("test-secret-value"));
        assert!(!serialized.contains("api_key"));
    }

    #[test]
    fn routes_chat_completions_through_local_bridge() {
        let request = ProviderApplyRequest {
            profile_name: Some("DeepSeek".to_string()),
            provider_id: "deepseek-direct".to_string(),
            model: "deepseek-chat".to_string(),
            name: Some("DeepSeek Direct".to_string()),
            base_url: Some("https://api.deepseek.com".to_string()),
            env_key: None,
            credential_mode: Some("file".to_string()),
            api_key: Some("test-secret-value".to_string()),
            protocol: "chat_completions".to_string(),
            context_window: 128_000,
            expected_version: None,
        };
        let (edits, definition) = build_apply_edits(&request).expect("chat bridge provider");
        let serialized = serde_json::to_string(&edits).expect("serialize edits");
        assert!(serialized.contains("http://127.0.0.1:32198/v1/chat/deepseek-direct"));
        assert!(!serialized.contains("https://api.deepseek.com"));
        assert!(definition.is_some_and(|provider| provider.compatibility == "chat_bridge"));
    }

    #[test]
    fn reports_chat_bridge_limits_without_claiming_the_probe_verified_them() {
        let capabilities = provider_test_capabilities("chat_completions", 128_000, true);
        let status = |name: &str| {
            capabilities
                .iter()
                .find(|item| item.capability == name)
                .map(|item| item.status.as_str())
        };
        assert_eq!(status("text_generation"), Some("verified"));
        assert_eq!(status("function_tools"), Some("bridge_supported"));
        assert_eq!(status("image_input"), Some("limited"));
        assert_eq!(status("builtin_tools"), Some("limited"));
        assert_eq!(status("reasoning_compaction"), Some("limited"));
        assert_eq!(
            capabilities
                .iter()
                .find(|item| item.capability == "context_window")
                .and_then(|item| item.value.as_deref()),
            Some("128000")
        );
    }

    #[test]
    fn leaves_native_responses_features_unverified_after_a_text_probe() {
        let capabilities = provider_test_capabilities("responses", 128_000, true);
        assert_eq!(
            capabilities
                .iter()
                .find(|item| item.capability == "text_generation")
                .map(|item| item.status.as_str()),
            Some("verified")
        );
        assert!(
            capabilities
                .iter()
                .filter(|item| {
                    item.capability != "text_generation" && item.status == "unverified"
                })
                .count()
                >= 5
        );
    }
}
