use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use url::Url;

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
    pub wire_api: String,
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
    pub restore_available: bool,
    pub warnings: Vec<String>,
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
    pub provider_id: String,
    pub model: String,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub env_key: Option<String>,
    pub expected_version: Option<String>,
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
    let wire_api = string_field(value, "wire_api").unwrap_or_else(|| "responses".to_string());
    ProviderDefinition {
        id: id.to_string(),
        name,
        compatibility: compatibility_for(base_url.as_deref(), &wire_api),
        base_url,
        env_key,
        env_available,
        wire_api,
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
                wire_api: "responses".to_string(),
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
        if active.env_key.is_some() && !active.env_available {
            warnings.push(format!(
                "当前进程没有检测到环境变量 {}。",
                active.env_key.as_deref().unwrap_or_default()
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
        restore_available: restore_path.is_file(),
        warnings,
    })
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

fn validate_base_url(value: &str) -> Result<String, String> {
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
    if is_known_chat_only_url(value) {
        return Err(
            "这个厂商直连地址目前只公布了 Chat Completions，不能直接用于只接受 Responses API 的 Codex。请选择支持 Responses 的云平台入口，或填写 Responses 适配网关。"
                .to_string(),
        );
    }
    Ok(value.to_string())
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
    let base_url = request
        .base_url
        .as_deref()
        .ok_or_else(|| "自定义 Provider 必须填写 Responses API Base URL。".to_string())
        .and_then(validate_base_url)?;
    let env_key = request
        .env_key
        .as_deref()
        .map(|value| validate_identifier(value, "环境变量名"))
        .transpose()?;
    Ok(ProviderDefinition {
        id,
        name,
        base_url: Some(base_url),
        env_available: env_key
            .as_deref()
            .is_some_and(|key| env::var_os(key).is_some()),
        env_key,
        wire_api: "responses".to_string(),
        builtin: false,
        compatibility: "responses".to_string(),
    })
}

fn edit(key_path: impl Into<String>, value: Value) -> Value {
    json!({
        "keyPath": key_path.into(),
        "value": value,
        "mergeStrategy": "upsert"
    })
}

fn definition_edits(definition: &ProviderDefinition) -> Vec<Value> {
    let prefix = format!("model_providers.{}", definition.id);
    let mut edits = vec![
        edit(format!("{prefix}.name"), json!(definition.name)),
        edit(format!("{prefix}.wire_api"), json!("responses")),
    ];
    if let Some(base_url) = &definition.base_url {
        edits.push(edit(format!("{prefix}.base_url"), json!(base_url)));
    }
    if let Some(env_key) = &definition.env_key {
        edits.push(edit(format!("{prefix}.env_key"), json!(env_key)));
    }
    edits
}

pub fn build_apply_edits(
    request: &ProviderApplyRequest,
) -> Result<(Vec<Value>, Option<ProviderDefinition>), String> {
    let provider_id = validate_identifier(&request.provider_id, "Provider ID")?;
    let model = validate_model(&request.model)?;
    let definition = if is_builtin(&provider_id) {
        None
    } else {
        Some(custom_definition(request)?)
    };
    let mut edits = vec![
        edit("model_provider", json!(provider_id)),
        edit("model", json!(model)),
    ];
    if let Some(definition) = &definition {
        edits.extend(definition_edits(definition));
    }
    Ok((edits, definition))
}

pub fn probe_provider(request: &ProviderApplyRequest) -> Result<ProviderTestResult, String> {
    let provider_id = validate_identifier(&request.provider_id, "Provider ID")?;
    let model = validate_model(&request.model)?;
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
            wire_api: "responses".to_string(),
            builtin: true,
            compatibility: "responses".to_string(),
        }
    } else {
        custom_definition(request)?
    };

    let base_url = definition
        .base_url
        .as_deref()
        .ok_or_else(|| "没有可探测的 Responses API 地址。".to_string())?;
    let endpoint = format!("{}/responses", base_url.trim_end_matches('/'));
    let credential = definition
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
        .transpose()?;
    let body = serde_json::to_string(&json!({
        "model": model,
        "input": "Return only OK.",
        "max_output_tokens": 16,
        "store": false,
        "stream": false
    }))
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
            check_kind: "responses_request".to_string(),
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
        "真实 /responses 请求成功；模型、地址和凭据已通过本次探测。".to_string()
    } else {
        response_error_message(response)
    };
    Ok(ProviderTestResult {
        success,
        check_kind: "responses_request".to_string(),
        provider_id,
        model: request.model.trim().to_string(),
        endpoint: Some(endpoint),
        latency_ms,
        http_status: Some(http_status),
        message,
    })
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
    let mut edits = vec![edit("model_provider", json!(provider_id))];
    if let Some(model) = &point.model {
        edits.push(edit("model", json!(validate_model(model)?)));
    }
    if let Some(definition) = &point.definition {
        edits.extend(definition_edits(definition));
    }
    Ok(edits)
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
            provider_id: "glm".to_string(),
            model: "glm-5.2".to_string(),
            name: Some("GLM".to_string()),
            base_url: Some("https://open.bigmodel.cn/api/coding/paas/v4".to_string()),
            env_key: Some("ZHIPUAI_API_KEY".to_string()),
            expected_version: None,
        };
        let error = build_apply_edits(&request).expect_err("direct GLM should be rejected");
        assert!(error.contains("Responses API"));
    }

    #[test]
    fn accepts_qianfan_responses_for_glm() {
        let request = ProviderApplyRequest {
            provider_id: "qianfan-glm".to_string(),
            model: "glm-5".to_string(),
            name: Some("Baidu Qianfan · GLM".to_string()),
            base_url: Some("https://qianfan.baidubce.com/v2".to_string()),
            env_key: Some("QIANFAN_API_KEY".to_string()),
            expected_version: None,
        };
        let (edits, definition) =
            build_apply_edits(&request).expect("Qianfan exposes Responses API");
        assert_eq!(edits.len(), 6);
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
            "https://api.stepfun.com/v1",
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
            provider_id: "glm-gateway".to_string(),
            model: "glm-5.2".to_string(),
            name: Some("GLM via Responses gateway".to_string()),
            base_url: Some("https://gateway.example.com/v1".to_string()),
            env_key: Some("ZHIPUAI_API_KEY".to_string()),
            expected_version: Some("v1".to_string()),
        };
        let (edits, definition) = build_apply_edits(&request).expect("valid provider");
        assert_eq!(edits.len(), 6);
        assert_eq!(
            definition.and_then(|item| item.base_url),
            Some("https://gateway.example.com/v1".to_string())
        );
        assert!(!edits.iter().any(|item| item.to_string().contains("secret")));
    }
}
