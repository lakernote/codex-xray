mod codex;
mod cost_estimate;
mod environment;
mod local_usage;
mod pricing;
mod provider;
mod settings;
mod storage;
mod trace_analysis;

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use codex::{AppServerClient, UsageSnapshot};
use cost_estimate::{
    CostEstimateSnapshot, ProjectTurnUsageDetail, ProjectUsageSnapshot, build_cost_estimate,
    build_project_turn_usage, build_project_usage,
};
use environment::{EnvironmentRuntime, EnvironmentSnapshot, build_environment_snapshot};
use pricing::{
    PricingApplyRequest, PricingConfigSnapshot, activate_pricing_config, pricing_config_snapshot,
    reset_pricing_config as reset_pricing_config_file, save_pricing_config,
};
use provider::{
    ProviderApplyRequest, ProviderSnapshot, ProviderTestResult, build_apply_edits,
    build_provider_snapshot, probe_provider, read_restore_point, restore_edits, restore_point,
    save_restore_point,
};
use settings::{
    SettingsApplyRequest, SettingsSnapshot, build_settings_edits, build_settings_snapshot,
    read_restore_point as read_settings_restore_point, restore_edits as restore_settings_edits,
    restore_point as settings_restore_point, save_restore_point as save_settings_restore_point,
};
use storage::{health as storage_health, read_cache, write_cache};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, WindowEvent};
use trace_analysis::{
    ExtensionUsageSnapshot, TraceIndexCache, TraceSessionDetail, TraceSnapshot,
    analyze_trace_session_cached, build_extension_usage_cached, build_trace_catalog,
    build_trace_snapshot_cached, get_trace_session_detail_cached,
};

struct UsageState {
    // Codex X-Ray owns only this application-data tree.
    client: Arc<Mutex<Option<AppServerClient>>>,
    app_data_dir: PathBuf,
    codex_state_dir: PathBuf,
    database_path: PathBuf,
    pricing_config_path: PathBuf,
    cost_scan: Arc<Mutex<()>>,
    trace_scan: Arc<Mutex<TraceIndexCache>>,
    provider_restore_path: PathBuf,
    settings_restore_path: PathBuf,
}

#[tauri::command]
fn update_tray_summary(
    app: tauri::AppHandle,
    running: usize,
    waiting: usize,
    failed: usize,
) -> Result<(), String> {
    let tray = app
        .tray_by_id("codex-xray")
        .ok_or_else(|| "未找到 Codex X-Ray 托盘图标".to_string())?;
    let tooltip =
        format!("Codex X-Ray · {running} running · {waiting} waiting · {failed} attention");
    tray.set_tooltip(Some(tooltip))
        .map_err(|error| format!("更新托盘状态失败：{error}"))
}

#[tauri::command]
async fn get_environment_snapshot(
    state: tauri::State<'_, UsageState>,
) -> Result<EnvironmentSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let app_data_dir = state.app_data_dir.clone();
    let database_path = state.database_path.clone();
    let provider_restore_path = state.provider_restore_path.clone();
    let settings_restore_path = state.settings_restore_path.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = client
            .lock()
            .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
        if guard.is_none() {
            *guard = Some(
                AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
            );
        }

        let client = guard.as_mut().expect("client is initialized");
        let raw = client.read_config().map_err(|error| error.to_string())?;
        let provider = build_provider_snapshot(&raw, &provider_restore_path)?;
        let settings = build_settings_snapshot(&raw, &settings_restore_path)?;
        let storage = storage_health(&database_path)?;
        Ok(build_environment_snapshot(
            &raw,
            &provider,
            &settings,
            EnvironmentRuntime {
                codex_version: client.codex_version(),
                codex_binary: client.codex_binary(),
                xray_data_path: &app_data_dir,
                xray_sqlite_path: &database_path,
                storage,
            },
        ))
    })
    .await
    .map_err(|error| format!("环境诊断后台任务异常：{error}"))?
}

fn show_main(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Codex X-Ray main window is unavailable".to_string())?;
    window
        .show()
        .and_then(|_| window.unminimize())
        .and_then(|_| window.set_focus())
        .map_err(|error| format!("Unable to show Codex X-Ray: {error}"))
}

#[cfg(target_os = "macos")]
fn launch_target(target: &std::ffi::OsStr) -> Result<(), String> {
    Command::new("open")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Unable to open target: {error}"))
}

#[cfg(target_os = "windows")]
fn launch_target(target: &std::ffi::OsStr) -> Result<(), String> {
    Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Unable to open target: {error}"))
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn launch_target(target: &std::ffi::OsStr) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Unable to open target: {error}"))
}

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    let allowed = [
        "https://learn.chatgpt.com/",
        "https://developers.openai.com/",
        "https://openai.com/",
        "https://help.aliyun.com/",
        "https://www.volcengine.com/",
        "https://cloud.baidu.com/",
        "https://platform.minimaxi.com/",
        "https://platform.stepfun.com/",
    ];
    if url.contains(['\r', '\n']) || !allowed.iter().any(|prefix| url.starts_with(prefix)) {
        return Err("This link is not on the Codex X-Ray documentation allowlist".to_string());
    }
    launch_target(std::ffi::OsStr::new(&url))
}

#[cfg(target_os = "macos")]
fn reveal_target(target: &std::path::Path) -> Result<(), String> {
    Command::new("open")
        .arg("-R")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Unable to reveal local path: {error}"))
}

#[cfg(target_os = "windows")]
fn reveal_target(target: &std::path::Path) -> Result<(), String> {
    Command::new("explorer")
        .arg(format!("/select,{}", target.display()))
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Unable to reveal local path: {error}"))
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn reveal_target(target: &std::path::Path) -> Result<(), String> {
    let directory = if target.is_dir() {
        target
    } else {
        target.parent().unwrap_or(target)
    };
    Command::new("xdg-open")
        .arg(directory)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Unable to reveal local path: {error}"))
}

#[tauri::command]
fn reveal_local_path(path: String, state: tauri::State<'_, UsageState>) -> Result<(), String> {
    if path.contains(['\0', '\r', '\n']) {
        return Err("Invalid local path".to_string());
    }
    let target = PathBuf::from(path);
    if !target.is_absolute() || !target.exists() {
        return Err("The local path does not exist".to_string());
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let allowed = home
        .iter()
        .chain(std::iter::once(&state.app_data_dir))
        .any(|root| target.starts_with(root));
    if !allowed {
        return Err("The path is outside the user and Codex X-Ray data directories".to_string());
    }

    reveal_target(&target)
}

#[tauri::command]
fn get_cached_usage(state: tauri::State<'_, UsageState>) -> Result<Option<UsageSnapshot>, String> {
    read_cache(&state.database_path, "usage.snapshot")
}

#[tauri::command]
async fn get_usage(state: tauri::State<'_, UsageState>) -> Result<UsageSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let database_path = state.database_path.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = client
            .lock()
            .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;

        if guard.is_none() {
            *guard = Some(
                AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
            );
        }

        let first_result = guard.as_mut().expect("client is initialized").fetch_usage();

        let mut snapshot = if let Ok(snapshot) = first_result {
            snapshot
        } else {
            guard.take();
            let mut restarted = AppServerClient::start(&codex_state_dir)
                .map_err(|error| format!("重新连接 Codex App Server 失败：{error}"))?;
            let result = restarted.fetch_usage().map_err(|error| error.to_string());
            *guard = Some(restarted);
            result?
        };

        if let Err(error) = write_cache(&database_path, "usage.snapshot", &snapshot) {
            snapshot
                .warnings
                .push(format!("Usage 启动缓存未保存：{error}"));
        }
        Ok(snapshot)
    })
    .await
    .map_err(|error| format!("Usage 后台任务异常：{error}"))?
}

#[tauri::command]
fn get_cached_cost_estimate(
    state: tauri::State<'_, UsageState>,
) -> Result<Option<CostEstimateSnapshot>, String> {
    read_cache(&state.database_path, "cost.snapshot")
}

#[tauri::command]
async fn get_cost_estimate(
    state: tauri::State<'_, UsageState>,
) -> Result<CostEstimateSnapshot, String> {
    let cost_scan = Arc::clone(&state.cost_scan);
    let database_path = state.database_path.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let _guard = cost_scan
            .lock()
            .map_err(|_| "Codex X-Ray 成本索引状态已损坏".to_string())?;
        let mut estimate = build_cost_estimate(&database_path)?;
        if let Err(error) = write_cache(&database_path, "cost.snapshot", &estimate) {
            estimate
                .warnings
                .push(format!("成本启动缓存未保存：{error}"));
        }
        Ok(estimate)
    })
    .await
    .map_err(|error| format!("成本后台任务异常：{error}"))?
}

#[tauri::command]
fn get_cached_project_usage(
    state: tauri::State<'_, UsageState>,
) -> Result<Option<ProjectUsageSnapshot>, String> {
    read_cache(&state.database_path, "project.snapshot")
}

#[tauri::command]
async fn get_project_usage(
    state: tauri::State<'_, UsageState>,
) -> Result<ProjectUsageSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let cost_scan = Arc::clone(&state.cost_scan);
    let database_path = state.database_path.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let metadata_result = (|| {
            let mut client_guard = client
                .lock()
                .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
            if client_guard.is_none() {
                *client_guard = Some(
                    AppServerClient::start(&codex_state_dir)
                        .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
                );
            }
            client_guard
                .as_mut()
                .expect("client is initialized")
                .fetch_thread_metadata()
                .map_err(|error| error.to_string())
        })();
        let (thread_metadata, metadata_warning) = match metadata_result {
            Ok(metadata) => (Some(metadata), None),
            Err(error) => (
                None,
                Some(format!("官方对话名暂不可用，已保留 Session ID：{error}")),
            ),
        };

        let _guard = cost_scan
            .lock()
            .map_err(|_| "Codex X-Ray 成本索引状态已损坏".to_string())?;
        let mut snapshot = match build_project_usage(&database_path, thread_metadata.as_deref()) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                build_cost_estimate(&database_path)?;
                build_project_usage(&database_path, thread_metadata.as_deref())?
            }
        };
        if let Some(warning) = metadata_warning {
            snapshot.warnings.push(warning);
        }
        if let Err(error) = write_cache(&database_path, "project.snapshot", &snapshot) {
            snapshot
                .warnings
                .push(format!("项目用量缓存未保存：{error}"));
        }
        Ok(snapshot)
    })
    .await
    .map_err(|error| format!("项目用量后台任务异常：{error}"))?
}

#[tauri::command]
async fn get_project_turn_usage(
    state: tauri::State<'_, UsageState>,
    session_id: String,
) -> Result<ProjectTurnUsageDetail, String> {
    let cost_scan = Arc::clone(&state.cost_scan);
    let database_path = state.database_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = cost_scan
            .lock()
            .map_err(|_| "Codex X-Ray 成本索引状态已损坏".to_string())?;
        build_project_turn_usage(&database_path, &session_id)
    })
    .await
    .map_err(|error| format!("Turn 用量后台任务异常：{error}"))?
}

#[tauri::command]
fn get_pricing_config(
    state: tauri::State<'_, UsageState>,
) -> Result<PricingConfigSnapshot, String> {
    pricing_config_snapshot(&state.pricing_config_path)
}

#[tauri::command]
fn apply_pricing_config(
    state: tauri::State<'_, UsageState>,
    request: PricingApplyRequest,
) -> Result<PricingConfigSnapshot, String> {
    let _guard = state
        .cost_scan
        .lock()
        .map_err(|_| "Codex X-Ray 成本索引状态已损坏".to_string())?;
    save_pricing_config(&state.pricing_config_path, request)
}

#[tauri::command]
fn reset_pricing_config(
    state: tauri::State<'_, UsageState>,
) -> Result<PricingConfigSnapshot, String> {
    let _guard = state
        .cost_scan
        .lock()
        .map_err(|_| "Codex X-Ray 成本索引状态已损坏".to_string())?;
    reset_pricing_config_file(&state.pricing_config_path)
}

#[tauri::command]
fn get_cached_trace(state: tauri::State<'_, UsageState>) -> Result<Option<TraceSnapshot>, String> {
    read_cache(&state.database_path, "trace.snapshot")
}

#[tauri::command]
async fn get_trace(state: tauri::State<'_, UsageState>) -> Result<TraceSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let trace_scan = Arc::clone(&state.trace_scan);
    let database_path = state.database_path.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let (thread_metadata, metadata_warning) = {
            let mut client_guard = client
                .lock()
                .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
            if client_guard.is_none() {
                *client_guard = Some(
                    AppServerClient::start(&codex_state_dir)
                        .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
                );
            }
            match client_guard
                .as_mut()
                .expect("client is initialized")
                .fetch_thread_metadata()
            {
                Ok(metadata) => (Some(metadata), None),
                Err(error) => (
                    None,
                    Some(format!(
                        "官方对话名称和运行状态暂不可用，已使用本地 session 元数据：{error}"
                    )),
                ),
            }
        };
        let mut guard = trace_scan
            .lock()
            .map_err(|_| "Codex X-Ray 执行解剖索引状态已损坏".to_string())?;
        let mut snapshot =
            build_trace_snapshot_cached(&database_path, &mut guard, thread_metadata.as_deref())?;
        if let Some(warning) = metadata_warning {
            snapshot.warnings.push(warning);
        }
        if let Err(error) = write_cache(&database_path, "trace.snapshot", &snapshot) {
            snapshot
                .warnings
                .push(format!("执行解剖缓存未保存：{error}"));
        }
        Ok(snapshot)
    })
    .await
    .map_err(|error| format!("执行解剖后台任务异常：{error}"))?
}

#[tauri::command]
async fn get_trace_catalog(state: tauri::State<'_, UsageState>) -> Result<TraceSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let database_path = state.database_path.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let thread_metadata = {
            let mut client_guard = client
                .lock()
                .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
            if client_guard.is_none() {
                *client_guard = Some(
                    AppServerClient::start(&codex_state_dir)
                        .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
                );
            }
            client_guard
                .as_mut()
                .expect("client is initialized")
                .fetch_thread_metadata()
                .map_err(|error| format!("无法读取 Codex 项目与对话目录：{error}"))?
        };
        let mut snapshot = build_trace_catalog(&database_path, &thread_metadata)?;
        if let Err(error) = write_cache(&database_path, "trace.snapshot", &snapshot) {
            snapshot
                .warnings
                .push(format!("项目目录与分析状态未能持久化：{error}"));
        }
        Ok(snapshot)
    })
    .await
    .map_err(|error| format!("项目目录后台任务异常：{error}"))?
}

#[tauri::command]
async fn get_trace_session(
    state: tauri::State<'_, UsageState>,
    session_id: String,
) -> Result<Option<TraceSessionDetail>, String> {
    let trace_scan = Arc::clone(&state.trace_scan);
    let database_path = state.database_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = trace_scan
            .lock()
            .map_err(|_| "Codex X-Ray 执行解剖索引状态已损坏".to_string())?;
        get_trace_session_detail_cached(&database_path, &session_id, &mut guard)
    })
    .await
    .map_err(|error| format!("执行解剖详情后台任务异常：{error}"))?
}

#[tauri::command]
async fn analyze_trace_session(
    state: tauri::State<'_, UsageState>,
    session_id: String,
    session_path: Option<String>,
) -> Result<Option<TraceSessionDetail>, String> {
    let trace_scan = Arc::clone(&state.trace_scan);
    let database_path = state.database_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = trace_scan
            .lock()
            .map_err(|_| "Codex X-Ray 执行解剖索引状态已损坏".to_string())?;
        analyze_trace_session_cached(
            &database_path,
            &session_id,
            session_path.as_deref(),
            &mut guard,
        )
    })
    .await
    .map_err(|error| format!("Session 执行解剖后台任务异常：{error}"))?
}

#[tauri::command]
async fn get_extension_usage(
    state: tauri::State<'_, UsageState>,
) -> Result<ExtensionUsageSnapshot, String> {
    let trace_scan = Arc::clone(&state.trace_scan);
    let database_path = state.database_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = trace_scan
            .lock()
            .map_err(|_| "Codex X-Ray 扩展使用索引状态已损坏".to_string())?;
        build_extension_usage_cached(&database_path, &mut guard)
    })
    .await
    .map_err(|error| format!("扩展使用统计后台任务异常：{error}"))?
}

#[tauri::command]
async fn get_provider_config(
    state: tauri::State<'_, UsageState>,
) -> Result<ProviderSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let restore_path = state.provider_restore_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = client
            .lock()
            .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
        if guard.is_none() {
            *guard = Some(
                AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
            );
        }
        let raw = match guard.as_mut().expect("client is initialized").read_config() {
            Ok(raw) => raw,
            Err(_) => {
                guard.take();
                let mut restarted = AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("重新连接 Codex App Server 失败：{error}"))?;
                let raw = restarted.read_config().map_err(|error| error.to_string())?;
                *guard = Some(restarted);
                raw
            }
        };
        let mut snapshot = build_provider_snapshot(&raw, &restore_path)?;
        match guard
            .as_mut()
            .expect("client is initialized")
            .fetch_models()
        {
            Ok(models) => snapshot.models = models,
            Err(error) => snapshot
                .warnings
                .push(format!("官方模型目录暂不可用：{error}")),
        }
        Ok(snapshot)
    })
    .await
    .map_err(|error| format!("Provider 配置读取任务异常：{error}"))?
}

#[tauri::command]
async fn apply_provider_config(
    state: tauri::State<'_, UsageState>,
    request: ProviderApplyRequest,
) -> Result<ProviderSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let restore_path = state.provider_restore_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = client
            .lock()
            .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
        if guard.is_none() {
            *guard = Some(
                AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
            );
        }
        let client = guard.as_mut().expect("client is initialized");
        let before_raw = client.read_config().map_err(|error| error.to_string())?;
        let before = build_provider_snapshot(&before_raw, &restore_path)?;
        let (edits, _) = build_apply_edits(&request)?;
        client
            .batch_write_config(edits, request.expected_version)
            .map_err(|error| format!("Codex 拒绝了 Provider 配置变更：{error}"))?;
        let after_raw = client.read_config().map_err(|error| error.to_string())?;
        let mut after = build_provider_snapshot(&after_raw, &restore_path)?;
        match client.fetch_models() {
            Ok(models) => after.models = models,
            Err(error) => after
                .warnings
                .push(format!("官方模型目录暂不可用：{error}")),
        }
        save_restore_point(&restore_path, &restore_point(&before))?;
        after.restore_available = true;
        Ok(after)
    })
    .await
    .map_err(|error| format!("Provider 配置写入任务异常：{error}"))?
}

#[tauri::command]
async fn test_provider_connection(
    state: tauri::State<'_, UsageState>,
    request: ProviderApplyRequest,
) -> Result<ProviderTestResult, String> {
    if request.provider_id.trim() != "openai" {
        return tauri::async_runtime::spawn_blocking(move || probe_provider(&request))
            .await
            .map_err(|error| format!("Provider 连接测试后台任务异常：{error}"))?;
    }

    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let provider_restore_path = state.provider_restore_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let _ = build_apply_edits(&request)?;
        let mut guard = client
            .lock()
            .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
        if guard.is_none() {
            *guard = Some(
                AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
            );
        }
        let app_server = guard.as_mut().expect("client is initialized");
        let raw = app_server.read_config().map_err(|error| error.to_string())?;
        let _ = build_provider_snapshot(&raw, &provider_restore_path)?;
        let models = app_server
            .fetch_models()
            .map_err(|error| format!("Codex 官方模型目录请求失败：{error}"))?;
        let model = request.model.trim().to_string();
        let found = models.iter().any(|candidate| candidate.id == model);
        Ok(ProviderTestResult {
            success: found,
            check_kind: "codex_model_catalog".to_string(),
            provider_id: "openai".to_string(),
            model: model.clone(),
            endpoint: None,
            latency_ms: started.elapsed().as_millis(),
            http_status: None,
            message: if found {
                "Codex 官方 model/list 已返回该模型；登录与模型目录可用。此检查不会额外发起一次 LLM 生成。"
                    .to_string()
            } else {
                format!(
                    "Codex 官方 model/list 未返回 {model}。请从当前模型建议中选择，或刷新后再试。"
                )
            },
        })
    })
    .await
    .map_err(|error| format!("OpenAI 模型验证后台任务异常：{error}"))?
}

#[tauri::command]
async fn restore_provider_config(
    state: tauri::State<'_, UsageState>,
) -> Result<ProviderSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let restore_path = state.provider_restore_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target = read_restore_point(&restore_path)?;
        let mut guard = client
            .lock()
            .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
        if guard.is_none() {
            *guard = Some(
                AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
            );
        }
        let client = guard.as_mut().expect("client is initialized");
        let before_raw = client.read_config().map_err(|error| error.to_string())?;
        let before = build_provider_snapshot(&before_raw, &restore_path)?;
        client
            .batch_write_config(restore_edits(&target)?, before.version.clone())
            .map_err(|error| format!("Codex 拒绝了 Provider 恢复操作：{error}"))?;
        let after_raw = client.read_config().map_err(|error| error.to_string())?;
        let mut after = build_provider_snapshot(&after_raw, &restore_path)?;
        match client.fetch_models() {
            Ok(models) => after.models = models,
            Err(error) => after
                .warnings
                .push(format!("官方模型目录暂不可用：{error}")),
        }
        save_restore_point(&restore_path, &restore_point(&before))?;
        after.restore_available = true;
        Ok(after)
    })
    .await
    .map_err(|error| format!("Provider 配置恢复任务异常：{error}"))?
}

#[tauri::command]
async fn get_codex_settings(
    state: tauri::State<'_, UsageState>,
) -> Result<SettingsSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let restore_path = state.settings_restore_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = client
            .lock()
            .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
        if guard.is_none() {
            *guard = Some(
                AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
            );
        }
        let raw = match guard.as_mut().expect("client is initialized").read_config() {
            Ok(raw) => raw,
            Err(_) => {
                guard.take();
                let mut restarted = AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("重新连接 Codex App Server 失败：{error}"))?;
                let raw = restarted.read_config().map_err(|error| error.to_string())?;
                *guard = Some(restarted);
                raw
            }
        };
        build_settings_snapshot(&raw, &restore_path)
    })
    .await
    .map_err(|error| format!("Codex 设置读取任务异常：{error}"))?
}

#[tauri::command]
async fn apply_codex_settings(
    state: tauri::State<'_, UsageState>,
    request: SettingsApplyRequest,
) -> Result<SettingsSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let restore_path = state.settings_restore_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = client
            .lock()
            .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
        if guard.is_none() {
            *guard = Some(
                AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
            );
        }
        let client = guard.as_mut().expect("client is initialized");
        let before_raw = client.read_config().map_err(|error| error.to_string())?;
        let before = build_settings_snapshot(&before_raw, &restore_path)?;
        let (edits, changed_keys) = build_settings_edits(&before.settings, &request.settings)?;
        if edits.is_empty() {
            return Ok(before);
        }
        client
            .batch_write_config_with_reload(edits, request.expected_version, true)
            .map_err(|error| format!("Codex 拒绝了设置变更：{error}"))?;
        let after_raw = client.read_config().map_err(|error| error.to_string())?;
        let mut after = build_settings_snapshot(&after_raw, &restore_path)?;
        save_settings_restore_point(
            &restore_path,
            &settings_restore_point(&before.settings, changed_keys),
        )?;
        after.restore_available = true;
        Ok(after)
    })
    .await
    .map_err(|error| format!("Codex 设置写入任务异常：{error}"))?
}

#[tauri::command]
async fn restore_codex_settings(
    state: tauri::State<'_, UsageState>,
) -> Result<SettingsSnapshot, String> {
    let client = Arc::clone(&state.client);
    let codex_state_dir = state.codex_state_dir.clone();
    let restore_path = state.settings_restore_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target = read_settings_restore_point(&restore_path)?;
        let edits = restore_settings_edits(&target)?;
        let mut guard = client
            .lock()
            .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
        if guard.is_none() {
            *guard = Some(
                AppServerClient::start(&codex_state_dir)
                    .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
            );
        }
        let client = guard.as_mut().expect("client is initialized");
        let before_raw = client.read_config().map_err(|error| error.to_string())?;
        let before = build_settings_snapshot(&before_raw, &restore_path)?;
        client
            .batch_write_config_with_reload(edits, before.version.clone(), true)
            .map_err(|error| format!("Codex 拒绝了设置恢复操作：{error}"))?;
        let after_raw = client.read_config().map_err(|error| error.to_string())?;
        let mut after = build_settings_snapshot(&after_raw, &restore_path)?;
        let (_, changed_keys) = build_settings_edits(&after.settings, &before.settings)?;
        save_settings_restore_point(
            &restore_path,
            &settings_restore_point(&before.settings, changed_keys),
        )?;
        after.restore_available = true;
        Ok(after)
    })
    .await
    .map_err(|error| format!("Codex 设置恢复任务异常：{error}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("无法确定 Codex X-Ray 数据目录：{error}"))?;
            let database_path = app_data_dir.join("codex-xray.sqlite");
            storage::initialize(&database_path)?;
            let pricing_config_path = app_data_dir.join("pricing-config.json");
            if let Err(error) = activate_pricing_config(&pricing_config_path) {
                eprintln!("Codex X-Ray 单价设置未载入，将使用公开默认值：{error}");
            }

            app.manage(UsageState {
                client: Arc::new(Mutex::new(None)),
                app_data_dir: app_data_dir.clone(),
                codex_state_dir: app_data_dir.join("codex-state"),
                database_path,
                pricing_config_path,
                cost_scan: Arc::new(Mutex::new(())),
                trace_scan: Arc::new(Mutex::new(TraceIndexCache::default())),
                provider_restore_path: app_data_dir.join("provider-restore.json"),
                settings_restore_path: app_data_dir.join("settings-restore.json"),
            });

            let open_item = MenuItem::with_id(
                app,
                "show_main",
                "Open Codex X-Ray / 打开 Codex X-Ray",
                true,
                None::<&str>,
            )?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit / 退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_item, &separator, &quit_item])?;
            let mut tray = TrayIconBuilder::with_id("codex-xray")
                .menu(&menu)
                .tooltip("Codex X-Ray · Usage, trace & control")
                .icon_as_template(false);
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.on_menu_event(|app, event| match event.id().as_ref() {
                "show_main" => {
                    let _ = show_main(app);
                }
                "quit" => app.exit(0),
                _ => {}
            })
            .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_cached_usage,
            get_usage,
            get_cached_cost_estimate,
            get_cost_estimate,
            get_cached_project_usage,
            get_project_usage,
            get_project_turn_usage,
            get_pricing_config,
            apply_pricing_config,
            reset_pricing_config,
            get_cached_trace,
            get_trace,
            get_trace_catalog,
            get_trace_session,
            analyze_trace_session,
            get_extension_usage,
            get_provider_config,
            test_provider_connection,
            apply_provider_config,
            restore_provider_config,
            get_codex_settings,
            apply_codex_settings,
            restore_codex_settings,
            get_environment_snapshot,
            update_tray_summary,
            open_external,
            reveal_local_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex X-Ray");
}
