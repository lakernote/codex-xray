use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::provider::ProviderSnapshot;
use crate::settings::{CodexSettings, SettingsSnapshot};
use crate::storage::StorageHealth;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentPath {
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub item_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentMcpServer {
    pub name: String,
    pub enabled: bool,
    pub transport: String,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentProvider {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub wire_api: String,
    pub endpoint: Option<String>,
    pub credential_variable: Option<String>,
    pub credential_source: String,
    pub credential_available: bool,
    pub compatibility: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentSnapshot {
    pub fetched_at: String,
    pub codex_version: String,
    pub codex_binary: String,
    pub codex_home: String,
    pub config_path: String,
    pub sessions_path: String,
    pub xray_data_path: String,
    pub xray_sqlite_path: String,
    pub storage: StorageHealth,
    pub config_version: Option<String>,
    pub provider: EnvironmentProvider,
    pub settings: CodexSettings,
    pub mcp_servers: Vec<EnvironmentMcpServer>,
    pub extension_paths: Vec<EnvironmentPath>,
    pub warnings: Vec<String>,
}

pub struct EnvironmentRuntime<'a> {
    pub codex_version: &'a str,
    pub codex_binary: &'a Path,
    pub xray_data_path: &'a Path,
    pub xray_sqlite_path: &'a Path,
    pub storage: StorageHealth,
}

pub fn codex_home() -> PathBuf {
    env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

fn user_home() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn direct_directory_count(path: &Path) -> Option<u64> {
    let entries = fs::read_dir(path).ok()?;
    Some(
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .count() as u64,
    )
}

fn extension_path(label: &str, path: PathBuf) -> EnvironmentPath {
    EnvironmentPath {
        label: label.to_string(),
        exists: path.is_dir(),
        item_count: direct_directory_count(&path),
        path: display_path(&path),
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn safe_url_target(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?;
            let port = url
                .port()
                .map(|value| format!(":{value}"))
                .unwrap_or_default();
            Some(format!("{}://{host}{port}", url.scheme()))
        })
        .unwrap_or_else(|| value.chars().take(120).collect())
}

fn safe_command_target(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value)
        .chars()
        .take(120)
        .collect()
}

fn mcp_servers(raw: &Value) -> Vec<EnvironmentMcpServer> {
    let Some(servers) = raw
        .get("config")
        .and_then(|config| config.get("mcp_servers"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };

    let mut result = servers
        .iter()
        .map(|(name, value)| {
            let enabled = value
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let url = string_field(value, "url");
            let command = string_field(value, "command");
            let (transport, target) = if let Some(url) = url {
                ("HTTP".to_string(), Some(safe_url_target(&url)))
            } else if let Some(command) = command {
                ("STDIO".to_string(), Some(safe_command_target(&command)))
            } else {
                ("Unknown".to_string(), None)
            };
            EnvironmentMcpServer {
                name: name.clone(),
                enabled,
                transport,
                target,
            }
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| {
        right
            .enabled
            .cmp(&left.enabled)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    result
}

pub fn build_environment_snapshot(
    raw: &Value,
    provider_snapshot: &ProviderSnapshot,
    settings_snapshot: &SettingsSnapshot,
    runtime: EnvironmentRuntime<'_>,
) -> EnvironmentSnapshot {
    let EnvironmentRuntime {
        codex_version,
        codex_binary,
        xray_data_path,
        xray_sqlite_path,
        storage,
    } = runtime;
    let home = codex_home();
    let sessions_path = home.join("sessions");
    let active_provider = provider_snapshot
        .providers
        .iter()
        .find(|provider| provider.id == provider_snapshot.active_provider);
    let provider = EnvironmentProvider {
        id: provider_snapshot.active_provider.clone(),
        name: active_provider
            .map(|provider| provider.name.clone())
            .unwrap_or_else(|| provider_snapshot.active_provider.clone()),
        model: provider_snapshot.active_model.clone(),
        wire_api: active_provider
            .map(|provider| provider.wire_api.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        endpoint: active_provider.and_then(|provider| provider.base_url.clone()),
        credential_variable: active_provider.and_then(|provider| provider.env_key.clone()),
        credential_source: active_provider
            .map(|provider| provider.credential_source.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        credential_available: active_provider
            .map(|provider| provider.credential_available)
            .unwrap_or(false),
        compatibility: active_provider
            .map(|provider| provider.compatibility.clone())
            .unwrap_or_else(|| "unknown".to_string()),
    };

    let mut warnings = Vec::new();
    if !home.is_dir() {
        warnings
            .push("CODEX_HOME does not exist, so local sessions cannot be inspected.".to_string());
    } else if !sessions_path.is_dir() {
        warnings.push("The sessions directory is not present; Usage and Execution will have no local history.".to_string());
    }
    if provider.compatibility != "responses" {
        warnings
            .push("The active provider is not confirmed as Responses API compatible.".to_string());
    }
    if !provider.credential_available {
        warnings.push("The active provider credential is not available.".to_string());
    }
    if settings_snapshot.settings.history_persistence == "none" {
        warnings.push(
            "Session history persistence is disabled; historical analysis will be incomplete."
                .to_string(),
        );
    }
    if settings_snapshot.settings.sandbox_mode == "danger-full-access"
        && settings_snapshot.settings.approval_policy == "never"
    {
        warnings.push(
            "Codex currently has full filesystem access with approvals disabled.".to_string(),
        );
    }
    if !storage.integrity_ok {
        warnings.push(format!(
            "The analysis database quick check failed: {}",
            storage.integrity_message
        ));
    }
    if storage.foreign_key_violations > 0 {
        warnings.push(format!(
            "The analysis database has {} foreign-key violations.",
            storage.foreign_key_violations
        ));
    }
    if storage.malformed_session_lines > 0 {
        warnings.push(format!(
            "{} local session lines could not be parsed and were skipped.",
            storage.malformed_session_lines
        ));
    }

    let mut extension_paths = vec![
        extension_path("Codex skills", home.join("skills")),
        extension_path(
            "Agent skills",
            user_home()
                .unwrap_or_else(|| home.parent().unwrap_or(Path::new("")).to_path_buf())
                .join(".agents/skills"),
        ),
        extension_path("Plugin cache", home.join("plugins/cache")),
    ];
    extension_paths.retain(|item| item.exists || item.label == "Codex skills");

    EnvironmentSnapshot {
        fetched_at: Utc::now().to_rfc3339(),
        codex_version: codex_version.to_string(),
        codex_binary: display_path(codex_binary),
        codex_home: display_path(&home),
        config_path: settings_snapshot.config_path.clone(),
        sessions_path: display_path(&sessions_path),
        xray_data_path: display_path(xray_data_path),
        xray_sqlite_path: display_path(xray_sqlite_path),
        storage,
        config_version: settings_snapshot.version.clone(),
        provider,
        settings: settings_snapshot.settings.clone(),
        mcp_servers: mcp_servers(raw),
        extension_paths,
        warnings,
    }
}
