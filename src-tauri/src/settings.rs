use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SETTING_KEYS: &[&str] = &[
    "model_reasoning_effort",
    "plan_mode_reasoning_effort",
    "model_reasoning_summary",
    "model_verbosity",
    "personality",
    "approval_policy",
    "approvals_reviewer",
    "sandbox_mode",
    "sandbox_workspace_write.network_access",
    "web_search",
    "history.persistence",
    "model_auto_compact_token_limit",
    "model_auto_compact_token_limit_scope",
    "features.memories",
    "memories.use_memories",
    "memories.generate_memories",
    "memories.disable_on_external_context",
    "features.multi_agent",
    "features.goals",
    "features.hooks",
    "features.unified_exec",
    "features.fast_mode",
    "features.apps",
    "hide_agent_reasoning",
    "show_raw_agent_reasoning",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodexSettings {
    pub model_reasoning_effort: Option<String>,
    pub plan_mode_reasoning_effort: Option<String>,
    pub model_reasoning_summary: Option<String>,
    pub model_verbosity: Option<String>,
    pub personality: Option<String>,
    pub approval_policy: String,
    pub approvals_reviewer: String,
    pub sandbox_mode: String,
    pub workspace_network_access: bool,
    pub web_search: String,
    pub history_persistence: String,
    pub auto_compact_token_limit: Option<u64>,
    pub auto_compact_scope: String,
    pub memories_enabled: bool,
    pub memories_use: bool,
    pub memories_generate: bool,
    pub memories_disable_on_external_context: bool,
    pub multi_agent_enabled: bool,
    pub goals_enabled: bool,
    pub hooks_enabled: bool,
    pub unified_exec_enabled: bool,
    pub fast_mode_enabled: bool,
    pub apps_enabled: bool,
    pub hide_agent_reasoning: bool,
    pub show_raw_agent_reasoning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSnapshot {
    pub fetched_at: String,
    pub config_path: String,
    pub version: Option<String>,
    pub settings: CodexSettings,
    pub restore_available: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsApplyRequest {
    pub settings: CodexSettings,
    pub expected_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsRestorePoint {
    settings: CodexSettings,
    changed_keys: Vec<String>,
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

fn value_at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn string_at(value: &Value, path: &str) -> Option<String> {
    value_at(value, path)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn bool_at(value: &Value, path: &str, fallback: bool) -> bool {
    value_at(value, path)
        .and_then(Value::as_bool)
        .unwrap_or(fallback)
}

fn u64_at(value: &Value, path: &str) -> Option<u64> {
    value_at(value, path).and_then(Value::as_u64)
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

fn parse_settings(config: &Value) -> CodexSettings {
    let approval_policy = match value_at(config, "approval_policy") {
        Some(Value::Object(_)) => "granular".to_string(),
        _ => string_at(config, "approval_policy").unwrap_or_else(|| "untrusted".to_string()),
    };

    CodexSettings {
        model_reasoning_effort: string_at(config, "model_reasoning_effort"),
        plan_mode_reasoning_effort: string_at(config, "plan_mode_reasoning_effort"),
        model_reasoning_summary: string_at(config, "model_reasoning_summary"),
        model_verbosity: string_at(config, "model_verbosity"),
        personality: string_at(config, "personality"),
        approval_policy,
        approvals_reviewer: string_at(config, "approvals_reviewer")
            .unwrap_or_else(|| "user".to_string()),
        sandbox_mode: string_at(config, "sandbox_mode").unwrap_or_else(|| "read-only".to_string()),
        workspace_network_access: bool_at(config, "sandbox_workspace_write.network_access", false),
        web_search: string_at(config, "web_search").unwrap_or_else(|| "cached".to_string()),
        history_persistence: string_at(config, "history.persistence")
            .unwrap_or_else(|| "save-all".to_string()),
        auto_compact_token_limit: u64_at(config, "model_auto_compact_token_limit"),
        auto_compact_scope: string_at(config, "model_auto_compact_token_limit_scope")
            .unwrap_or_else(|| "total".to_string()),
        memories_enabled: bool_at(config, "features.memories", false),
        memories_use: bool_at(config, "memories.use_memories", true),
        memories_generate: bool_at(config, "memories.generate_memories", true),
        memories_disable_on_external_context: bool_at(
            config,
            "memories.disable_on_external_context",
            false,
        ),
        multi_agent_enabled: bool_at(config, "features.multi_agent", true),
        goals_enabled: bool_at(config, "features.goals", true),
        hooks_enabled: bool_at(config, "features.hooks", true),
        unified_exec_enabled: bool_at(config, "features.unified_exec", true),
        fast_mode_enabled: bool_at(config, "features.fast_mode", true),
        apps_enabled: bool_at(config, "features.apps", true),
        hide_agent_reasoning: bool_at(config, "hide_agent_reasoning", false),
        show_raw_agent_reasoning: bool_at(config, "show_raw_agent_reasoning", false),
    }
}

pub fn build_settings_snapshot(
    raw: &Value,
    restore_path: &Path,
) -> Result<SettingsSnapshot, String> {
    let config = raw
        .get("config")
        .ok_or_else(|| "config/read 缺少 config 字段".to_string())?;
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
    let settings = parse_settings(config);
    let mut warnings = Vec::new();
    if settings.approval_policy == "granular" {
        warnings.push(
            "当前使用精细审批策略；普通模式只展示状态，切换到预设后会替换这组规则。".to_string(),
        );
    }
    if settings.history_persistence == "none" {
        warnings
            .push("会话历史保存已关闭，Codex 庖丁将无法持续生成完整用量和执行分析。".to_string());
    }

    Ok(SettingsSnapshot {
        fetched_at: Utc::now().to_rfc3339(),
        config_path,
        version,
        settings,
        restore_available: restore_path.is_file(),
        warnings,
    })
}

fn validate_option(value: Option<&str>, allowed: &[&str], label: &str) -> Result<(), String> {
    if value.is_some_and(|value| !allowed.contains(&value)) {
        return Err(format!("{label} 不是 Codex 支持的值。"));
    }
    Ok(())
}

fn validate_settings(settings: &CodexSettings) -> Result<(), String> {
    validate_option(
        settings.model_reasoning_effort.as_deref(),
        &["minimal", "low", "medium", "high", "xhigh"],
        "推理强度",
    )?;
    validate_option(
        settings.plan_mode_reasoning_effort.as_deref(),
        &["none", "minimal", "low", "medium", "high", "xhigh"],
        "计划模式推理强度",
    )?;
    validate_option(
        settings.model_reasoning_summary.as_deref(),
        &["auto", "concise", "detailed", "none"],
        "推理摘要",
    )?;
    validate_option(
        settings.model_verbosity.as_deref(),
        &["low", "medium", "high"],
        "回答详细度",
    )?;
    validate_option(
        settings.personality.as_deref(),
        &["none", "friendly", "pragmatic"],
        "沟通风格",
    )?;
    if !["untrusted", "on-request", "never", "granular"]
        .contains(&settings.approval_policy.as_str())
    {
        return Err("审批方式不是 Codex 支持的值。".to_string());
    }
    if !["user", "auto_review"].contains(&settings.approvals_reviewer.as_str()) {
        return Err("审批人不是 Codex 支持的值。".to_string());
    }
    if !["read-only", "workspace-write", "danger-full-access"]
        .contains(&settings.sandbox_mode.as_str())
    {
        return Err("文件权限不是 Codex 支持的值。".to_string());
    }
    if !["disabled", "cached", "indexed", "live"].contains(&settings.web_search.as_str()) {
        return Err("联网搜索模式不是 Codex 支持的值。".to_string());
    }
    if !["save-all", "none"].contains(&settings.history_persistence.as_str()) {
        return Err("历史保存模式不是 Codex 支持的值。".to_string());
    }
    if !["total", "body_after_prefix"].contains(&settings.auto_compact_scope.as_str()) {
        return Err("自动压缩计算范围不是 Codex 支持的值。".to_string());
    }
    if settings
        .auto_compact_token_limit
        .is_some_and(|value| !(16_000..=10_000_000).contains(&value))
    {
        return Err("自动压缩阈值必须在 16,000 到 10,000,000 Token 之间。".to_string());
    }
    if settings.hide_agent_reasoning && settings.show_raw_agent_reasoning {
        return Err("不能同时隐藏推理过程并显示原始推理。".to_string());
    }
    Ok(())
}

fn edit(key_path: &str, value: Value) -> Value {
    json!({
        "keyPath": key_path,
        "value": value,
        "mergeStrategy": "replace"
    })
}

fn setting_value(settings: &CodexSettings, key: &str) -> Result<Value, String> {
    let value = match key {
        "model_reasoning_effort" => json!(settings.model_reasoning_effort),
        "plan_mode_reasoning_effort" => json!(settings.plan_mode_reasoning_effort),
        "model_reasoning_summary" => json!(settings.model_reasoning_summary),
        "model_verbosity" => json!(settings.model_verbosity),
        "personality" => json!(settings.personality),
        "approval_policy" => {
            if settings.approval_policy == "granular" {
                return Err("精细审批策略需要在高级配置中逐项编辑。".to_string());
            }
            json!(settings.approval_policy)
        }
        "approvals_reviewer" => json!(settings.approvals_reviewer),
        "sandbox_mode" => json!(settings.sandbox_mode),
        "sandbox_workspace_write.network_access" => json!(settings.workspace_network_access),
        "web_search" => json!(settings.web_search),
        "history.persistence" => json!(settings.history_persistence),
        "model_auto_compact_token_limit" => json!(settings.auto_compact_token_limit),
        "model_auto_compact_token_limit_scope" => json!(settings.auto_compact_scope),
        "features.memories" => json!(settings.memories_enabled),
        "memories.use_memories" => json!(settings.memories_use),
        "memories.generate_memories" => json!(settings.memories_generate),
        "memories.disable_on_external_context" => {
            json!(settings.memories_disable_on_external_context)
        }
        "features.multi_agent" => json!(settings.multi_agent_enabled),
        "features.goals" => json!(settings.goals_enabled),
        "features.hooks" => json!(settings.hooks_enabled),
        "features.unified_exec" => json!(settings.unified_exec_enabled),
        "features.fast_mode" => json!(settings.fast_mode_enabled),
        "features.apps" => json!(settings.apps_enabled),
        "hide_agent_reasoning" => json!(settings.hide_agent_reasoning),
        "show_raw_agent_reasoning" => json!(settings.show_raw_agent_reasoning),
        _ => return Err(format!("未知配置字段：{key}")),
    };
    Ok(value)
}

pub fn build_settings_edits(
    before: &CodexSettings,
    after: &CodexSettings,
) -> Result<(Vec<Value>, Vec<String>), String> {
    validate_settings(after)?;
    let mut edits = Vec::new();
    let mut changed_keys = Vec::new();
    for key in SETTING_KEYS {
        let before_value = setting_value(before, key);
        let after_value = setting_value(after, key);
        let (Ok(before_value), Ok(after_value)) = (before_value, after_value) else {
            if key == &"approval_policy" && before.approval_policy == after.approval_policy {
                continue;
            }
            return Err("精细审批策略只能保留或替换为普通预设。".to_string());
        };
        if before_value != after_value {
            edits.push(edit(key, after_value));
            changed_keys.push((*key).to_string());
        }
    }
    Ok((edits, changed_keys))
}

pub fn restore_point(settings: &CodexSettings, changed_keys: Vec<String>) -> SettingsRestorePoint {
    SettingsRestorePoint {
        settings: settings.clone(),
        changed_keys,
    }
}

pub fn restore_edits(point: &SettingsRestorePoint) -> Result<Vec<Value>, String> {
    validate_settings(&point.settings)?;
    point
        .changed_keys
        .iter()
        .map(|key| setting_value(&point.settings, key).map(|value| edit(key, value)))
        .collect()
}

pub fn save_restore_point(path: &Path, point: &SettingsRestorePoint) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建设置恢复点目录：{error}"))?;
    }
    let temporary = path.with_extension("tmp");
    let content = serde_json::to_vec_pretty(point)
        .map_err(|error| format!("无法序列化设置恢复点：{error}"))?;
    fs::write(&temporary, content)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| format!("无法保存设置恢复点：{error}"))
}

pub fn read_restore_point(path: &Path) -> Result<SettingsRestorePoint, String> {
    let content = fs::read(path).map_err(|error| format!("无法读取设置恢复点：{error}"))?;
    serde_json::from_slice(&content).map_err(|error| format!("设置恢复点无法解析：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_only_changed_setting_edits() {
        let raw = json!({ "config": {} });
        let before = build_settings_snapshot(&raw, Path::new("/missing"))
            .expect("settings")
            .settings;
        let mut after = before.clone();
        after.model_verbosity = Some("high".to_string());
        after.web_search = "live".to_string();
        let (edits, keys) = build_settings_edits(&before, &after).expect("valid edits");
        assert_eq!(edits.len(), 2);
        assert_eq!(keys, vec!["model_verbosity", "web_search"]);
    }

    #[test]
    fn rejects_conflicting_reasoning_display_settings() {
        let raw = json!({ "config": {} });
        let mut settings = build_settings_snapshot(&raw, Path::new("/missing"))
            .expect("settings")
            .settings;
        settings.hide_agent_reasoning = true;
        settings.show_raw_agent_reasoning = true;
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn null_value_clears_optional_override() {
        let raw = json!({
            "config": {
                "model_verbosity": "high"
            }
        });
        let before = build_settings_snapshot(&raw, Path::new("/missing"))
            .expect("settings")
            .settings;
        let mut after = before.clone();
        after.model_verbosity = None;
        let (edits, _) = build_settings_edits(&before, &after).expect("clear edit");
        assert_eq!(edits[0]["value"], Value::Null);
    }
}
