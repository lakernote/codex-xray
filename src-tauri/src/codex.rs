use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::local_usage::{LocalTodayUsage, scan_today_usage};
use crate::protocol_capture::{ProtocolRecord, record_json};
use crate::provider::ModelOption;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const THREAD_METADATA_SCAN_INTERVAL: Duration = Duration::from_secs(60);
const THREAD_PAGE_LIMIT: usize = 200;
const MAX_THREAD_PAGES: usize = 100;

#[derive(Debug, Error)]
pub enum CodexError {
    #[error("找不到可运行的 Codex CLI；请安装 Codex CLI 或设置 CODEX_BIN")]
    CodexNotFound,
    #[error("无法创建 Codex X-Ray 状态目录：{0}")]
    StateDirectory(String),
    #[error("启动 Codex 失败：{0}")]
    Spawn(String),
    #[error("Codex App Server 没有提供 {0}")]
    MissingPipe(&'static str),
    #[error("发送 {method} 失败：{message}")]
    Send { method: String, message: String },
    #[error("{method} 超时（{seconds} 秒）")]
    Timeout { method: String, seconds: u64 },
    #[error("{method} 返回错误：{message}")]
    Rpc { method: String, message: String },
    #[error("{method} 返回的数据无法解析：{message}")]
    InvalidResponse { method: String, message: String },
    #[error("Codex App Server 已断开；{0}")]
    Disconnected(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub fetched_at: String,
    pub codex_version: String,
    pub account: Option<AccountInfo>,
    pub current_limit_id: Option<String>,
    pub rate_limits: Vec<RateLimit>,
    pub summary: Option<UsageSummary>,
    pub daily_usage: Vec<DailyUsage>,
    #[serde(default)]
    pub local_today: Option<LocalTodayUsage>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub account_type: String,
    pub plan_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all(deserialize = "camelCase"))]
pub struct RateLimitWindow {
    pub used_percent: f64,
    pub window_duration_mins: Option<u64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all(deserialize = "camelCase"))]
pub struct CreditsInfo {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateLimit {
    pub limit_id: String,
    pub limit_name: Option<String>,
    pub plan_type: Option<String>,
    pub primary: Option<RateLimitWindow>,
    pub secondary: Option<RateLimitWindow>,
    pub credits: Option<CreditsInfo>,
    pub spend_control_reached: Option<bool>,
    pub rate_limit_reached_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageSummary {
    pub lifetime_tokens: Option<u64>,
    pub peak_daily_tokens: Option<u64>,
    pub longest_running_turn_sec: Option<u64>,
    pub current_streak_days: Option<u64>,
    pub longest_streak_days: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DailyUsage {
    pub start_date: String,
    pub tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ThreadMetadata {
    pub id: String,
    pub name: Option<String>,
    pub cwd: String,
    pub status: Option<String>,
    pub path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub parent_thread_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAccountResponse {
    account: Option<RawAccount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAccount {
    #[serde(rename = "type")]
    account_type: String,
    plan_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRateLimitResponse {
    rate_limits: RawRateLimit,
    rate_limits_by_limit_id: Option<BTreeMap<String, RawRateLimit>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRateLimit {
    limit_id: Option<String>,
    limit_name: Option<String>,
    primary: Option<RateLimitWindow>,
    secondary: Option<RateLimitWindow>,
    credits: Option<CreditsInfo>,
    spend_control_reached: Option<bool>,
    plan_type: Option<String>,
    rate_limit_reached_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawUsageResponse {
    summary: Option<UsageSummaryCamel>,
    daily_usage_buckets: Option<Vec<DailyUsageCamel>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageSummaryCamel {
    lifetime_tokens: Option<u64>,
    peak_daily_tokens: Option<u64>,
    longest_running_turn_sec: Option<u64>,
    current_streak_days: Option<u64>,
    longest_streak_days: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyUsageCamel {
    start_date: String,
    tokens: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThreadListResponse {
    data: Vec<RawThreadMetadata>,
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThreadMetadata {
    id: String,
    name: Option<String>,
    cwd: String,
    status: Value,
    path: Option<String>,
    created_at: i64,
    updated_at: i64,
    parent_thread_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawSessionIndexEntry {
    id: String,
    thread_name: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug)]
struct LocalSessionIndexEntry {
    name: Option<String>,
    updated_at: Option<i64>,
}

#[derive(Debug)]
struct LocalSessionMetadata {
    id: String,
    cwd: String,
    source: String,
    created_at: i64,
    parent_thread_id: Option<String>,
}

pub struct AppServerClient {
    child: Child,
    input: BufWriter<ChildStdin>,
    messages: Receiver<Value>,
    stderr_tail: Arc<Mutex<String>>,
    protocol_pending: Arc<Mutex<BTreeMap<u64, (String, Instant)>>>,
    next_id: u64,
    codex_version: String,
    codex_binary: PathBuf,
    last_thread_metadata_scan: Option<Instant>,
}

impl AppServerClient {
    pub fn start(state_dir: &Path) -> Result<Self, CodexError> {
        fs::create_dir_all(state_dir)
            .map_err(|error| CodexError::StateDirectory(error.to_string()))?;

        let codex_binary = discover_codex_binary()?;
        let codex_version = read_codex_version(&codex_binary)?;
        let sqlite_value = serde_json::to_string(&state_dir.to_string_lossy().as_ref())
            .map_err(|error| CodexError::StateDirectory(error.to_string()))?;
        let sqlite_override = format!("sqlite_home={sqlite_value}");

        let mut child = Command::new(&codex_binary)
            .args(["-c", &sqlite_override, "app-server", "--listen", "stdio://"])
            .env("CODEX_SQLITE_HOME", state_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| CodexError::Spawn(error.to_string()))?;

        let input = child.stdin.take().ok_or(CodexError::MissingPipe("stdin"))?;
        let output = child
            .stdout
            .take()
            .ok_or(CodexError::MissingPipe("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(CodexError::MissingPipe("stderr"))?;

        let (message_tx, messages) = mpsc::channel();
        let protocol_pending = Arc::new(Mutex::new(BTreeMap::<u64, (String, Instant)>::new()));
        let response_pending = Arc::clone(&protocol_pending);
        thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Ok(message) = serde_json::from_str::<Value>(&line) {
                    let response_id = message.get("id").and_then(Value::as_u64);
                    let pending = response_id.and_then(|id| {
                        response_pending
                            .lock()
                            .ok()
                            .and_then(|mut pending| pending.remove(&id))
                    });
                    let method = message
                        .get("method")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| pending.as_ref().map(|(method, _)| method.clone()));
                    let correlation_id = response_id.map(|id| id.to_string());
                    let duration_ms = pending
                        .as_ref()
                        .map(|(_, started)| started.elapsed().as_millis() as u64);
                    let kind = if response_id.is_some() {
                        if message.get("error").is_some() {
                            "error"
                        } else {
                            "response"
                        }
                    } else {
                        "notification"
                    };
                    record_json(
                        ProtocolRecord {
                            channel: "app_server",
                            direction: "server_to_xray",
                            kind,
                            method: method.as_deref(),
                            correlation_id: correlation_id.as_deref(),
                            status: None,
                            duration_ms,
                        },
                        &message,
                    );
                    if message_tx.send(message).is_err() {
                        break;
                    }
                }
            }
        });

        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let stderr_writer = Arc::clone(&stderr_tail);
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Ok(mut tail) = stderr_writer.lock() {
                    tail.push_str(&line);
                    tail.push('\n');
                    if tail.len() > 8_000 {
                        let start = tail.len() - 8_000;
                        *tail = tail[start..].to_owned();
                    }
                }
            }
        });

        let mut client = Self {
            child,
            input: BufWriter::new(input),
            messages,
            stderr_tail,
            protocol_pending,
            next_id: 1,
            codex_version,
            codex_binary,
            last_thread_metadata_scan: None,
        };

        client.request_with_timeout(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "codex-xray",
                    "title": "Codex X-Ray",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
            STARTUP_TIMEOUT,
        )?;
        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    pub fn codex_version(&self) -> &str {
        &self.codex_version
    }

    pub fn codex_binary(&self) -> &Path {
        &self.codex_binary
    }

    pub fn fetch_usage(&mut self) -> Result<UsageSnapshot, CodexError> {
        let mut warnings = Vec::new();

        let account = match self.request("account/read", json!({ "refreshToken": false })) {
            Ok(value) => match parse_account(value) {
                Ok(value) => value,
                Err(error) => {
                    warnings.push(error.to_string());
                    None
                }
            },
            Err(error) => {
                warnings.push(format!("账户信息不可用：{error}"));
                None
            }
        };

        let (current_limit_id, rate_limits) =
            match self.request("account/rateLimits/read", json!({})) {
                Ok(value) => match parse_rate_limits(value) {
                    Ok(value) => value,
                    Err(error) => {
                        warnings.push(error.to_string());
                        (None, Vec::new())
                    }
                },
                Err(error) => {
                    warnings.push(format!("额度窗口不可用：{error}"));
                    (None, Vec::new())
                }
            };

        let (summary, daily_usage) = match self.request("account/usage/read", json!({})) {
            Ok(value) => match parse_usage(value) {
                Ok(value) => value,
                Err(error) => {
                    warnings.push(error.to_string());
                    (None, Vec::new())
                }
            },
            Err(error) => {
                warnings.push(format!("Token 汇总不可用：{error}"));
                (None, Vec::new())
            }
        };

        let local_today = match scan_today_usage() {
            Ok(usage) => Some(usage),
            Err(error) => {
                warnings.push(format!("本机今日 Token 暂不可用：{error}"));
                None
            }
        };

        if account.is_none() && rate_limits.is_empty() && summary.is_none() {
            return Err(CodexError::InvalidResponse {
                method: "Usage".to_string(),
                message: warnings.join("；"),
            });
        }

        Ok(UsageSnapshot {
            fetched_at: Utc::now().to_rfc3339(),
            codex_version: self.codex_version.clone(),
            account,
            current_limit_id,
            rate_limits,
            summary,
            daily_usage,
            local_today,
            warnings,
        })
    }

    pub fn fetch_thread_metadata(&mut self) -> Result<Vec<ThreadMetadata>, CodexError> {
        let use_cached_metadata = self
            .last_thread_metadata_scan
            .is_some_and(|scanned| scanned.elapsed() < THREAD_METADATA_SCAN_INTERVAL);
        let scan_modes: &[bool] = if use_cached_metadata {
            &[true, false]
        } else {
            &[false]
        };

        for &use_state_db_only in scan_modes {
            let mut cursor: Option<String> = None;
            let mut threads = Vec::new();
            let mut seen_cursors = HashSet::new();
            let mut pages_read = 0usize;

            loop {
                let value = self.request_with_timeout(
                    "thread/list",
                    json!({
                        "cursor": cursor,
                        "limit": THREAD_PAGE_LIMIT,
                        "sortKey": "updated_at",
                        "sortDirection": "desc",
                        "useStateDbOnly": use_state_db_only
                    }),
                    STARTUP_TIMEOUT,
                )?;
                let page =
                    serde_json::from_value::<RawThreadListResponse>(value).map_err(|error| {
                        CodexError::InvalidResponse {
                            method: "thread/list".to_string(),
                            message: error.to_string(),
                        }
                    })?;

                threads.extend(page.data.into_iter().map(|thread| {
                    ThreadMetadata {
                        id: thread.id,
                        name: thread
                            .name
                            .map(|name| name.trim().to_string())
                            .filter(|name| !name.is_empty()),
                        cwd: thread.cwd,
                        status: normalize_thread_status(&thread.status),
                        path: thread.path,
                        created_at: thread.created_at,
                        updated_at: thread.updated_at,
                        parent_thread_id: thread.parent_thread_id,
                    }
                }));

                pages_read += 1;
                let Some(next_cursor) = page.next_cursor else {
                    break;
                };
                if pages_read >= MAX_THREAD_PAGES || !seen_cursors.insert(next_cursor.clone()) {
                    break;
                }
                cursor = Some(next_cursor);
            }

            if !threads.is_empty() || !use_state_db_only {
                supplement_thread_metadata_from_local_sessions(&mut threads);
                if !use_state_db_only {
                    self.last_thread_metadata_scan = Some(Instant::now());
                }
                return Ok(threads);
            }
        }

        Ok(Vec::new())
    }

    pub fn read_config(&mut self) -> Result<Value, CodexError> {
        self.request(
            "config/read",
            json!({
                "cwd": null,
                "includeLayers": true
            }),
        )
    }

    pub fn fetch_models(&mut self) -> Result<Vec<ModelOption>, CodexError> {
        let value = self.request(
            "model/list",
            json!({
                "limit": 100,
                "includeHidden": false
            }),
        )?;
        let data = value.get("data").and_then(Value::as_array).ok_or_else(|| {
            CodexError::InvalidResponse {
                method: "model/list".to_string(),
                message: "缺少 data 数组".to_string(),
            }
        })?;
        Ok(data
            .iter()
            .filter_map(|item| {
                let id = item
                    .get("model")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)?
                    .to_string();
                let display_name = item
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string();
                let supported_reasoning_efforts = item
                    .get("supportedReasoningEfforts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|effort| {
                        effort
                            .get("reasoningEffort")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                    .collect();
                Some(ModelOption {
                    id,
                    display_name,
                    default_reasoning_effort: item
                        .get("defaultReasoningEffort")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    supported_reasoning_efforts,
                    supports_personality: item
                        .get("supportsPersonality")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    is_default: item
                        .get("isDefault")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect())
    }

    pub fn batch_write_config(
        &mut self,
        edits: Vec<Value>,
        expected_version: Option<String>,
    ) -> Result<Value, CodexError> {
        self.batch_write_config_with_reload(edits, expected_version, false)
    }

    pub fn batch_write_config_with_reload(
        &mut self,
        edits: Vec<Value>,
        expected_version: Option<String>,
        reload_user_config: bool,
    ) -> Result<Value, CodexError> {
        self.request(
            "config/batchWrite",
            json!({
                "edits": edits,
                "expectedVersion": expected_version,
                "filePath": null,
                "reloadUserConfig": reload_user_config
            }),
        )
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, CodexError> {
        self.request_with_timeout(method, params, REQUEST_TIMEOUT)
    }

    fn request_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CodexError> {
        let id = self.next_id;
        self.next_id += 1;

        let payload = json!({
            "method": method,
            "id": id,
            "params": params,
        });
        if let Ok(mut pending) = self.protocol_pending.lock() {
            pending.insert(id, (method.to_string(), Instant::now()));
        }
        if let Err(error) = self.write_message(method, &payload) {
            if let Ok(mut pending) = self.protocol_pending.lock() {
                pending.remove(&id);
            }
            return Err(error);
        }

        let started_at = Instant::now();
        loop {
            let remaining = match timeout.checked_sub(started_at.elapsed()) {
                Some(remaining) => remaining,
                None => {
                    if let Ok(mut pending) = self.protocol_pending.lock() {
                        pending.remove(&id);
                    }
                    return Err(CodexError::Timeout {
                        method: method.to_string(),
                        seconds: timeout.as_secs(),
                    });
                }
            };

            let message = match self.messages.recv_timeout(remaining) {
                Ok(message) => message,
                Err(error) => {
                    if let Ok(mut pending) = self.protocol_pending.lock() {
                        pending.remove(&id);
                    }
                    return Err(if matches!(error, mpsc::RecvTimeoutError::Timeout) {
                        CodexError::Timeout {
                            method: method.to_string(),
                            seconds: timeout.as_secs(),
                        }
                    } else {
                        CodexError::Disconnected(self.stderr())
                    });
                }
            };

            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }

            if let Some(error) = message.get("error") {
                let detail = error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| error.to_string());
                return Err(CodexError::Rpc {
                    method: method.to_string(),
                    message: detail,
                });
            }

            return message
                .get("result")
                .cloned()
                .ok_or_else(|| CodexError::InvalidResponse {
                    method: method.to_string(),
                    message: "缺少 result 字段".to_string(),
                });
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), CodexError> {
        let payload = json!({
            "method": method,
            "params": params,
        });
        self.write_message(method, &payload)
    }

    fn write_message(&mut self, method: &str, payload: &Value) -> Result<(), CodexError> {
        let correlation_id = payload
            .get("id")
            .and_then(Value::as_u64)
            .map(|id| id.to_string());
        record_json(
            ProtocolRecord {
                channel: "app_server",
                direction: "xray_to_server",
                kind: if correlation_id.is_some() {
                    "request"
                } else {
                    "notification"
                },
                method: Some(method),
                correlation_id: correlation_id.as_deref(),
                status: None,
                duration_ms: None,
            },
            payload,
        );
        writeln!(self.input, "{payload}")
            .and_then(|_| self.input.flush())
            .map_err(|error| CodexError::Send {
                method: method.to_string(),
                message: error.to_string(),
            })
    }

    fn stderr(&self) -> String {
        self.stderr_tail
            .lock()
            .map(|tail| tail.trim().to_string())
            .unwrap_or_else(|_| "无法读取 stderr".to_string())
    }
}

fn normalize_thread_status(value: &Value) -> Option<String> {
    let status_type = value.get("type").and_then(Value::as_str)?;
    match status_type {
        "active" => {
            let flags = value
                .get("activeFlags")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if flags.contains(&"waitingOnApproval") {
                Some("waiting_approval".to_string())
            } else if flags.contains(&"waitingOnUserInput") {
                Some("waiting_input".to_string())
            } else {
                Some("running".to_string())
            }
        }
        "systemError" => Some("failed".to_string()),
        "idle" | "notLoaded" => None,
        _ => None,
    }
}

fn supplement_thread_metadata_from_local_sessions(threads: &mut Vec<ThreadMetadata>) {
    for home in codex_homes() {
        let _ = supplement_thread_metadata_from_home(threads, &home);
    }
    threads.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn supplement_thread_metadata_from_home(
    threads: &mut Vec<ThreadMetadata>,
    home: &Path,
) -> io::Result<usize> {
    let index = read_local_session_index(&home.join("session_index.jsonl"))?;
    let mut known = threads
        .iter()
        .enumerate()
        .map(|(index, thread)| (thread.id.clone(), index))
        .collect::<std::collections::HashMap<_, _>>();

    for thread in threads.iter_mut() {
        let Some(local) = index.get(&thread.id) else {
            continue;
        };
        if thread.name.is_none() {
            thread.name = local.name.clone();
        }
        if let Some(updated_at) = local.updated_at {
            thread.updated_at = thread.updated_at.max(updated_at);
        }
    }

    let mut paths = Vec::new();
    collect_session_paths(&home.join("sessions"), &mut paths)?;
    let mut added = 0;
    for path in paths {
        if let Some(session_id) = session_id_from_path(&path)
            && let Some(existing) = known.get(session_id).copied()
        {
            if threads[existing].path.is_none() {
                threads[existing].path = Some(path.to_string_lossy().to_string());
            }
            continue;
        }
        let Some(local) = read_local_session_metadata(&path)? else {
            continue;
        };
        if !matches!(local.source.as_str(), "cli" | "vscode") {
            continue;
        }
        if let Some(existing) = known.get(&local.id).copied() {
            if threads[existing].path.is_none() {
                threads[existing].path = Some(path.to_string_lossy().to_string());
            }
            continue;
        }

        let indexed = index.get(&local.id);
        let modified_at = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64);
        let updated_at = indexed
            .and_then(|entry| entry.updated_at)
            .into_iter()
            .chain(modified_at)
            .max()
            .unwrap_or(local.created_at);
        let position = threads.len();
        threads.push(ThreadMetadata {
            id: local.id.clone(),
            name: indexed.and_then(|entry| entry.name.clone()),
            cwd: local.cwd,
            status: None,
            path: Some(path.to_string_lossy().to_string()),
            created_at: local.created_at,
            updated_at,
            parent_thread_id: local.parent_thread_id,
        });
        known.insert(local.id, position);
        added += 1;
    }
    Ok(added)
}

fn codex_homes() -> Vec<PathBuf> {
    if let Some(configured) = env::var_os("CODEX_HOME") {
        let homes = configured
            .to_string_lossy()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if !homes.is_empty() {
            return homes;
        }
    }
    env::var_os("HOME")
        .map(|home| vec![PathBuf::from(home).join(".codex")])
        .unwrap_or_default()
}

fn read_local_session_index(
    path: &Path,
) -> io::Result<std::collections::HashMap<String, LocalSessionIndexEntry>> {
    if !path.is_file() {
        return Ok(std::collections::HashMap::new());
    }
    let mut output = std::collections::HashMap::new();
    for line in BufReader::new(File::open(path)?)
        .lines()
        .map_while(Result::ok)
    {
        let Ok(entry) = serde_json::from_str::<RawSessionIndexEntry>(&line) else {
            continue;
        };
        let name = entry
            .thread_name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        let updated_at = entry
            .updated_at
            .as_deref()
            .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
            .map(|timestamp| timestamp.timestamp());
        output.insert(entry.id, LocalSessionIndexEntry { name, updated_at });
    }
    Ok(output)
}

fn collect_session_paths(directory: &Path, output: &mut Vec<PathBuf>) -> io::Result<()> {
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_session_paths(&path, output)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
            output.push(path);
        }
    }
    Ok(())
}

fn session_id_from_path(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let session_id = stem.get(stem.len().checked_sub(36)?..)?;
    let bytes = session_id.as_bytes();
    if bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|position| bytes[position] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
    {
        Some(session_id)
    } else {
        None
    }
}

fn read_local_session_metadata(path: &Path) -> io::Result<Option<LocalSessionMetadata>> {
    let reader = BufReader::new(File::open(path)?);
    for line in reader.lines().take(8).map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            return Ok(None);
        };
        let Some(id) = payload.get("id").and_then(Value::as_str) else {
            return Ok(None);
        };
        let Some(cwd) = payload.get("cwd").and_then(Value::as_str) else {
            return Ok(None);
        };
        let source = payload
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let created_at = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
            .map(|timestamp| timestamp.timestamp())
            .unwrap_or_default();
        return Ok(Some(LocalSessionMetadata {
            id: id.to_string(),
            cwd: cwd.to_string(),
            source,
            created_at,
            parent_thread_id: payload
                .get("parent_thread_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        }));
    }
    Ok(None)
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn discover_codex_binary() -> Result<PathBuf, CodexError> {
    let mut candidates = Vec::new();
    if let Ok(path) = env::var("CODEX_BIN")
        && !path.trim().is_empty()
    {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("codex"),
        PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"),
        PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
    ]);

    candidates
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
        .ok_or(CodexError::CodexNotFound)
}

fn read_codex_version(path: &Path) -> Result<String, CodexError> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| CodexError::Spawn(error.to_string()))?;
    if !output.status.success() {
        return Err(CodexError::Spawn(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn parse_account(value: Value) -> Result<Option<AccountInfo>, CodexError> {
    let response: RawAccountResponse =
        serde_json::from_value(value).map_err(|error| CodexError::InvalidResponse {
            method: "account/read".to_string(),
            message: error.to_string(),
        })?;

    Ok(response.account.map(|account| AccountInfo {
        account_type: account.account_type,
        plan_type: account.plan_type,
    }))
}

fn parse_rate_limits(value: Value) -> Result<(Option<String>, Vec<RateLimit>), CodexError> {
    let response: RawRateLimitResponse =
        serde_json::from_value(value).map_err(|error| CodexError::InvalidResponse {
            method: "account/rateLimits/read".to_string(),
            message: error.to_string(),
        })?;

    let current_limit_id = response.rate_limits.limit_id.clone();
    let mut normalized = BTreeMap::new();

    if let Some(buckets) = response.rate_limits_by_limit_id {
        for (key, raw) in buckets {
            normalized.insert(key.clone(), normalize_rate_limit(raw, key));
        }
    }

    let fallback_id = response
        .rate_limits
        .limit_id
        .clone()
        .unwrap_or_else(|| "codex".to_string());
    normalized
        .entry(fallback_id.clone())
        .or_insert_with(|| normalize_rate_limit(response.rate_limits, fallback_id));

    let mut limits = Vec::with_capacity(normalized.len());
    if let Some(current) = current_limit_id
        .as_ref()
        .and_then(|limit_id| normalized.remove(limit_id))
    {
        limits.push(current);
    }
    limits.extend(normalized.into_values());

    Ok((current_limit_id, limits))
}

fn normalize_rate_limit(raw: RawRateLimit, fallback_id: String) -> RateLimit {
    RateLimit {
        limit_id: raw.limit_id.unwrap_or(fallback_id),
        limit_name: raw.limit_name,
        plan_type: raw.plan_type,
        primary: raw.primary,
        secondary: raw.secondary,
        credits: raw.credits,
        spend_control_reached: raw.spend_control_reached,
        rate_limit_reached_type: raw.rate_limit_reached_type,
    }
}

fn parse_usage(value: Value) -> Result<(Option<UsageSummary>, Vec<DailyUsage>), CodexError> {
    let response: RawUsageResponse =
        serde_json::from_value(value).map_err(|error| CodexError::InvalidResponse {
            method: "account/usage/read".to_string(),
            message: error.to_string(),
        })?;

    let summary = response.summary.map(|summary| UsageSummary {
        lifetime_tokens: summary.lifetime_tokens,
        peak_daily_tokens: summary.peak_daily_tokens,
        longest_running_turn_sec: summary.longest_running_turn_sec,
        current_streak_days: summary.current_streak_days,
        longest_streak_days: summary.longest_streak_days,
    });
    let daily_usage = response
        .daily_usage_buckets
        .unwrap_or_default()
        .into_iter()
        .map(|bucket| DailyUsage {
            start_date: bucket.start_date,
            tokens: bucket.tokens,
        })
        .collect();

    Ok((summary, daily_usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multi_bucket_rate_limits_and_keeps_current_first() {
        let payload = json!({
            "rateLimits": {
                "limitId": "codex",
                "limitName": null,
                "primary": null,
                "secondary": null,
                "credits": {
                    "hasCredits": true,
                    "unlimited": true,
                    "balance": null
                },
                "planType": "business"
            },
            "rateLimitsByLimitId": {
                "codex_spark": {
                    "limitId": "codex_spark",
                    "limitName": "Spark",
                    "primary": {
                        "usedPercent": 18.5,
                        "windowDurationMins": 300,
                        "resetsAt": 1_800_000_000
                    },
                    "secondary": {
                        "usedPercent": 42,
                        "windowDurationMins": 10080,
                        "resetsAt": 1_800_500_000
                    },
                    "credits": null
                },
                "codex": {
                    "limitId": "codex",
                    "limitName": null,
                    "primary": null,
                    "secondary": null,
                    "credits": {
                        "hasCredits": true,
                        "unlimited": true,
                        "balance": null
                    },
                    "planType": "business"
                }
            },
            "rateLimitResetCredits": null
        });

        let (current, limits) = parse_rate_limits(payload).expect("valid limits");
        assert_eq!(current.as_deref(), Some("codex"));
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].limit_id, "codex");
        assert!(
            limits[0]
                .credits
                .as_ref()
                .is_some_and(|value| value.unlimited)
        );
        assert_eq!(
            limits[1].primary.as_ref().map(|value| value.used_percent),
            Some(18.5)
        );
    }

    #[test]
    fn parses_usage_without_losing_large_token_counts() {
        let payload = json!({
            "summary": {
                "lifetimeTokens": 23_918_079_313_u64,
                "peakDailyTokens": 932_096_397_u64,
                "longestRunningTurnSec": 35_263,
                "currentStreakDays": 6,
                "longestStreakDays": 54
            },
            "dailyUsageBuckets": [
                { "startDate": "2026-07-25", "tokens": 6_101_106 }
            ]
        });

        let (summary, daily) = parse_usage(payload).expect("valid usage");
        assert_eq!(
            summary.and_then(|value| value.lifetime_tokens),
            Some(23_918_079_313)
        );
        assert_eq!(daily[0].tokens, 6_101_106);
    }

    #[test]
    fn account_parser_never_keeps_email() {
        let payload = json!({
            "account": {
                "type": "chatgpt",
                "email": "private@example.com",
                "planType": "business"
            },
            "requiresOpenaiAuth": true
        });

        let account = parse_account(payload)
            .expect("valid account")
            .expect("account exists");
        assert_eq!(account.account_type, "chatgpt");
        assert_eq!(account.plan_type.as_deref(), Some("business"));
    }

    #[test]
    fn maps_official_thread_wait_states() {
        assert_eq!(
            normalize_thread_status(&json!({
                "type": "active",
                "activeFlags": ["waitingOnApproval"]
            }))
            .as_deref(),
            Some("waiting_approval")
        );
        assert_eq!(
            normalize_thread_status(&json!({
                "type": "active",
                "activeFlags": ["waitingOnUserInput"]
            }))
            .as_deref(),
            Some("waiting_input")
        );
        assert_eq!(normalize_thread_status(&json!({ "type": "idle" })), None);
    }

    #[test]
    fn supplements_threads_missing_from_app_server_with_active_session_files() {
        const MISSING_ID: &str = "019fb337-e786-7800-bd0d-fbcd46e0f003";
        let home = std::env::temp_dir().join(format!(
            "codex-xray-thread-catalog-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let sessions = home.join("sessions/2026/07/30");
        fs::create_dir_all(&sessions).expect("create sessions");
        fs::write(
            home.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"existing\",\"thread_name\":\"Existing title\",\"updated_at\":\"2026-07-30T12:00:00Z\"}}\n\
                 {{\"id\":\"{MISSING_ID}\",\"thread_name\":\"测试下\",\"updated_at\":\"2026-07-30T13:30:22Z\"}}\n"
            ),
        )
        .expect("write session index");
        let missing_path = sessions.join(format!("rollout-2026-07-30T21-30-13-{MISSING_ID}.jsonl"));
        fs::write(
            &missing_path,
            format!(
                "{{\"timestamp\":\"2026-07-30T13:30:13Z\",\"type\":\"session_meta\",\
                 \"payload\":{{\"id\":\"{MISSING_ID}\",\"cwd\":\"/Users/test/Documents/副业\",\
                 \"source\":\"vscode\",\"parent_thread_id\":null}}}}\n"
            ),
        )
        .expect("write missing session");
        assert_eq!(session_id_from_path(&missing_path), Some(MISSING_ID));

        let mut threads = vec![ThreadMetadata {
            id: "existing".to_string(),
            name: None,
            cwd: "/tmp/existing".to_string(),
            status: None,
            path: None,
            created_at: 1,
            updated_at: 1,
            parent_thread_id: None,
        }];
        let added =
            supplement_thread_metadata_from_home(&mut threads, &home).expect("supplement catalog");

        assert_eq!(added, 1);
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].name.as_deref(), Some("Existing title"));
        let missing = threads
            .iter()
            .find(|thread| thread.id == MISSING_ID)
            .expect("missing thread was added");
        assert_eq!(missing.name.as_deref(), Some("测试下"));
        assert_eq!(missing.cwd, "/Users/test/Documents/副业");
        assert!(
            missing
                .path
                .as_deref()
                .is_some_and(|path| path.ends_with(&format!("{MISSING_ID}.jsonl")))
        );

        fs::remove_dir_all(home).expect("cleanup");
    }
}
