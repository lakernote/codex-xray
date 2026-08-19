use std::collections::{HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use chrono::Utc;
use qrcode::QrCode;
use qrcode::render::svg;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::codex::{AppServerClient, ThreadMetadata};
use crate::desktop_ipc::{
    DesktopIpcClient, DesktopPendingRequest, DesktopThreadView, DesktopTurnOutcome,
};
use crate::trace_analysis::sanitize_text;

const WEIXIN_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const WEIXIN_CHANNEL_VERSION: &str = "2.4.6";
const WEIXIN_CLIENT_VERSION: u32 = (2 << 16) | (4 << 8) | 6;
const SESSION_EXPIRED_ERRCODE: i64 = -14;
const MAX_SEEN_MESSAGES: usize = 256;
const MAX_AGENT_PREVIEW_CHARS: usize = 8_000;
const MAX_WEIXIN_CHUNK_CHARS: usize = 3_500;
const WEIXIN_TASK_LIST_LIMIT: usize = 12;
const ACTIVE_SESSION_WINDOW: Duration = Duration::from_secs(5 * 60);
const SESSION_TAIL_BYTES: u64 = 8 * 1024 * 1024;
const PROGRESS_FIRST_FEEDBACK: Duration = Duration::from_secs(10);
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_secs(20);
const PROGRESS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const DESKTOP_IPC_PROBE_ATTEMPTS: usize = 2;
const DESKTOP_WAKE_RETRY_DELAYS_MS: [u64; 6] = [350, 650, 1_000, 1_500, 2_200, 3_000];
static CLIENT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
pub struct RemoteChannelSnapshot {
    pub enabled: bool,
    pub state: String,
    pub account_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub qr_svg: Option<String>,
    pub verify_code_required: bool,
    pub attached_thread_id: Option<String>,
    pub attached_thread_title: Option<String>,
    pub attached_cwd: Option<String>,
    pub control_ready: bool,
    pub control_backend: String,
    pub active_turn_id: Option<String>,
    pub latest_activity: Option<String>,
    pub agent_preview: Option<String>,
    pub pending_approval: Option<RemoteApprovalSnapshot>,
    pub last_inbound_at: Option<String>,
    pub last_outbound_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteApprovalSnapshot {
    pub kind: String,
    pub summary: String,
    pub requested_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteTaskSummary {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub status: String,
    pub updated_at: i64,
    pub control_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RemoteChannelFile {
    version: u32,
    #[serde(default)]
    enabled: bool,
    account_id: Option<String>,
    owner_user_id: Option<String>,
    #[serde(default = "default_base_url")]
    base_url: String,
    get_updates_buf: Option<String>,
    attached_thread_id: Option<String>,
    attached_thread_title: Option<String>,
    attached_cwd: Option<String>,
    #[serde(default)]
    pending_thread_id: Option<String>,
    #[serde(default)]
    pending_turn_id: Option<String>,
    #[serde(default)]
    pending_reply_user_id: Option<String>,
    #[serde(default)]
    pending_reply_context_token: Option<String>,
}

impl Default for RemoteChannelFile {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: false,
            account_id: None,
            owner_user_id: None,
            base_url: default_base_url(),
            get_updates_buf: None,
            attached_thread_id: None,
            attached_thread_title: None,
            attached_cwd: None,
            pending_thread_id: None,
            pending_turn_id: None,
            pending_reply_user_id: None,
            pending_reply_context_token: None,
        }
    }
}

#[derive(Debug, Clone)]
struct ActiveLogin {
    qrcode: String,
    current_base_url: String,
    qr_svg: String,
    verify_code: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingApproval {
    request_id: Value,
    method: String,
    snapshot: RemoteApprovalSnapshot,
}

#[derive(Debug, Clone)]
struct ReplyTarget {
    user_id: String,
    context_token: Option<String>,
}

#[derive(Debug)]
struct ProgressFeedback {
    started_at: Instant,
    last_sent_at: Option<Instant>,
    last_signature: String,
}

impl ProgressFeedback {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            last_sent_at: None,
            last_signature: String::new(),
        }
    }

    fn should_send(&self, now: Instant, signature: &str) -> bool {
        if now.duration_since(self.started_at) < PROGRESS_FIRST_FEEDBACK {
            return false;
        }
        let Some(last_sent_at) = self.last_sent_at else {
            return true;
        };
        let since_last = now.duration_since(last_sent_at);
        since_last >= PROGRESS_HEARTBEAT_INTERVAL
            || (since_last >= PROGRESS_MIN_INTERVAL && signature != self.last_signature)
    }

    fn mark_sent(&mut self, now: Instant, signature: String) {
        self.last_sent_at = Some(now);
        self.last_signature = signature;
    }
}

#[derive(Debug)]
struct RemoteChannelRuntime {
    enabled: bool,
    state: String,
    account_id: Option<String>,
    owner_user_id: Option<String>,
    base_url: String,
    token: Option<String>,
    get_updates_buf: Option<String>,
    active_login: Option<ActiveLogin>,
    verify_code_required: bool,
    attached_thread_id: Option<String>,
    attached_thread_title: Option<String>,
    attached_cwd: Option<String>,
    control_ready: bool,
    control_backend: String,
    desktop_owner_client_id: Option<String>,
    active_turn_id: Option<String>,
    pending_thread_id: Option<String>,
    pending_turn_id: Option<String>,
    pending_reply_user_id: Option<String>,
    pending_reply_context_token: Option<String>,
    latest_activity: Option<String>,
    agent_preview: String,
    pending_approval: Option<PendingApproval>,
    last_context_token: Option<String>,
    last_inbound_at: Option<String>,
    last_outbound_at: Option<String>,
    last_error: Option<String>,
    task_choices: Vec<String>,
    task_selection_active: bool,
    seen_messages: VecDeque<String>,
}

#[derive(Clone)]
pub struct RemoteChannelState {
    runtime: Arc<RwLock<RemoteChannelRuntime>>,
    worker: Arc<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    config_path: PathBuf,
    token_path: PathBuf,
    client: reqwest::Client,
    codex_client: Arc<Mutex<Option<AppServerClient>>>,
    desktop_ipc: Arc<Mutex<Option<DesktopIpcClient>>>,
    codex_state_dir: PathBuf,
}

impl RemoteChannelState {
    pub fn load(
        app_data_dir: &Path,
        codex_client: Arc<Mutex<Option<AppServerClient>>>,
        codex_state_dir: PathBuf,
    ) -> Result<Self, String> {
        let config_path = app_data_dir.join("remote-channel.json");
        let token_path = app_data_dir.join("remote-channel-weixin.token");
        let config = read_config(&config_path)?;
        let token = read_secret(&token_path).ok();
        let pending_turn_id = config.pending_turn_id.clone();
        let state = if config.enabled && token.is_some() {
            "connected"
        } else if token.is_some() {
            "stopped"
        } else {
            "login_required"
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| format!("无法初始化微信 HTTP 客户端：{error}"))?;
        Ok(Self {
            runtime: Arc::new(RwLock::new(RemoteChannelRuntime {
                enabled: config.enabled && token.is_some(),
                state: state.to_string(),
                account_id: config.account_id,
                owner_user_id: config.owner_user_id,
                base_url: normalize_weixin_base_url(&config.base_url)
                    .unwrap_or_else(|_| default_base_url()),
                token,
                get_updates_buf: config.get_updates_buf,
                active_login: None,
                verify_code_required: false,
                attached_thread_id: config.attached_thread_id,
                attached_thread_title: config.attached_thread_title,
                attached_cwd: config.attached_cwd,
                // A previous App Server process cannot be treated as still attached after X-Ray
                // restarts. The first remote instruction will safely resume an idle thread.
                control_ready: false,
                control_backend: "none".to_string(),
                desktop_owner_client_id: None,
                active_turn_id: pending_turn_id,
                pending_thread_id: config.pending_thread_id,
                pending_turn_id: config.pending_turn_id,
                pending_reply_user_id: config.pending_reply_user_id,
                pending_reply_context_token: config.pending_reply_context_token.clone(),
                latest_activity: None,
                agent_preview: String::new(),
                pending_approval: None,
                last_context_token: config.pending_reply_context_token,
                last_inbound_at: None,
                last_outbound_at: None,
                last_error: None,
                task_choices: Vec::new(),
                task_selection_active: false,
                seen_messages: VecDeque::new(),
            })),
            worker: Arc::new(Mutex::new(None)),
            config_path,
            token_path,
            client,
            codex_client,
            desktop_ipc: Arc::new(Mutex::new(None)),
            codex_state_dir,
        })
    }

    pub fn snapshot(&self) -> RemoteChannelSnapshot {
        let runtime = self.runtime.read().expect("remote channel runtime lock");
        RemoteChannelSnapshot {
            enabled: runtime.enabled,
            state: runtime.state.clone(),
            account_id: runtime.account_id.clone(),
            owner_user_id: runtime.owner_user_id.clone(),
            qr_svg: runtime
                .active_login
                .as_ref()
                .map(|login| login.qr_svg.clone()),
            verify_code_required: runtime.verify_code_required,
            attached_thread_id: runtime.attached_thread_id.clone(),
            attached_thread_title: runtime.attached_thread_title.clone(),
            attached_cwd: runtime.attached_cwd.clone(),
            control_ready: runtime.control_ready,
            control_backend: runtime.control_backend.clone(),
            active_turn_id: runtime.active_turn_id.clone(),
            latest_activity: runtime.latest_activity.clone(),
            agent_preview: (!runtime.agent_preview.is_empty())
                .then(|| runtime.agent_preview.clone()),
            pending_approval: runtime
                .pending_approval
                .as_ref()
                .map(|approval| approval.snapshot.clone()),
            last_inbound_at: runtime.last_inbound_at.clone(),
            last_outbound_at: runtime.last_outbound_at.clone(),
            last_error: runtime.last_error.clone(),
        }
    }

    pub fn start_if_enabled(&self) {
        let (enabled, attached_thread_id) = self
            .runtime
            .read()
            .map(|runtime| {
                (
                    runtime.enabled && runtime.token.is_some(),
                    runtime.attached_thread_id.clone(),
                )
            })
            .unwrap_or((false, None));
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(thread_id) = attached_thread_id
                && let Err(error) = state.restore_saved_attachment(thread_id).await
                && let Ok(mut runtime) = state.runtime.write()
            {
                runtime.control_ready = false;
                runtime.control_backend = "none".to_string();
                runtime.desktop_owner_client_id = None;
                if is_occupied_error(&error) {
                    runtime.latest_activity = Some(occupied_activity());
                    runtime.last_error = None;
                } else {
                    runtime.latest_activity = Some("恢复控制目标失败".to_string());
                    runtime.last_error = Some(error);
                }
            }
            if enabled {
                state.resume_pending_monitor();
                state.spawn_poll_loop();
            }
        });
    }

    fn resume_pending_monitor(&self) {
        let pending = self.runtime.read().ok().and_then(|runtime| {
            Some((
                runtime.pending_thread_id.clone()?,
                runtime.pending_turn_id.clone()?,
                runtime.control_backend.clone(),
                ReplyTarget {
                    user_id: runtime.pending_reply_user_id.clone()?,
                    context_token: runtime.pending_reply_context_token.clone(),
                },
            ))
        });
        let Some((thread_id, turn_id, backend, target)) = pending else {
            return;
        };
        if let Ok(mut runtime) = self.runtime.write() {
            runtime.active_turn_id = Some(turn_id.clone());
            runtime.latest_activity = Some("正在恢复重启前的 Codex 回合".to_string());
        }
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            if backend == "desktop_ipc" {
                state.monitor_desktop_turn(thread_id, turn_id, target).await;
            } else {
                state.monitor_turn(thread_id, turn_id, target).await;
            }
        });
    }

    async fn restore_saved_attachment(&self, thread_id: String) -> Result<(), String> {
        self.attach_thread(&thread_id).await.map(|_| ())
    }

    pub async fn start_login(&self) -> Result<RemoteChannelSnapshot, String> {
        self.abort_worker();
        let (base_url, local_token) = {
            let runtime = self
                .runtime
                .read()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            (runtime.base_url.clone(), runtime.token.clone())
        };
        let response: WeixinQrStartResponse = self
            .post_json(
                &base_url,
                "ilink/bot/get_bot_qrcode?bot_type=3",
                None,
                json!({
                    "local_token_list": local_token.into_iter().collect::<Vec<_>>()
                }),
                Duration::from_secs(30),
            )
            .await?;
        if response.qrcode.trim().is_empty() || response.qrcode_img_content.trim().is_empty() {
            return Err("微信登录接口没有返回完整二维码。".to_string());
        }
        let qr_svg = render_qr_svg(&response.qrcode_img_content)?;
        {
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            runtime.enabled = false;
            runtime.state = "login_required".to_string();
            runtime.active_login = Some(ActiveLogin {
                qrcode: response.qrcode,
                current_base_url: base_url,
                qr_svg,
                verify_code: None,
            });
            runtime.verify_code_required = false;
            runtime.last_error = None;
        }
        self.persist()?;
        Ok(self.snapshot())
    }

    pub async fn poll_login(
        &self,
        verify_code: Option<String>,
    ) -> Result<RemoteChannelSnapshot, String> {
        let (qrcode, base_url, current_verify_code) = {
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            let login = runtime
                .active_login
                .as_mut()
                .ok_or_else(|| "当前没有进行中的微信登录。".to_string())?;
            if let Some(code) = verify_code
                .map(|code| code.trim().to_string())
                .filter(|code| !code.is_empty())
            {
                login.verify_code = Some(code);
            }
            (
                login.qrcode.clone(),
                login.current_base_url.clone(),
                login.verify_code.clone(),
            )
        };
        let mut endpoint = format!(
            "ilink/bot/get_qrcode_status?qrcode={}",
            url::form_urlencoded::byte_serialize(qrcode.as_bytes()).collect::<String>()
        );
        if let Some(code) = current_verify_code {
            endpoint.push_str("&verify_code=");
            endpoint.extend(url::form_urlencoded::byte_serialize(code.as_bytes()));
        }
        let response: WeixinQrStatusResponse = self
            .get_json(&base_url, &endpoint, Duration::from_secs(35))
            .await?;
        if response.status != "need_verifycode"
            && let Ok(mut runtime) = self.runtime.write()
        {
            runtime.verify_code_required = false;
        }
        match response.status.as_str() {
            "confirmed" => {
                let token = response
                    .bot_token
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "微信确认登录后没有返回 bot_token。".to_string())?;
                let account_id = response
                    .ilink_bot_id
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "微信确认登录后没有返回 ilink_bot_id。".to_string())?;
                let confirmed_base_url = response
                    .baseurl
                    .as_deref()
                    .map(normalize_weixin_base_url)
                    .transpose()?
                    .unwrap_or(base_url);
                save_secret(&self.token_path, &token)?;
                {
                    let mut runtime = self
                        .runtime
                        .write()
                        .map_err(|_| "微信通道状态已损坏".to_string())?;
                    runtime.enabled = true;
                    runtime.state = "connected".to_string();
                    runtime.account_id = Some(normalize_identifier(&account_id));
                    runtime.owner_user_id = response.ilink_user_id;
                    runtime.base_url = confirmed_base_url;
                    runtime.token = Some(token);
                    runtime.get_updates_buf = None;
                    runtime.active_login = None;
                    runtime.verify_code_required = false;
                    runtime.last_error = None;
                }
                self.persist()?;
                self.spawn_poll_loop();
            }
            "scaned_but_redirect" => {
                if let Some(host) = response.redirect_host {
                    let redirected = normalize_weixin_base_url(&format!("https://{host}"))?;
                    if let Ok(mut runtime) = self.runtime.write()
                        && let Some(login) = runtime.active_login.as_mut()
                    {
                        login.current_base_url = redirected;
                    }
                }
            }
            "need_verifycode" => {
                if let Ok(mut runtime) = self.runtime.write() {
                    runtime.verify_code_required = true;
                    runtime.last_error = Some("请输入手机微信显示的配对数字。".to_string());
                }
            }
            "expired" | "verify_code_blocked" => {
                if let Ok(mut runtime) = self.runtime.write() {
                    runtime.active_login = None;
                    runtime.verify_code_required = false;
                    runtime.last_error = Some(format!("微信登录未完成：{}", response.status));
                }
            }
            "binded_redirect" => {
                if let Ok(mut runtime) = self.runtime.write() {
                    runtime.active_login = None;
                    runtime.verify_code_required = false;
                    runtime.last_error =
                        Some("该微信账号已经绑定，请直接启用已保存连接。".to_string());
                }
            }
            _ => {}
        }
        Ok(self.snapshot())
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<RemoteChannelSnapshot, String> {
        if enabled {
            {
                let mut runtime = self
                    .runtime
                    .write()
                    .map_err(|_| "微信通道状态已损坏".to_string())?;
                if runtime.token.is_none() {
                    return Err("请先扫码登录微信。".to_string());
                }
                runtime.enabled = true;
                runtime.state = "connected".to_string();
                runtime.last_error = None;
            }
            self.persist()?;
            self.spawn_poll_loop();
        } else {
            self.abort_worker();
            {
                let mut runtime = self
                    .runtime
                    .write()
                    .map_err(|_| "微信通道状态已损坏".to_string())?;
                runtime.enabled = false;
                runtime.state = "stopped".to_string();
                runtime.pending_approval = None;
            }
            self.persist()?;
        }
        Ok(self.snapshot())
    }

    pub fn disconnect(&self) -> Result<RemoteChannelSnapshot, String> {
        self.abort_worker();
        match fs::remove_file(&self.token_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("无法删除微信登录令牌：{error}")),
        }
        {
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            runtime.enabled = false;
            runtime.state = "login_required".to_string();
            runtime.account_id = None;
            runtime.owner_user_id = None;
            runtime.token = None;
            runtime.get_updates_buf = None;
            runtime.active_login = None;
            runtime.verify_code_required = false;
            runtime.pending_approval = None;
            runtime.last_error = None;
        }
        self.persist()?;
        Ok(self.snapshot())
    }

    pub async fn tasks(&self) -> Result<Vec<RemoteTaskSummary>, String> {
        let client = Arc::clone(&self.codex_client);
        let state_dir = self.codex_state_dir.clone();
        tauri::async_runtime::spawn_blocking(move || {
            with_codex_client(&client, &state_dir, |client| {
                let threads = client
                    .fetch_thread_metadata()
                    .map_err(|error| error.to_string())?;
                Ok(summarize_threads(threads))
            })
        })
        .await
        .map_err(|error| format!("读取 Codex 任务的后台操作失败：{error}"))?
    }

    async fn probe_desktop_control(
        &self,
        thread_id: &str,
        previous_thread_id: Option<&str>,
    ) -> Result<Option<(String, DesktopThreadView)>, String> {
        let desktop = Arc::clone(&self.desktop_ipc);
        let thread_id = thread_id.to_string();
        let previous_thread_id = previous_thread_id.map(ToOwned::to_owned);
        tauri::async_runtime::spawn_blocking(move || {
            let mut guard = desktop
                .lock()
                .map_err(|_| "Codex Desktop IPC 状态已损坏".to_string())?;
            probe_desktop_thread(&mut guard, &thread_id, previous_thread_id.as_deref())
        })
        .await
        .map_err(|error| format!("检测 Codex Desktop 原任务失败：{error}"))?
    }

    pub async fn attach_thread(&self, requested_id: &str) -> Result<RemoteChannelSnapshot, String> {
        let requested_id = requested_id.trim().to_string();
        if requested_id.is_empty() {
            return Err("任务 ID 不能为空。".to_string());
        }
        let tasks = self.tasks().await?;
        let choices = self
            .runtime
            .read()
            .map(|runtime| runtime.task_choices.clone())
            .unwrap_or_default();
        let resolved_id = requested_id
            .parse::<usize>()
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| choices.get(index).cloned())
            .unwrap_or(requested_id);
        let matches = tasks
            .iter()
            .filter(|task| task.id == resolved_id || task.id.starts_with(&resolved_id))
            .collect::<Vec<_>>();
        let task = match matches.as_slice() {
            [task] => (*task).clone(),
            [] => return Err("没有找到匹配的 Codex 任务。".to_string()),
            _ => return Err("任务 ID 前缀不唯一，请提供更多字符。".to_string()),
        };
        let already_controlled = self
            .runtime
            .read()
            .map(|runtime| {
                runtime.control_ready
                    && runtime.attached_thread_id.as_deref() == Some(task.id.as_str())
            })
            .unwrap_or(false);
        if already_controlled {
            return Ok(self.snapshot());
        }
        let (previous_thread_id, previous_backend) = self
            .runtime
            .read()
            .map(|runtime| {
                (
                    runtime.attached_thread_id.clone(),
                    runtime.control_backend.clone(),
                )
            })
            .unwrap_or_default();
        let desktop_thread_id = task.id.clone();
        let previous_desktop_thread = (previous_backend == "desktop_ipc")
            .then_some(previous_thread_id)
            .flatten();
        {
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            runtime.attached_thread_id = Some(task.id.clone());
            runtime.attached_thread_title = Some(task.title.clone());
            runtime.attached_cwd = Some(task.cwd.clone());
            runtime.control_ready = false;
            runtime.control_backend = "none".to_string();
            runtime.desktop_owner_client_id = None;
            runtime.active_turn_id = None;
            runtime.agent_preview.clear();
            runtime.pending_approval = None;
            runtime.latest_activity = Some("正在检测 Codex Desktop 原任务…".to_string());
            runtime.last_error = None;
            runtime.task_selection_active = false;
        }
        let initial_probe = self
            .probe_desktop_control(&desktop_thread_id, previous_desktop_thread.as_deref())
            .await;
        let mut desktop_error = None;
        let mut desktop_connection = match initial_probe {
            Ok(connection) => connection,
            Err(error) => {
                desktop_error = Some(error);
                None
            }
        };

        if desktop_connection.is_none() {
            if let Ok(mut runtime) = self.runtime.write() {
                runtime.latest_activity =
                    Some("正在后台唤醒 Codex Desktop 中的原任务…".to_string());
            }

            match launch_codex_thread(&desktop_thread_id) {
                Ok(()) => {
                    for delay_ms in DESKTOP_WAKE_RETRY_DELAYS_MS {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        match self
                            .probe_desktop_control(
                                &desktop_thread_id,
                                previous_desktop_thread.as_deref(),
                            )
                            .await
                        {
                            Ok(Some(connection)) => {
                                desktop_connection = Some(connection);
                                desktop_error = None;
                                break;
                            }
                            Ok(None) => {}
                            Err(error) => desktop_error = Some(error),
                        }
                    }
                }
                Err(error) => desktop_error = Some(error),
            }
        }

        if let Some((owner, view)) = desktop_connection {
            let pending_approval = view.pending_request.as_ref().map(desktop_pending_approval);
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            runtime.attached_thread_id = Some(task.id);
            runtime.attached_thread_title = Some(task.title);
            runtime.attached_cwd = Some(task.cwd);
            runtime.control_ready = true;
            runtime.control_backend = "desktop_ipc".to_string();
            runtime.desktop_owner_client_id = Some(owner);
            runtime.active_turn_id = view.active_turn_id;
            runtime.agent_preview = view.agent_preview.unwrap_or_default();
            runtime.pending_approval = pending_approval;
            runtime.latest_activity = Some("已自动连接 Codex Desktop 中的原任务".to_string());
            runtime.last_error = None;
            runtime.task_selection_active = false;
            drop(runtime);
            self.persist()?;
            return Ok(self.snapshot());
        }
        let active_elsewhere = matches!(
            task.status.as_str(),
            "running" | "waiting_approval" | "waiting_input"
        );
        if active_elsewhere {
            let failure = desktop_control_unavailable_message(desktop_error.as_deref());
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            runtime.attached_thread_id = Some(task.id);
            runtime.attached_thread_title = Some(task.title);
            runtime.attached_cwd = Some(task.cwd);
            runtime.control_ready = false;
            runtime.control_backend = "none".to_string();
            runtime.desktop_owner_client_id = None;
            runtime.latest_activity = Some(occupied_activity());
            runtime.last_error = Some(failure);
            runtime.task_selection_active = false;
            drop(runtime);
            self.persist()?;
            return Ok(self.snapshot());
        }

        // No Desktop owner claimed this idle task. It is safe to resume it through the
        // X-Ray-owned App Server. An active task never reaches this fallback.
        let thread_id = task.id.clone();
        let client = Arc::clone(&self.codex_client);
        let state_dir = self.codex_state_dir.clone();
        let attach_result = tauri::async_runtime::spawn_blocking(move || {
            with_codex_client(&client, &state_dir, |client| {
                client
                    .attach_remote_thread(&thread_id)
                    .map_err(|error| error.to_string())
            })
        })
        .await
        .map_err(|error| format!("接管 Codex 任务的后台操作失败：{error}"))?;
        let attached_thread_id = match attach_result {
            Ok(thread_id) => thread_id,
            Err(error) if is_active_writer_conflict(&error) => {
                let failure = desktop_control_unavailable_message(desktop_error.as_deref());
                let mut runtime = self
                    .runtime
                    .write()
                    .map_err(|_| "微信通道状态已损坏".to_string())?;
                runtime.attached_thread_id = Some(task.id);
                runtime.attached_thread_title = Some(task.title);
                runtime.attached_cwd = Some(task.cwd);
                runtime.control_ready = false;
                runtime.control_backend = "none".to_string();
                runtime.desktop_owner_client_id = None;
                runtime.latest_activity = Some(occupied_activity());
                runtime.last_error = Some(failure);
                runtime.task_selection_active = false;
                drop(runtime);
                self.persist()?;
                return Ok(self.snapshot());
            }
            Err(error) => return Err(attach_error_message(&error)),
        };
        {
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            runtime.attached_thread_id = Some(attached_thread_id);
            runtime.attached_thread_title = Some(task.title);
            runtime.attached_cwd = Some(task.cwd);
            runtime.control_ready = true;
            runtime.control_backend = "xray_app_server".to_string();
            runtime.desktop_owner_client_id = None;
            runtime.active_turn_id = None;
            runtime.agent_preview.clear();
            runtime.pending_approval = None;
            runtime.latest_activity = Some("已由 X-Ray 独立 App Server 接管原任务".to_string());
            runtime.last_error = None;
            runtime.task_selection_active = false;
        }
        self.persist()?;
        Ok(self.snapshot())
    }

    pub async fn interrupt_active_turn(&self) -> Result<RemoteChannelSnapshot, String> {
        let (thread_id, turn_id, backend, desktop_owner) = {
            let runtime = self
                .runtime
                .read()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            (
                runtime
                    .attached_thread_id
                    .clone()
                    .ok_or_else(|| "当前没有已接管的任务。".to_string())?,
                runtime
                    .active_turn_id
                    .clone()
                    .ok_or_else(|| "当前任务没有正在执行的回合。".to_string())?,
                runtime.control_backend.clone(),
                runtime.desktop_owner_client_id.clone(),
            )
        };
        if backend == "desktop_ipc" {
            let owner =
                desktop_owner.ok_or_else(|| "Codex Desktop 控制 owner 已丢失".to_string())?;
            let desktop = Arc::clone(&self.desktop_ipc);
            tauri::async_runtime::spawn_blocking(move || {
                let mut guard = desktop
                    .lock()
                    .map_err(|_| "Codex Desktop IPC 状态已损坏".to_string())?;
                let result = guard
                    .as_mut()
                    .ok_or_else(|| "Codex Desktop IPC 尚未连接".to_string())
                    .and_then(|client| client.interrupt_turn(&thread_id, &owner, &turn_id));
                if result.is_err() {
                    *guard = None;
                }
                result
            })
            .await
            .map_err(|error| format!("停止 Desktop 原任务的后台操作失败：{error}"))??;
        } else if backend == "xray_app_server" {
            let client = Arc::clone(&self.codex_client);
            let state_dir = self.codex_state_dir.clone();
            tauri::async_runtime::spawn_blocking(move || {
                with_codex_client(&client, &state_dir, |client| {
                    client
                        .interrupt_remote_turn(&thread_id, &turn_id)
                        .map_err(|error| error.to_string())
                })
            })
            .await
            .map_err(|error| format!("停止 Codex 任务的后台操作失败：{error}"))??;
        } else {
            return Err("当前任务尚未建立可控制连接。请在 X-Ray 页面重新选择。".to_string());
        }
        if let Ok(mut runtime) = self.runtime.write() {
            runtime.latest_activity = Some("已请求停止当前回合".to_string());
        }
        Ok(self.snapshot())
    }

    fn spawn_poll_loop(&self) {
        self.abort_worker();
        let account = self.runtime.read().ok().and_then(|runtime| {
            Some(WeixinAccount {
                token: runtime.token.clone()?,
                base_url: runtime.base_url.clone(),
            })
        });
        let Some(account) = account else {
            return;
        };
        let state = self.clone();
        let handle = tauri::async_runtime::spawn(async move {
            state.poll_loop(account).await;
        });
        if let Ok(mut worker) = self.worker.lock() {
            *worker = Some(handle);
        }
    }

    fn abort_worker(&self) {
        if let Ok(mut worker) = self.worker.lock()
            && let Some(handle) = worker.take()
        {
            handle.abort();
        }
    }

    async fn poll_loop(&self, account: WeixinAccount) {
        let _ = self
            .post_json::<Value>(
                &account.base_url,
                "ilink/bot/msg/notifystart",
                Some(&account.token),
                json!({ "base_info": base_info() }),
                Duration::from_secs(10),
            )
            .await;
        loop {
            let (enabled, get_updates_buf) = match self.runtime.read() {
                Ok(runtime) => (runtime.enabled, runtime.get_updates_buf.clone()),
                Err(_) => return,
            };
            if !enabled {
                return;
            }
            let response = self
                .post_json::<WeixinGetUpdatesResponse>(
                    &account.base_url,
                    "ilink/bot/getupdates",
                    Some(&account.token),
                    json!({
                        "get_updates_buf": get_updates_buf.unwrap_or_default(),
                        "base_info": base_info()
                    }),
                    Duration::from_secs(45),
                )
                .await;
            match response {
                Ok(response) => {
                    if response.ret == Some(SESSION_EXPIRED_ERRCODE)
                        || response.errcode == Some(SESSION_EXPIRED_ERRCODE)
                    {
                        if let Ok(mut runtime) = self.runtime.write() {
                            runtime.enabled = false;
                            runtime.state = "login_required".to_string();
                            runtime.last_error = Some("微信登录已过期，请重新扫码。".to_string());
                        }
                        let _ = self.persist();
                        return;
                    }
                    if response.ret.unwrap_or(0) != 0 || response.errcode.unwrap_or(0) != 0 {
                        self.set_degraded(format!(
                            "getupdates 失败：ret={} errcode={} {}",
                            response.ret.unwrap_or(0),
                            response.errcode.unwrap_or(0),
                            response.errmsg.unwrap_or_default()
                        ));
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                    if let Some(buffer) = response.get_updates_buf {
                        if let Ok(mut runtime) = self.runtime.write() {
                            runtime.get_updates_buf = Some(buffer);
                            runtime.state = "connected".to_string();
                            runtime.last_error = None;
                        }
                        let _ = self.persist();
                    }
                    for message in response.msgs.unwrap_or_default() {
                        if let Err(error) = self.handle_inbound(message).await {
                            self.set_degraded(error);
                        }
                    }
                }
                Err(error) => {
                    self.set_degraded(error);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    async fn handle_inbound(&self, message: WeixinMessage) -> Result<(), String> {
        if !is_direct_user_message(&message) {
            return Ok(());
        }
        let sender = message
            .from_user_id
            .clone()
            .filter(|sender| !sender.trim().is_empty())
            .ok_or_else(|| "收到缺少发送者的微信消息。".to_string())?;
        let message_id = message
            .message_id
            .map(|id| id.to_string())
            .or(message.client_id.clone())
            .or_else(|| message.seq.map(|seq| seq.to_string()))
            .unwrap_or_else(next_client_id);
        {
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            if runtime.seen_messages.contains(&message_id) {
                return Ok(());
            }
            runtime.seen_messages.push_back(message_id.clone());
            while runtime.seen_messages.len() > MAX_SEEN_MESSAGES {
                runtime.seen_messages.pop_front();
            }
            match runtime.owner_user_id.as_deref() {
                Some(owner) if owner != sender => return Ok(()),
                None => runtime.owner_user_id = Some(sender.clone()),
                _ => {}
            }
            runtime.last_context_token = message.context_token.clone();
            runtime.last_inbound_at = Some(Utc::now().to_rfc3339());
            runtime.state = "connected".to_string();
            runtime.last_error = None;
        }
        self.persist()?;
        let text = message_text(&message).unwrap_or_default();
        if text.trim().is_empty() {
            self.send_text(
                &ReplyTarget {
                    user_id: sender,
                    context_token: message.context_token,
                },
                "我收到了这条消息，但当前只支持文字或语音转文字。请直接发送文字；发送 /help 可以查看用法。",
            )
            .await?;
            return Ok(());
        }
        let target = ReplyTarget {
            user_id: sender,
            context_token: message.context_token,
        };
        if let Err(error) = self
            .handle_text(target.clone(), &message_id, text.trim())
            .await
        {
            if let Ok(mut runtime) = self.runtime.write() {
                runtime.latest_activity = Some("微信指令未能执行，已告知用户".to_string());
            }
            let reply = command_error_reply(&error);
            return self.send_text(&target, &reply).await.map_err(|send_error| {
                format!("{error}；同时无法把错误提示发回微信：{send_error}")
            });
        }
        Ok(())
    }

    async fn handle_text(
        &self,
        target: ReplyTarget,
        message_id: &str,
        text: &str,
    ) -> Result<(), String> {
        let mut parts = text.splitn(2, char::is_whitespace);
        let command = normalize_command_token(parts.next().unwrap_or_default());
        let argument = parts.next().unwrap_or_default().trim();
        match command.as_str() {
            "/help" | "/start" | "/?" | "帮助" | "help" => {
                self.send_text(
                    &target,
                    "X-Ray 微信遥控\n\n通常不需要命令：先在 Mac 的 X-Ray 页面选择控制目标，然后在这里直接发送普通文字。长任务会主动同步处理中状态，完成、停止、失败和审批都会明确提醒。\n\n/list 临时查看并切换任务\n/status 查看当前目标和进度\n/stop 停止当前回合\n/approve 批准操作\n/deny 拒绝操作\n\nX-Ray 不会自动复制或 fork 会话。只有在 X-Ray 页面明确点击“新建并控制”才会创建任务。",
                )
                .await
            }
            "/tasks" | "/task" | "/list" | "任务" | "任务列表" | "tasks" => {
                let tasks = self
                    .tasks()
                    .await?
                    .into_iter()
                    .take(WEIXIN_TASK_LIST_LIMIT)
                    .collect::<Vec<_>>();
                if let Ok(mut runtime) = self.runtime.write() {
                    runtime.task_choices = tasks.iter().map(|task| task.id.clone()).collect();
                    runtime.task_selection_active = true;
                }
                let body = format_task_list(&tasks);
                self.send_text(&target, &body).await
            }
            "/attach" | "/use" | "接管" => {
                if argument.is_empty() {
                    return self
                        .send_text(
                            &target,
                            "还缺少任务序号。请先发 /list，然后直接回复序号，例如：1。",
                        )
                        .await;
                }
                let snapshot = self.attach_thread(argument).await?;
                let mode = if snapshot.control_ready {
                    if snapshot.control_backend == "desktop_ipc" {
                        "已连接 Codex Desktop 中的同一个原任务。现在直接发送普通文字即可。"
                    } else {
                        "已选为 X-Ray 独立控制目标。现在直接发送普通文字即可。"
                    }
                } else {
                    occupied_weixin_message()
                };
                self.send_text(
                    &target,
                    &format!(
                        "{}\n{}\n{}",
                        snapshot
                            .attached_thread_title
                            .as_deref()
                            .unwrap_or("Codex 任务"),
                        snapshot
                            .attached_cwd
                            .as_deref()
                            .map(remote_path_label)
                            .unwrap_or_else(|| "未知目录".to_string()),
                        mode
                    ),
                )
                .await
            }
            "/new" | "新建" => {
                self.send_text(
                    &target,
                    "为避免从微信误建任务，请在 Mac 的 X-Ray「远程通道」页面点击“新建并控制”。建好后，这里直接发送普通文字即可。",
                )
                .await
            }
            "/status" | "/state" | "状态" | "status" => {
                let body = self.status_text().await;
                self.send_text(&target, &body).await
            }
            "/cancel" | "取消" if self.cancel_task_selection() => {
                self.send_text(&target, "已退出任务选择，普通文字仍会发送给当前控制目标。")
                    .await
            }
            "/stop" | "/cancel" | "停止" | "取消" => match self.interrupt_active_turn().await {
                Ok(_) => self.send_text(&target, "已请求停止当前 Codex 回合。").await,
                Err(error) => {
                    self.send_text(&target, &sanitize_weixin_system_text(&error, 700))
                        .await
                }
            },
            "/approve" | "/yes" | "批准" | "同意" => {
                let result = self.resolve_approval(true).await;
                self.send_text(&target, &result).await
            }
            "/deny" | "/no" | "拒绝" => {
                let result = self.resolve_approval(false).await;
                self.send_text(&target, &result).await
            }
            value if value.starts_with('/') => {
                self.send_text(&target, &unknown_command_reply(value)).await
            }
            _ => {
                if let Some(result) = self.select_listed_task(text).await {
                    return match result {
                        Ok(snapshot) => {
                            let message = selected_task_message(&snapshot);
                            self.send_text(&target, &message).await
                        }
                        Err(error) => {
                            self.send_text(&target, &sanitize_weixin_system_text(&error, 700))
                                .await
                        }
                    };
                }
                self.cancel_task_selection();
                self.submit_prompt(target, message_id, text).await
            }
        }
    }

    async fn select_listed_task(
        &self,
        text: &str,
    ) -> Option<Result<RemoteChannelSnapshot, String>> {
        let index = text.trim().parse::<usize>().ok()?;
        let choice = {
            let runtime = self.runtime.read().ok()?;
            if !runtime.task_selection_active {
                return None;
            }
            index
                .checked_sub(1)
                .and_then(|index| runtime.task_choices.get(index))
                .cloned()
        };
        Some(match choice {
            Some(thread_id) => self.attach_thread(&thread_id).await,
            None => Err(
                "没有这个序号。请重新发送 /list 查看当前任务，或发送 /cancel 退出选择。"
                    .to_string(),
            ),
        })
    }

    fn cancel_task_selection(&self) -> bool {
        self.runtime
            .write()
            .map(|mut runtime| {
                let was_active = runtime.task_selection_active;
                runtime.task_selection_active = false;
                was_active
            })
            .unwrap_or(false)
    }

    pub async fn create_thread(
        &self,
        requested_cwd: &str,
    ) -> Result<RemoteChannelSnapshot, String> {
        let path = PathBuf::from(requested_cwd);
        if !path.is_absolute() || !path.is_dir() {
            return Err("项目目录必须是本机已存在的绝对目录。".to_string());
        }
        let cwd = path
            .canonicalize()
            .map_err(|error| format!("无法读取项目目录：{error}"))?;
        let known_project = self.tasks().await?.into_iter().any(|task| {
            PathBuf::from(task.cwd)
                .canonicalize()
                .is_ok_and(|known| known == cwd)
        });
        if !known_project {
            return Err(
                "为避免扩大本机目录权限，只能从已经在 Codex 中出现过的项目目录新建任务。"
                    .to_string(),
            );
        }
        self.stop_following_current_desktop().await;
        let cwd_for_client = cwd.clone();
        let client = Arc::clone(&self.codex_client);
        let state_dir = self.codex_state_dir.clone();
        let thread_id = tauri::async_runtime::spawn_blocking(move || {
            with_codex_client(&client, &state_dir, |client| {
                client
                    .start_remote_thread(&cwd_for_client)
                    .map_err(|error| error.to_string())
            })
        })
        .await
        .map_err(|error| format!("新建 Codex 任务的后台操作失败：{error}"))??;
        {
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            runtime.attached_thread_id = Some(thread_id);
            runtime.attached_thread_title = Some(
                cwd.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("新任务")
                    .to_string(),
            );
            runtime.attached_cwd = Some(cwd.to_string_lossy().to_string());
            runtime.control_ready = true;
            runtime.control_backend = "xray_app_server".to_string();
            runtime.desktop_owner_client_id = None;
            runtime.active_turn_id = None;
            runtime.agent_preview.clear();
            runtime.pending_approval = None;
            runtime.latest_activity = Some("已创建 X-Ray 独立远程任务".to_string());
            runtime.task_selection_active = false;
        }
        self.persist()?;
        Ok(self.snapshot())
    }

    async fn stop_following_current_desktop(&self) {
        let thread_id = self.runtime.read().ok().and_then(|runtime| {
            (runtime.control_backend == "desktop_ipc")
                .then(|| runtime.attached_thread_id.clone())
                .flatten()
        });
        let Some(thread_id) = thread_id else {
            return;
        };
        let desktop = Arc::clone(&self.desktop_ipc);
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let mut guard = desktop
                .lock()
                .map_err(|_| "Codex Desktop IPC 状态已损坏".to_string())?;
            let result = if let Some(client) = guard.as_mut() {
                client.unfollow(&thread_id)
            } else {
                Ok(())
            };
            if result.is_err() {
                *guard = None;
            }
            result
        })
        .await;
    }

    async fn refresh_desktop_state(&self, thread_id: &str) -> Result<(), String> {
        let backend = self
            .runtime
            .read()
            .map_err(|_| "微信通道状态已损坏".to_string())?
            .control_backend
            .clone();
        if backend != "desktop_ipc" {
            return Ok(());
        }
        let desktop = Arc::clone(&self.desktop_ipc);
        let thread_for_client = thread_id.to_string();
        let (replacement_owner, view) = tauri::async_runtime::spawn_blocking(move || {
            let mut guard = desktop
                .lock()
                .map_err(|_| "Codex Desktop IPC 状态已损坏".to_string())?;
            let existing = guard
                .as_mut()
                .ok_or_else(|| "Codex Desktop IPC 尚未连接".to_string())
                .and_then(|client| {
                    for _ in 0..16 {
                        if client
                            .receive_update(&thread_for_client, None, Duration::from_millis(2))?
                            .is_none()
                        {
                            break;
                        }
                    }
                    client
                        .current_view(&thread_for_client, None)
                        .ok_or_else(|| "Codex Desktop 原任务快照已丢失".to_string())
                });
            match existing {
                Ok(view) => Ok((None, view)),
                Err(first_error) => {
                    // Read-only state refresh is safe to retry. Re-establish both the
                    // router connection and the owner mirror after a Desktop restart or
                    // any framing/parsing failure.
                    *guard = None;
                    match probe_desktop_thread(&mut guard, &thread_for_client, None) {
                        Ok(Some((owner, view))) => Ok((Some(owner), view)),
                        Ok(None) => Err(format!(
                            "{first_error}；自动重连后 Desktop 未声明这个任务的 owner"
                        )),
                        Err(reconnect_error) => {
                            Err(format!("{first_error}；自动重连失败：{reconnect_error}"))
                        }
                    }
                }
            }
        })
        .await
        .map_err(|error| format!("刷新 Desktop 原任务状态失败：{error}"))??;
        if let Some(owner) = replacement_owner
            && let Ok(mut runtime) = self.runtime.write()
        {
            runtime.desktop_owner_client_id = Some(owner);
            runtime.latest_activity = Some("Codex Desktop IPC 已自动重连".to_string());
            runtime.last_error = None;
        }
        self.apply_desktop_view(view);
        Ok(())
    }

    fn apply_desktop_view(&self, view: DesktopThreadView) -> bool {
        let Some(mut runtime) = self.runtime.write().ok() else {
            return false;
        };
        let pending = view.pending_request.as_ref().map(desktop_pending_approval);
        let new_approval = pending.as_ref().is_some_and(|candidate| {
            runtime
                .pending_approval
                .as_ref()
                .is_none_or(|current| current.request_id != candidate.request_id)
        });
        runtime.active_turn_id = view.active_turn_id;
        if let Some(preview) = view.agent_preview {
            runtime.agent_preview = preview;
            trim_front_chars(&mut runtime.agent_preview, MAX_AGENT_PREVIEW_CHARS);
            runtime.latest_activity = Some("Codex Desktop 正在生成回复".to_string());
        }
        runtime.pending_approval = pending;
        if runtime.pending_approval.is_some() {
            runtime.latest_activity = Some("等待微信审批".to_string());
        }
        new_approval
    }

    async fn maybe_send_progress(&self, target: &ReplyTarget, feedback: &mut ProgressFeedback) {
        let details = self.runtime.read().ok().and_then(|runtime| {
            if runtime.active_turn_id.is_none() || runtime.pending_approval.is_some() {
                return None;
            }
            Some((
                runtime
                    .attached_thread_title
                    .clone()
                    .unwrap_or_else(|| "Codex 任务".to_string()),
                runtime
                    .latest_activity
                    .clone()
                    .unwrap_or_else(|| "Codex 仍在执行".to_string()),
                tail_chars(runtime.agent_preview.trim(), 320),
                runtime.control_backend.clone(),
            ))
        });
        let Some((title, activity, preview, backend)) = details else {
            return;
        };
        let signature = format!("{activity}\n{}", tail_chars(&preview, 180));
        let now = Instant::now();
        if !feedback.should_send(now, &signature) {
            return;
        }
        feedback.mark_sent(now, signature);
        let elapsed = format_elapsed(now.duration_since(feedback.started_at));
        let source = if backend == "desktop_ipc" {
            "Desktop 原任务"
        } else {
            "X-Ray 独立任务"
        };
        let mut lines = vec![
            format!("⏳ 仍在处理中（已运行 {elapsed}）"),
            format!("任务：{title}"),
            format!("来源：{source}"),
            format!("进度：{activity}"),
        ];
        if !preview.is_empty() {
            lines.push(format!("最新输出：{preview}"));
        }
        lines.push("我会继续跟进；也可以发送 /status 查看详情，/stop 停止。".to_string());
        let _ = self.send_text(target, &lines.join("\n")).await;
    }

    async fn monitor_desktop_turn(&self, thread_id: String, turn_id: String, target: ReplyTarget) {
        let session_path = self.session_path_for_thread(&thread_id).await;
        let mut progress_feedback = ProgressFeedback::new();
        loop {
            let still_expected = self
                .runtime
                .read()
                .map(|runtime| runtime.pending_turn_id.as_deref() == Some(turn_id.as_str()))
                .unwrap_or(false);
            if !still_expected {
                return;
            }
            let desktop = Arc::clone(&self.desktop_ipc);
            let thread_for_client = thread_id.clone();
            let turn_for_client = turn_id.clone();
            let update = tauri::async_runtime::spawn_blocking(move || {
                let mut guard = desktop
                    .lock()
                    .map_err(|_| "Codex Desktop IPC 状态已损坏".to_string())?;
                let result = guard
                    .as_mut()
                    .ok_or_else(|| "Codex Desktop IPC 尚未连接".to_string())
                    .and_then(|client| {
                        client.receive_update(
                            &thread_for_client,
                            Some(&turn_for_client),
                            Duration::from_millis(500),
                        )
                    });
                if result.is_err() {
                    *guard = None;
                }
                result
            })
            .await;
            let view = match update {
                Ok(Ok(Some(view))) => view,
                Ok(Ok(None)) => {
                    if self
                        .finish_turn_from_session(session_path.as_deref(), &turn_id, &target)
                        .await
                    {
                        return;
                    }
                    self.maybe_send_progress(&target, &mut progress_feedback)
                        .await;
                    continue;
                }
                Ok(Err(error)) => {
                    if self
                        .finish_turn_from_session(session_path.as_deref(), &turn_id, &target)
                        .await
                    {
                        return;
                    }
                    match self.refresh_desktop_state(&thread_id).await {
                        Ok(()) => continue,
                        Err(reconnect_error) => {
                            self.finish_turn_with_error(
                                &target,
                                format!("{error}；{reconnect_error}"),
                            )
                            .await;
                            return;
                        }
                    }
                }
                Err(error) => {
                    if self
                        .finish_turn_from_session(session_path.as_deref(), &turn_id, &target)
                        .await
                    {
                        return;
                    }
                    self.finish_turn_with_error(&target, error.to_string())
                        .await;
                    return;
                }
            };
            let outcome = view.outcome.clone();
            let notify_approval = self.apply_desktop_view(view);
            if notify_approval {
                let approval = self
                    .runtime
                    .read()
                    .ok()
                    .and_then(|runtime| runtime.pending_approval.clone());
                if let Some(approval) = approval {
                    let _ = self
                        .send_text(
                            &target,
                            &format!(
                                "⚠️ Codex Desktop 原任务需要你确认\n类型：{}\n{}\n\n回复 /approve 批准，或 /deny 拒绝。",
                                approval.snapshot.kind, approval.snapshot.summary
                            ),
                        )
                        .await;
                }
            }
            match outcome {
                Some(DesktopTurnOutcome::Completed(text)) => {
                    self.deliver_turn_result(
                        &turn_id,
                        &target,
                        text,
                        "Codex Desktop 原回合已完成".to_string(),
                    )
                    .await;
                    return;
                }
                Some(DesktopTurnOutcome::Aborted) => {
                    self.deliver_turn_result(
                        &turn_id,
                        &target,
                        "Codex 回合已停止。".to_string(),
                        "Codex Desktop 原回合已停止".to_string(),
                    )
                    .await;
                    return;
                }
                Some(DesktopTurnOutcome::Failed(error)) => {
                    self.deliver_turn_result(
                        &turn_id,
                        &target,
                        format!(
                            "Codex 回合失败：{}",
                            sanitize_weixin_system_text(&error, 700)
                        ),
                        "Codex Desktop 原回合失败".to_string(),
                    )
                    .await;
                    return;
                }
                None => {
                    self.maybe_send_progress(&target, &mut progress_feedback)
                        .await;
                }
            }
        }
    }

    async fn submit_prompt(
        &self,
        target: ReplyTarget,
        message_id: &str,
        prompt: &str,
    ) -> Result<(), String> {
        let (mut thread_id, mut control_ready) = {
            let runtime = self
                .runtime
                .read()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            (runtime.attached_thread_id.clone(), runtime.control_ready)
        };
        let Some(initial_thread_id) = thread_id.clone() else {
            return self
                .send_text(
                    &target,
                    "X-Ray 还没有选择微信控制目标。请在 Mac 的 X-Ray「远程通道」页面选择一个现有任务，或明确新建任务；人在外面时也可以发送 /list 临时选择。",
                )
                .await;
        };
        if !control_ready {
            let snapshot = self.attach_thread(&initial_thread_id).await?;
            control_ready = snapshot.control_ready;
            thread_id = snapshot.attached_thread_id;
            if !control_ready {
                return self.send_text(&target, occupied_weixin_message()).await;
            }
        }
        let thread_id = thread_id.ok_or_else(|| "重新检测后任务目标已丢失".to_string())?;
        let backend = self
            .runtime
            .read()
            .map_err(|_| "微信通道状态已损坏".to_string())?
            .control_backend
            .clone();

        if backend == "desktop_ipc" {
            self.refresh_desktop_state(&thread_id).await?;
            let (owner, active_turn_id, already_monitored) = {
                let runtime = self
                    .runtime
                    .read()
                    .map_err(|_| "微信通道状态已损坏".to_string())?;
                let active = runtime.active_turn_id.clone();
                (
                    runtime
                        .desktop_owner_client_id
                        .clone()
                        .ok_or_else(|| "Codex Desktop 控制 owner 已丢失".to_string())?,
                    active.clone(),
                    active.is_some() && runtime.pending_turn_id == active,
                )
            };
            let desktop = Arc::clone(&self.desktop_ipc);
            let thread_for_client = thread_id.clone();
            let owner_for_client = owner.clone();
            let prompt = prompt.to_string();
            let client_message_id = format!("weixin-{message_id}");
            let active_for_client = active_turn_id.clone();
            let turn_id = tauri::async_runtime::spawn_blocking(move || {
                let mut guard = desktop
                    .lock()
                    .map_err(|_| "Codex Desktop IPC 状态已损坏".to_string())?;
                let result = guard
                    .as_mut()
                    .ok_or_else(|| "Codex Desktop IPC 尚未连接".to_string())
                    .and_then(|client| {
                        if let Some(turn_id) = active_for_client {
                            client.steer_turn(
                                &thread_for_client,
                                &owner_for_client,
                                &prompt,
                                &client_message_id,
                            )?;
                            Ok((turn_id, true))
                        } else {
                            client
                                .start_turn(
                                    &thread_for_client,
                                    &owner_for_client,
                                    &prompt,
                                    &client_message_id,
                                )
                                .map(|turn_id| (turn_id, false))
                        }
                    });
                // Never retry a mutating request automatically: the remote side may
                // have accepted it before the transport failed. Drop the stream so the
                // next read-only refresh can safely reconnect without duplicating work.
                if result.is_err() {
                    *guard = None;
                }
                result
            })
            .await
            .map_err(|error| format!("控制 Desktop 原任务的后台操作失败：{error}"))??;
            let (turn_id, steered) = turn_id;
            {
                let mut runtime = self
                    .runtime
                    .write()
                    .map_err(|_| "微信通道状态已损坏".to_string())?;
                runtime.active_turn_id = Some(turn_id.clone());
                runtime.pending_thread_id = Some(thread_id.clone());
                runtime.pending_turn_id = Some(turn_id.clone());
                runtime.pending_reply_user_id = Some(target.user_id.clone());
                runtime.pending_reply_context_token = target.context_token.clone();
                runtime.latest_activity = Some(if steered {
                    "已向 Codex Desktop 当前回合追加指令".to_string()
                } else {
                    "Codex Desktop 原任务正在处理微信指令".to_string()
                });
                if !steered {
                    runtime.agent_preview.clear();
                    runtime.pending_approval = None;
                }
            }
            let recovery_warning = self.persist().err();
            let acknowledgement = if steered {
                "✅ 已追加到 Codex Desktop 正在执行的原回合，我会继续同步进度。".to_string()
            } else if let Some(warning) = recovery_warning {
                format!(
                    "✅ 已收到，Codex Desktop 原任务正在处理。超过约 10 秒会主动同步进度。\n\n但重启恢复信息保存失败：{warning}\n请保持 X-Ray 开启。"
                )
            } else {
                "✅ 已收到，Codex Desktop 原任务正在处理。超过约 10 秒会主动同步进度；也可发送 /status 查看，/stop 停止。".to_string()
            };
            let acknowledgement_result = self.send_text(&target, &acknowledgement).await;
            if !already_monitored {
                let state = self.clone();
                let monitor_target = target.clone();
                tauri::async_runtime::spawn(async move {
                    state
                        .monitor_desktop_turn(thread_id, turn_id, monitor_target)
                        .await;
                });
            }
            return acknowledgement_result;
        }

        if backend != "xray_app_server" {
            return Err(
                "当前目标没有可用控制链路；X-Ray 没有创建副本。请在 Mac 上重新选择目标。"
                    .to_string(),
            );
        }
        let active_turn_id = self
            .runtime
            .read()
            .map_err(|_| "微信通道状态已损坏".to_string())?
            .active_turn_id
            .clone();
        let client = Arc::clone(&self.codex_client);
        let state_dir = self.codex_state_dir.clone();
        let prompt = prompt.to_string();
        let client_message_id = format!("weixin-{message_id}");
        if let Some(turn_id) = active_turn_id {
            let thread_for_client = thread_id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                with_codex_client(&client, &state_dir, |client| {
                    client
                        .steer_remote_turn(
                            &thread_for_client,
                            &turn_id,
                            &prompt,
                            Some(&client_message_id),
                        )
                        .map_err(|error| error.to_string())
                })
            })
            .await
            .map_err(|error| format!("追加 Codex 指令的后台操作失败：{error}"))??;
            if let Ok(mut runtime) = self.runtime.write() {
                runtime.latest_activity = Some("已向正在执行的回合追加指令".to_string());
            }
            return self
                .send_text(&target, "✅ 已追加到当前 Codex 回合，我会继续同步进度。")
                .await;
        }
        let thread_for_client = thread_id.clone();
        let turn_id = tauri::async_runtime::spawn_blocking(move || {
            with_codex_client(&client, &state_dir, |client| {
                client
                    .start_remote_turn(&thread_for_client, &prompt, Some(&client_message_id))
                    .map_err(|error| error.to_string())
            })
        })
        .await
        .map_err(|error| format!("启动 Codex 回合的后台操作失败：{error}"))??;
        {
            let mut runtime = self
                .runtime
                .write()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            runtime.active_turn_id = Some(turn_id.clone());
            runtime.pending_thread_id = Some(thread_id.clone());
            runtime.pending_turn_id = Some(turn_id.clone());
            runtime.pending_reply_user_id = Some(target.user_id.clone());
            runtime.pending_reply_context_token = target.context_token.clone();
            runtime.latest_activity = Some("Codex 正在处理微信指令".to_string());
            runtime.agent_preview.clear();
            runtime.pending_approval = None;
        }
        let recovery_warning = self.persist().err();
        let acknowledgement = recovery_warning.map_or_else(
            || {
                "✅ 已收到，Codex 正在处理。超过约 10 秒会主动同步进度；也可发送 /status 查看，/stop 停止。"
                    .to_string()
            },
            |warning| {
                format!(
                    "✅ 已收到，Codex 正在处理。超过约 10 秒会主动同步进度。\n\n但重启恢复信息保存失败：{warning}\n请保持 X-Ray 开启。"
                )
            },
        );
        let acknowledgement_result = self.send_text(&target, &acknowledgement).await;
        let state = self.clone();
        let monitor_target = target.clone();
        tauri::async_runtime::spawn(async move {
            state.monitor_turn(thread_id, turn_id, monitor_target).await;
        });
        acknowledgement_result
    }

    async fn monitor_turn(&self, thread_id: String, turn_id: String, target: ReplyTarget) {
        let mut session_path = self.session_path_for_thread(&thread_id).await;
        let mut empty_polls = 0_u32;
        let mut progress_feedback = ProgressFeedback::new();
        loop {
            let still_active = self
                .runtime
                .read()
                .map(|runtime| runtime.active_turn_id.as_deref() == Some(turn_id.as_str()))
                .unwrap_or(false);
            if !still_active {
                return;
            }
            let client = Arc::clone(&self.codex_client);
            let state_dir = self.codex_state_dir.clone();
            let event = tauri::async_runtime::spawn_blocking(move || {
                with_codex_client(&client, &state_dir, |client| {
                    client
                        .receive_remote_event(Duration::from_millis(500))
                        .map_err(|error| error.to_string())
                })
            })
            .await;
            let event = match event {
                Ok(Ok(event)) => event,
                Ok(Err(error)) => {
                    if self
                        .finish_turn_from_session(session_path.as_deref(), &turn_id, &target)
                        .await
                    {
                        return;
                    }
                    self.finish_turn_with_error(&target, error).await;
                    return;
                }
                Err(error) => {
                    if self
                        .finish_turn_from_session(session_path.as_deref(), &turn_id, &target)
                        .await
                    {
                        return;
                    }
                    self.finish_turn_with_error(&target, error.to_string())
                        .await;
                    return;
                }
            };
            let Some(event) = event else {
                empty_polls = empty_polls.saturating_add(1);
                if session_path.is_none() && empty_polls.is_multiple_of(4) {
                    session_path = self.session_path_for_thread(&thread_id).await;
                }
                if self
                    .finish_turn_from_session(session_path.as_deref(), &turn_id, &target)
                    .await
                {
                    return;
                }
                self.maybe_send_progress(&target, &mut progress_feedback)
                    .await;
                continue;
            };
            empty_polls = 0;
            let method = event.get("method").and_then(Value::as_str);
            let params = event.get("params").cloned().unwrap_or(Value::Null);
            if event.get("id").is_some() && method.is_some() {
                self.handle_server_request(&thread_id, &turn_id, event, &target)
                    .await;
                continue;
            }
            match method {
                Some("item/agentMessage/delta")
                    if event_matches_turn(&params, &thread_id, &turn_id) =>
                {
                    if let Some(delta) = params.get("delta").and_then(Value::as_str)
                        && let Ok(mut runtime) = self.runtime.write()
                    {
                        runtime.agent_preview.push_str(delta);
                        trim_front_chars(&mut runtime.agent_preview, MAX_AGENT_PREVIEW_CHARS);
                        runtime.latest_activity = Some("Codex 正在生成回复".to_string());
                    }
                }
                Some("item/started") if event_matches_turn(&params, &thread_id, &turn_id) => {
                    if let Some(activity) = item_activity(&params, false)
                        && let Ok(mut runtime) = self.runtime.write()
                    {
                        runtime.latest_activity = Some(activity);
                    }
                }
                Some("item/completed") if event_matches_turn(&params, &thread_id, &turn_id) => {
                    if let Some(activity) = item_activity(&params, true)
                        && let Ok(mut runtime) = self.runtime.write()
                    {
                        runtime.latest_activity = Some(activity);
                    }
                }
                Some("turn/completed") if event_matches_turn(&params, &thread_id, &turn_id) => {
                    self.deliver_turn_result(
                        &turn_id,
                        &target,
                        final_agent_message(&params)
                            .unwrap_or_else(|| "Codex 回合已结束。".to_string()),
                        turn_completion_activity(&params),
                    )
                    .await;
                    return;
                }
                _ => {}
            }
            self.maybe_send_progress(&target, &mut progress_feedback)
                .await;
        }
    }

    async fn session_path_for_thread(&self, thread_id: &str) -> Option<PathBuf> {
        let client = Arc::clone(&self.codex_client);
        let state_dir = self.codex_state_dir.clone();
        let thread_id = thread_id.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            with_codex_client(&client, &state_dir, |client| {
                client
                    .fetch_thread_metadata()
                    .map_err(|error| error.to_string())
            })
            .ok()
            .and_then(|threads| {
                threads
                    .into_iter()
                    .find(|thread| thread.id == thread_id)
                    .and_then(|thread| thread.path)
                    .map(PathBuf::from)
            })
        })
        .await
        .ok()
        .flatten()
    }

    async fn finish_turn_from_session(
        &self,
        session_path: Option<&Path>,
        turn_id: &str,
        target: &ReplyTarget,
    ) -> bool {
        let Some(outcome) = session_path.and_then(|path| session_turn_outcome(path, turn_id))
        else {
            return false;
        };
        let (text, activity) = match outcome {
            SessionTurnOutcome::Completed(text) => (text, "Codex 已完成，结果已从任务日志恢复"),
            SessionTurnOutcome::Aborted => (
                "Codex 回合已停止。".to_string(),
                "Codex 回合已停止，状态已从任务日志恢复",
            ),
        };
        self.deliver_turn_result(turn_id, target, text, activity.to_string())
            .await;
        true
    }

    async fn deliver_turn_result(
        &self,
        turn_id: &str,
        target: &ReplyTarget,
        fallback_text: String,
        activity: String,
    ) {
        let terminal_label = if activity.contains("失败") {
            "⚠️ Codex 执行失败"
        } else if activity.contains("停止") || activity.contains("中断") {
            "⏹️ Codex 已停止"
        } else {
            "✅ Codex 已完成"
        };
        let final_text = {
            let mut runtime = match self.runtime.write() {
                Ok(runtime) => runtime,
                Err(_) => return,
            };
            runtime.active_turn_id = None;
            runtime.pending_approval = None;
            runtime.latest_activity = Some(activity);
            let text = if runtime.agent_preview.trim().is_empty() {
                fallback_text
            } else {
                runtime.agent_preview.trim().to_string()
            };
            runtime.agent_preview.clear();
            text
        };
        let outbound = format!("{terminal_label}\n\n{final_text}");
        match self.send_text(target, &outbound).await {
            Ok(()) => {
                if let Ok(mut runtime) = self.runtime.write()
                    && runtime.pending_turn_id.as_deref() == Some(turn_id)
                {
                    runtime.pending_thread_id = None;
                    runtime.pending_turn_id = None;
                    runtime.pending_reply_user_id = None;
                    runtime.pending_reply_context_token = None;
                }
                let _ = self.persist();
            }
            Err(error) => {
                if let Ok(mut runtime) = self.runtime.write() {
                    runtime.state = "degraded".to_string();
                    runtime.last_error = Some(format!("Codex 已完成，但微信回复失败：{error}"));
                    runtime.latest_activity = Some("Codex 已完成，等待重新发送结果".to_string());
                }
                let _ = self.persist();
            }
        }
    }

    async fn handle_server_request(
        &self,
        thread_id: &str,
        turn_id: &str,
        event: Value,
        target: &ReplyTarget,
    ) {
        let Some(id) = event.get("id").cloned() else {
            return;
        };
        let method = event
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = event.get("params").cloned().unwrap_or(Value::Null);
        if !event_matches_turn(&params, thread_id, turn_id) {
            return;
        }
        if !matches!(
            method.as_str(),
            "item/commandExecution/requestApproval"
                | "item/fileChange/requestApproval"
                | "execCommandApproval"
                | "applyPatchApproval"
        ) {
            let client = Arc::clone(&self.codex_client);
            let state_dir = self.codex_state_dir.clone();
            let method_for_error = method.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                with_codex_client(&client, &state_dir, |client| {
                    client
                        .respond_to_server_request_error(
                            id,
                            -32601,
                            &format!("X-Ray 微信通道暂不支持 {method_for_error}"),
                        )
                        .map_err(|error| error.to_string())
                })
            })
            .await;
            let _ = self
                .send_text(target, "Codex 请求了首版微信通道尚不支持的权限，已拒绝。")
                .await;
            return;
        }
        let snapshot = RemoteApprovalSnapshot {
            kind: approval_kind(&method).to_string(),
            summary: approval_summary(&method, &params),
            requested_at: Utc::now().to_rfc3339(),
        };
        if let Ok(mut runtime) = self.runtime.write() {
            runtime.latest_activity = Some("等待微信审批".to_string());
            runtime.pending_approval = Some(PendingApproval {
                request_id: id,
                method,
                snapshot: snapshot.clone(),
            });
        }
        let _ = self
            .send_text(
                target,
                &format!(
                    "⚠️ Codex 需要你确认\n类型：{}\n{}\n\n回复 /approve 批准，或 /deny 拒绝。",
                    snapshot.kind, snapshot.summary
                ),
            )
            .await;
    }

    async fn resolve_approval(&self, approve: bool) -> String {
        let (pending, backend, thread_id, desktop_owner) = self
            .runtime
            .write()
            .ok()
            .map(|mut runtime| {
                (
                    runtime.pending_approval.take(),
                    runtime.control_backend.clone(),
                    runtime.attached_thread_id.clone(),
                    runtime.desktop_owner_client_id.clone(),
                )
            })
            .unwrap_or_default();
        let Some(pending) = pending else {
            return "当前没有等待审批的操作。".to_string();
        };
        let response = if backend == "desktop_ipc" {
            let Some(thread_id) = thread_id else {
                return "审批回复失败：Desktop 任务 ID 已丢失".to_string();
            };
            let Some(owner) = desktop_owner else {
                return "审批回复失败：Desktop owner 已丢失".to_string();
            };
            let desktop = Arc::clone(&self.desktop_ipc);
            tauri::async_runtime::spawn_blocking(move || {
                let mut guard = desktop
                    .lock()
                    .map_err(|_| "Codex Desktop IPC 状态已损坏".to_string())?;
                let result = guard
                    .as_mut()
                    .ok_or_else(|| "Codex Desktop IPC 尚未连接".to_string())
                    .and_then(|client| {
                        client.resolve_approval(
                            &thread_id,
                            &owner,
                            pending.request_id,
                            &pending.method,
                            approve,
                        )
                    });
                if result.is_err() {
                    *guard = None;
                }
                result
            })
            .await
        } else {
            let result = approval_result(&pending.method, approve);
            let client = Arc::clone(&self.codex_client);
            let state_dir = self.codex_state_dir.clone();
            tauri::async_runtime::spawn_blocking(move || {
                with_codex_client(&client, &state_dir, |client| {
                    client
                        .respond_to_server_request(pending.request_id, result)
                        .map_err(|error| error.to_string())
                })
            })
            .await
        };
        match response {
            Ok(Ok(())) => {
                if let Ok(mut runtime) = self.runtime.write() {
                    runtime.latest_activity = Some(if approve {
                        "已从微信批准操作".to_string()
                    } else {
                        "已从微信拒绝操作".to_string()
                    });
                }
                if approve {
                    "已批准，Codex 将继续执行。".to_string()
                } else {
                    "已拒绝，Codex 将尝试其他方案。".to_string()
                }
            }
            Ok(Err(error)) => format!("审批回复失败：{error}"),
            Err(error) => format!("审批后台操作失败：{error}"),
        }
    }

    async fn status_text(&self) -> String {
        let desktop_thread = self.runtime.read().ok().and_then(|runtime| {
            (runtime.control_backend == "desktop_ipc")
                .then(|| runtime.attached_thread_id.clone())
                .flatten()
        });
        if let Some(thread_id) = desktop_thread {
            let _ = self.refresh_desktop_state(&thread_id).await;
        }
        let snapshot = self.snapshot();
        let state = match snapshot.state.as_str() {
            "connected" => "已连接",
            "stopped" => "已暂停",
            "degraded" => "连接异常",
            "login_required" => "需要重新扫码",
            _ => snapshot.state.as_str(),
        };
        let mut lines = vec![format!("微信通道：{state}")];
        if let Some(title) = snapshot.attached_thread_title {
            lines.push(format!("任务：{title}"));
        }
        if let Some(ref id) = snapshot.attached_thread_id {
            lines.push(format!("ID：{}", short_id(id)));
        }
        if let Some(cwd) = snapshot.attached_cwd {
            lines.push(format!("目录：{}", remote_path_label(&cwd)));
        }
        lines.push(format!(
            "控制：{}",
            match (snapshot.control_ready, snapshot.control_backend.as_str()) {
                (true, "desktop_ipc") => "Codex Desktop 原任务（IPC）",
                (true, "xray_app_server") => "X-Ray 独立 App Server 任务",
                (true, _) => "X-Ray 已建立控制",
                (false, _) => "暂不可控制（未创建副本）",
            }
        ));
        if let Some(ref turn) = snapshot.active_turn_id {
            lines.push(format!("回合：{}（运行中）", short_id(turn)));
            lines.push("主动反馈：已开启（自动限频）".to_string());
        }
        if let Some(activity) = snapshot.latest_activity {
            lines.push(format!("进度：{activity}"));
        }
        if let Some(preview) = snapshot.agent_preview {
            lines.push(format!("\n最新输出：\n{}", tail_chars(&preview, 700)));
        }
        if snapshot.pending_approval.is_some() {
            lines.push("等待审批：使用 /approve 或 /deny".to_string());
        }
        if snapshot.attached_thread_id.is_none() {
            lines.push(
                "下一步：请在 X-Ray 页面选择控制目标；人在外面时也可发送 /list 临时切换。"
                    .to_string(),
            );
        } else if !snapshot.control_ready {
            lines.push(format!("下一步：{}", occupied_weixin_message()));
        } else if snapshot.control_ready && snapshot.active_turn_id.is_none() {
            lines.push("下一步：直接发送普通文字即可控制 Codex。".to_string());
        }
        lines.join("\n")
    }

    async fn finish_turn_with_error(&self, target: &ReplyTarget, error: String) {
        if let Ok(mut runtime) = self.runtime.write() {
            runtime.active_turn_id = None;
            runtime.pending_approval = None;
            runtime.state = "degraded".to_string();
            runtime.last_error = Some(error.clone());
            runtime.latest_activity = Some("Codex 连接中断".to_string());
        }
        let _ = self
            .send_text(
                target,
                &format!(
                    "⚠️ Codex 任务连接中断\n\n{}\n\n可以发送 /status 查看当前目标。",
                    sanitize_weixin_system_text(&error, 700)
                ),
            )
            .await;
    }

    async fn send_text(&self, target: &ReplyTarget, text: &str) -> Result<(), String> {
        let (base_url, token) = {
            let runtime = self
                .runtime
                .read()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            (
                runtime.base_url.clone(),
                runtime
                    .token
                    .clone()
                    .ok_or_else(|| "微信通道尚未登录。".to_string())?,
            )
        };
        for chunk in split_text(text, MAX_WEIXIN_CHUNK_CHARS) {
            let mut message = json!({
                "from_user_id": "",
                "to_user_id": target.user_id,
                "client_id": next_client_id(),
                "message_type": 2,
                "message_state": 2,
                "item_list": [{
                    "type": 1,
                    "text_item": { "text": chunk }
                }]
            });
            if let Some(context_token) = &target.context_token {
                message["context_token"] = Value::String(context_token.clone());
            }
            let response = self
                .post_json::<WeixinApiResponse>(
                    &base_url,
                    "ilink/bot/sendmessage",
                    Some(&token),
                    json!({
                        "msg": message.clone(),
                        "base_info": base_info()
                    }),
                    Duration::from_secs(30),
                )
                .await?;
            if let Err(error) = ensure_api_success(&response, "sendmessage") {
                if target.context_token.is_some() && error.contains("ret=-2") {
                    message
                        .as_object_mut()
                        .expect("outbound message is an object")
                        .remove("context_token");
                    let retry = self
                        .post_json::<WeixinApiResponse>(
                            &base_url,
                            "ilink/bot/sendmessage",
                            Some(&token),
                            json!({
                                "msg": message,
                                "base_info": base_info()
                            }),
                            Duration::from_secs(30),
                        )
                        .await?;
                    ensure_api_success(&retry, "sendmessage")?;
                } else {
                    return Err(error);
                }
            }
            if let Ok(mut runtime) = self.runtime.write() {
                runtime.last_outbound_at = Some(Utc::now().to_rfc3339());
                runtime.state = "connected".to_string();
                runtime.last_error = None;
            }
            tokio::time::sleep(Duration::from_millis(1_200)).await;
        }
        Ok(())
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        base_url: &str,
        endpoint: &str,
        timeout: Duration,
    ) -> Result<T, String> {
        let url = endpoint_url(base_url, endpoint)?;
        let response = self
            .client
            .get(url)
            .header("iLink-App-Id", "bot")
            .header("iLink-App-ClientVersion", WEIXIN_CLIENT_VERSION)
            .timeout(timeout)
            .send()
            .await
            .map_err(|error| format!("微信登录状态请求失败：{error}"))?;
        parse_response(response).await
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        base_url: &str,
        endpoint: &str,
        token: Option<&str>,
        body: Value,
        timeout: Duration,
    ) -> Result<T, String> {
        let url = endpoint_url(base_url, endpoint)?;
        let mut request = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("AuthorizationType", "ilink_bot_token")
            .header("X-WECHAT-UIN", random_wechat_uin())
            .header("iLink-App-Id", "bot")
            .header("iLink-App-ClientVersion", WEIXIN_CLIENT_VERSION)
            .timeout(timeout)
            .json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("微信通道请求失败：{error}"))?;
        parse_response(response).await
    }

    fn set_degraded(&self, error: String) {
        if let Ok(mut runtime) = self.runtime.write() {
            runtime.state = "degraded".to_string();
            runtime.last_error = Some(error);
        }
    }

    fn persist(&self) -> Result<(), String> {
        let file = {
            let runtime = self
                .runtime
                .read()
                .map_err(|_| "微信通道状态已损坏".to_string())?;
            RemoteChannelFile {
                version: 1,
                enabled: runtime.enabled,
                account_id: runtime.account_id.clone(),
                owner_user_id: runtime.owner_user_id.clone(),
                base_url: runtime.base_url.clone(),
                get_updates_buf: runtime.get_updates_buf.clone(),
                attached_thread_id: runtime.attached_thread_id.clone(),
                attached_thread_title: runtime.attached_thread_title.clone(),
                attached_cwd: runtime.attached_cwd.clone(),
                pending_thread_id: runtime.pending_thread_id.clone(),
                pending_turn_id: runtime.pending_turn_id.clone(),
                pending_reply_user_id: runtime.pending_reply_user_id.clone(),
                pending_reply_context_token: runtime.pending_reply_context_token.clone(),
            }
        };
        save_config(&self.config_path, &file)
    }
}

#[derive(Debug, Clone)]
struct WeixinAccount {
    token: String,
    base_url: String,
}

#[derive(Debug, Deserialize)]
struct WeixinQrStartResponse {
    qrcode: String,
    qrcode_img_content: String,
}

#[derive(Debug, Deserialize)]
struct WeixinQrStatusResponse {
    status: String,
    bot_token: Option<String>,
    ilink_bot_id: Option<String>,
    baseurl: Option<String>,
    ilink_user_id: Option<String>,
    redirect_host: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WeixinGetUpdatesResponse {
    ret: Option<i64>,
    errcode: Option<i64>,
    errmsg: Option<String>,
    msgs: Option<Vec<WeixinMessage>>,
    get_updates_buf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WeixinApiResponse {
    ret: Option<i64>,
    errcode: Option<i64>,
    errmsg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WeixinMessage {
    seq: Option<u64>,
    message_id: Option<u64>,
    from_user_id: Option<String>,
    client_id: Option<String>,
    group_id: Option<String>,
    message_type: Option<u32>,
    item_list: Option<Vec<WeixinMessageItem>>,
    context_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WeixinMessageItem {
    #[serde(rename = "type")]
    item_type: Option<u32>,
    text_item: Option<WeixinTextItem>,
    voice_item: Option<WeixinTextItem>,
}

#[derive(Debug, Deserialize)]
struct WeixinTextItem {
    text: Option<String>,
}

fn with_codex_client<T>(
    client: &Arc<Mutex<Option<AppServerClient>>>,
    state_dir: &Path,
    operation: impl FnOnce(&mut AppServerClient) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = client
        .lock()
        .map_err(|_| "Codex X-Ray 内部连接状态已损坏".to_string())?;
    if guard.is_none() {
        *guard = Some(
            AppServerClient::start(state_dir)
                .map_err(|error| format!("无法启动 Codex App Server：{error}"))?,
        );
    }
    operation(guard.as_mut().expect("client is initialized"))
}

fn summarize_threads(threads: Vec<ThreadMetadata>) -> Vec<RemoteTaskSummary> {
    threads
        .into_iter()
        .filter(|thread| thread.parent_thread_id.is_none())
        .map(|thread| {
            let status = thread.status.clone().or_else(|| {
                thread
                    .path
                    .as_deref()
                    .filter(|path| session_has_recent_open_turn(Path::new(path)))
                    .map(|_| "running".to_string())
            });
            RemoteTaskSummary {
                title: thread.name.unwrap_or_else(|| {
                    path_title(&thread.cwd).unwrap_or_else(|| "未命名任务".to_string())
                }),
                control_mode: if status.is_some() {
                    "observe".to_string()
                } else {
                    "available".to_string()
                },
                status: status.unwrap_or_else(|| "idle".to_string()),
                id: thread.id,
                cwd: thread.cwd,
                updated_at: thread.updated_at,
            }
        })
        .collect()
}

fn session_has_recent_open_turn(path: &Path) -> bool {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    let recently_modified = metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|elapsed| elapsed <= ACTIVE_SESSION_WINDOW);
    if !recently_modified {
        return false;
    }
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let length = metadata.len();
    let start = length.saturating_sub(SESSION_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return false;
    }
    let mut content = String::new();
    if file.read_to_string(&mut content).is_err() {
        return false;
    }
    let mut open_turns = HashSet::new();
    for (index, line) in content.lines().enumerate() {
        if start > 0 && index == 0 {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let Some(turn_id) = entry.pointer("/payload/turn_id").and_then(Value::as_str) else {
            continue;
        };
        match entry.pointer("/payload/type").and_then(Value::as_str) {
            Some("task_started") => {
                open_turns.insert(turn_id.to_string());
            }
            Some("task_complete") | Some("turn_aborted") => {
                open_turns.remove(turn_id);
            }
            _ => {}
        }
    }
    !open_turns.is_empty()
}

#[derive(Debug, PartialEq, Eq)]
enum SessionTurnOutcome {
    Completed(String),
    Aborted,
}

fn session_turn_outcome(path: &Path, expected_turn_id: &str) -> Option<SessionTurnOutcome> {
    let metadata = fs::metadata(path).ok()?;
    let mut file = fs::File::open(path).ok()?;
    let start = metadata.len().saturating_sub(SESSION_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut content = String::new();
    file.read_to_string(&mut content).ok()?;
    let mut outcome = None;
    for (index, line) in content.lines().enumerate() {
        if start > 0 && index == 0 {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("event_msg")
            || entry.pointer("/payload/turn_id").and_then(Value::as_str) != Some(expected_turn_id)
        {
            continue;
        }
        match entry.pointer("/payload/type").and_then(Value::as_str) {
            Some("task_complete") => {
                let text = entry
                    .pointer("/payload/last_agent_message")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or("Codex 回合已结束。")
                    .to_string();
                outcome = Some(SessionTurnOutcome::Completed(text));
            }
            Some("turn_aborted") => outcome = Some(SessionTurnOutcome::Aborted),
            _ => {}
        }
    }
    outcome
}

fn format_task_list(tasks: &[RemoteTaskSummary]) -> String {
    if tasks.is_empty() {
        return "当前没有找到 Codex 任务。".to_string();
    }
    let mut lines = vec!["最近的 Codex 任务：".to_string()];
    for (index, task) in tasks.iter().enumerate() {
        let status = match task.status.as_str() {
            "running" => "运行中",
            "waiting_approval" => "等待审批",
            "waiting_input" => "等待输入",
            "failed" => "失败",
            _ => "空闲",
        };
        lines.push(format!(
            "{}. [{}] {}\n   {} · 目录：{}",
            index + 1,
            status,
            task.title,
            short_id(&task.id),
            remote_path_label(&task.cwd)
        ));
    }
    lines.push("\n直接回复序号即可切换，例如：2。发送 /cancel 退出选择。".to_string());
    lines.join("\n")
}

fn probe_desktop_thread(
    desktop: &mut Option<DesktopIpcClient>,
    conversation_id: &str,
    previous_conversation_id: Option<&str>,
) -> Result<Option<(String, DesktopThreadView)>, String> {
    let mut last_error = None;
    for attempt in 0..DESKTOP_IPC_PROBE_ATTEMPTS {
        if desktop.is_none() {
            match DesktopIpcClient::connect() {
                Ok(client) => *desktop = Some(client),
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 < DESKTOP_IPC_PROBE_ATTEMPTS {
                        std::thread::sleep(Duration::from_millis(75));
                        continue;
                    }
                    break;
                }
            }
        }

        let result = desktop
            .as_mut()
            .ok_or_else(|| "Codex Desktop IPC 初始化后连接丢失".to_string())
            .and_then(|client| {
                let Some(owner) = client.discover_owner(conversation_id)? else {
                    if let Some(previous) = previous_conversation_id
                        && previous != conversation_id
                    {
                        let _ = client.unfollow(previous);
                    }
                    return Ok(None);
                };

                // Follow the new task before releasing the previous mirror so a failed
                // switch never silently leaves the user with no valid Desktop snapshot.
                let view = client.follow(conversation_id, &owner)?;
                if let Some(previous) = previous_conversation_id
                    && previous != conversation_id
                {
                    let _ = client.unfollow(previous);
                }
                Ok(Some((owner, view)))
            });

        match result {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = Some(error);
                // A reader/parsing/transport error leaves framing state unknown. Never
                // reuse that stream: drop it and retry once with a fresh connection.
                *desktop = None;
                if attempt + 1 < DESKTOP_IPC_PROBE_ATTEMPTS {
                    std::thread::sleep(Duration::from_millis(75));
                }
            }
        }
    }

    Err(format!(
        "Codex Desktop IPC 自动重连后仍失败：{}",
        last_error.unwrap_or_else(|| "未知错误".to_string())
    ))
}

fn occupied_activity() -> String {
    "X-Ray 已尝试自动唤醒原任务，但写入权仍由 Codex Desktop 持有，IPC 尚未建立；未创建副本或新会话"
        .to_string()
}

fn occupied_weixin_message() -> &'static str {
    "X-Ray 已尝试自动唤醒这个 Codex Desktop 原任务，但尚未通过本机 IPC 建立控制。请回到 Mac，在 X-Ray 点击“重试自动连接”；如果仍失败，再手动打开完全相同的 Codex 任务。本次没有创建副本或新会话。"
}

fn desktop_control_unavailable_message(error: Option<&str>) -> String {
    let detail = error
        .map(str::trim)
        .filter(|error| !error.is_empty())
        .map(|error| format!(" IPC 详情：{}", sanitize_weixin_system_text(error, 500)))
        .unwrap_or_else(|| " Codex Desktop 没有及时注册这个任务的 owner。".to_string());
    format!(
        "X-Ray 已尝试在后台自动打开这个 Codex Desktop 原任务，但尚未建立同任务控制。{detail}请重试自动连接；如果仍失败，再手动打开完全相同的 Desktop 任务。X-Ray 没有创建副本或新会话。"
    )
}

fn codex_thread_deep_link(thread_id: &str) -> Result<String, String> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty()
        || thread_id.len() > 256
        || !thread_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("Codex 任务 ID 无法安全用于自动唤醒".to_string());
    }
    Ok(format!("codex://threads/{thread_id}"))
}

#[cfg(target_os = "macos")]
fn launch_codex_thread(thread_id: &str) -> Result<(), String> {
    let url = codex_thread_deep_link(thread_id)?;
    Command::new("open")
        .arg("-g")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法在后台唤醒 Codex Desktop 原任务：{error}"))
}

#[cfg(target_os = "windows")]
fn launch_codex_thread(thread_id: &str) -> Result<(), String> {
    let url = codex_thread_deep_link(thread_id)?;
    Command::new("cmd")
        .args(["/C", "start", "", &url])
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法唤醒 Codex Desktop 原任务：{error}"))
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn launch_codex_thread(thread_id: &str) -> Result<(), String> {
    let url = codex_thread_deep_link(thread_id)?;
    Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法唤醒 Codex Desktop 原任务：{error}"))
}

fn selected_task_message(snapshot: &RemoteChannelSnapshot) -> String {
    let title = snapshot
        .attached_thread_title
        .as_deref()
        .unwrap_or("Codex 任务");
    let cwd = snapshot
        .attached_cwd
        .as_deref()
        .map(remote_path_label)
        .unwrap_or_else(|| "未知目录".to_string());
    let state = if snapshot.control_ready && snapshot.control_backend == "desktop_ipc" {
        "已连接 Codex Desktop 中的同一个原任务。现在直接发送普通文字即可。".to_string()
    } else if snapshot.control_ready {
        "已设为 X-Ray 独立任务。现在直接发送普通文字即可。".to_string()
    } else {
        snapshot
            .last_error
            .clone()
            .unwrap_or_else(|| occupied_weixin_message().to_string())
    };
    format!("已选择：{title}\n{cwd}\n{state}")
}

fn message_text(message: &WeixinMessage) -> Option<String> {
    message.item_list.as_ref()?.iter().find_map(|item| {
        if item.item_type == Some(1) {
            item.text_item.as_ref()?.text.clone()
        } else if item.item_type == Some(3) {
            item.voice_item.as_ref()?.text.clone()
        } else {
            None
        }
    })
}

fn normalize_command_token(value: &str) -> String {
    let lowered = value.to_lowercase();
    lowered
        .strip_prefix('／')
        .map(|command| format!("/{command}"))
        .unwrap_or(lowered)
}

fn attach_error_message(error: &str) -> String {
    if is_active_writer_conflict(error) {
        return occupied_weixin_message().to_string();
    }
    format!(
        "接管原 Codex 任务失败：{}",
        sanitize_weixin_system_text(error, 700)
    )
}

fn desktop_pending_approval(request: &DesktopPendingRequest) -> PendingApproval {
    PendingApproval {
        request_id: request.request_id.clone(),
        method: request.method.clone(),
        snapshot: RemoteApprovalSnapshot {
            kind: approval_kind(&request.method).to_string(),
            summary: approval_summary(&request.method, &request.params),
            requested_at: Utc::now().to_rfc3339(),
        },
    }
}

fn is_active_writer_conflict(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("active writer") || error.contains("another writer")
}

fn is_occupied_error(error: &str) -> bool {
    is_active_writer_conflict(error) || error.contains("IPC")
}

fn command_error_reply(error: &str) -> String {
    if error.contains("没有创建副本或新会话") {
        return error.to_string();
    }
    if error.contains("没有找到匹配的 Codex 任务") {
        return "没有找到这个任务。请发送 /list 获取最新列表，然后直接回复序号。".to_string();
    }
    if error.contains("任务 ID 前缀不唯一") {
        return "这个任务 ID 同时匹配了多个任务。请发送 /list，然后直接回复序号，例如：2。"
            .to_string();
    }
    if error.contains("项目目录必须是本机已存在的绝对目录") {
        return "这个目录在 Mac 上不存在，或者不是绝对路径。请回到 X-Ray 页面重新选择项目目录。"
            .to_string();
    }
    format!(
        "这条指令没处理成功：{}\n\n可以重试，或发送 /help 查看用法。",
        sanitize_weixin_system_text(error, 700)
    )
}

fn unknown_command_reply(command: &str) -> String {
    const COMMANDS: [(&str, &str); 6] = [
        ("/help", "查看用法"),
        ("/list", "临时切换任务"),
        ("/status", "查看状态"),
        ("/stop", "停止当前回合"),
        ("/approve", "批准操作"),
        ("/deny", "拒绝操作"),
    ];
    let suggestion = COMMANDS
        .iter()
        .map(|candidate| (edit_distance(command, candidate.0), candidate))
        .min_by_key(|(distance, _)| *distance)
        .filter(|(distance, _)| *distance <= 2)
        .map(|(_, candidate)| *candidate);
    match suggestion {
        Some((candidate, description)) => format!(
            "命令 {command} 不存在。你是不是想用 {candidate}（{description}）？\n发送 /help 可以查看全部命令。"
        ),
        None => format!(
            "命令 {command} 不存在。发送 /help 查看可用命令；如果这是给 Codex 的普通指令，请不要以 / 开头。"
        ),
    }
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(left_index + 1);
        for (right_index, right_char) in right.iter().enumerate() {
            current.push(
                (previous[right_index + 1] + 1)
                    .min(current[right_index] + 1)
                    .min(previous[right_index] + usize::from(left_char != *right_char)),
            );
        }
        previous = current;
    }
    previous[right.len()]
}

fn is_direct_user_message(message: &WeixinMessage) -> bool {
    message.message_type != Some(2)
        && !message
            .group_id
            .as_deref()
            .is_some_and(|group_id| !group_id.trim().is_empty())
}

fn event_matches_turn(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    let event_thread = params.get("threadId").and_then(Value::as_str);
    let event_turn = params
        .get("turnId")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/turn/id").and_then(Value::as_str));
    event_thread.is_none_or(|value| value == thread_id)
        && event_turn.is_none_or(|value| value == turn_id)
}

fn item_activity(params: &Value, completed: bool) -> Option<String> {
    let item = params.get("item")?;
    let item_type = item.get("type").and_then(Value::as_str)?;
    let suffix = if completed { "完成" } else { "执行中" };
    Some(match item_type {
        "commandExecution" => {
            let command = item
                .get("command")
                .and_then(Value::as_str)
                .map(|value| sanitize_weixin_system_text(value, 120))
                .unwrap_or_else(|| "命令".to_string());
            format!("命令{suffix}：{command}")
        }
        "fileChange" => format!("文件修改{suffix}"),
        "mcpToolCall" => {
            let tool = item
                .get("tool")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("MCP 工具");
            format!("工具{suffix}：{tool}")
        }
        "webSearch" => format!("网页搜索{suffix}"),
        "agentMessage" => "Codex 正在整理回复".to_string(),
        _ => format!("{item_type} {suffix}"),
    })
}

fn turn_completion_activity(params: &Value) -> String {
    let status = params
        .pointer("/turn/status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    match status {
        "failed" => "Codex 回合失败".to_string(),
        "interrupted" => "Codex 回合已中断".to_string(),
        _ => "Codex 回合已完成".to_string(),
    }
}

fn final_agent_message(params: &Value) -> Option<String> {
    params
        .pointer("/turn/items")
        .and_then(Value::as_array)?
        .iter()
        .rev()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage"))
        .and_then(|item| item.get("text").and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn approval_kind(method: &str) -> &'static str {
    if method.to_ascii_lowercase().contains("permission") {
        return "权限请求";
    }
    match method {
        "item/fileChange/requestApproval" | "applyPatchApproval" => "文件修改",
        _ => "命令执行",
    }
}

fn approval_summary(method: &str, params: &Value) -> String {
    if method.to_ascii_lowercase().contains("permission") {
        return params
            .get("reason")
            .or_else(|| params.get("message"))
            .and_then(Value::as_str)
            .map(|value| sanitize_weixin_system_text(value, 500))
            .unwrap_or_else(|| "Codex 请求额外权限。".to_string());
    }
    if matches!(
        method,
        "item/fileChange/requestApproval" | "applyPatchApproval"
    ) {
        return params
            .get("reason")
            .and_then(Value::as_str)
            .map(|value| sanitize_weixin_system_text(value, 500))
            .unwrap_or_else(|| "Codex 请求修改工作区文件。".to_string());
    }
    params
        .get("command")
        .map(|command| match command {
            Value::String(command) => command.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
            _ => command.to_string(),
        })
        .map(|value| sanitize_weixin_system_text(&value, 700))
        .unwrap_or_else(|| "Codex 请求执行一条需要额外权限的命令。".to_string())
}

fn remote_path_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("未知目录")
        .to_string()
}

fn sanitize_weixin_system_text(value: &str, limit: usize) -> String {
    let sanitized = sanitize_text(value, limit.saturating_mul(2).max(limit));
    let shortened = abbreviate_home_paths(&sanitized);
    tail_chars(&shortened, limit)
}

fn abbreviate_home_paths(value: &str) -> String {
    let mut output = value.to_string();
    for variable in ["HOME", "USERPROFILE"] {
        if let Some(home) = std::env::var_os(variable)
            && let Some(home) = home.to_str()
            && !home.trim().is_empty()
        {
            output = output.replace(home, "~");
        }
    }
    for marker in ["/Users/", "/home/"] {
        while let Some(start) = output.find(marker) {
            let username_start = start + marker.len();
            let username_length = output[username_start..]
                .char_indices()
                .take_while(|(_, character)| {
                    !character.is_whitespace()
                        && !matches!(character, '/' | '\\' | '"' | '\'' | '`' | ',' | ';')
                })
                .map(|(_, character)| character.len_utf8())
                .sum::<usize>();
            if username_length == 0 {
                break;
            }
            output.replace_range(start..username_start + username_length, "~");
        }
    }
    output
}

fn approval_result(method: &str, approve: bool) -> Value {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({ "decision": if approve { "accept" } else { "decline" } })
        }
        _ if approve => json!({ "decision": "approved" }),
        _ => json!({
            "decision": {
                "denied": { "rejection": "用户从微信拒绝了该操作。" }
            }
        }),
    }
}

fn base_info() -> Value {
    json!({
        "channel_version": WEIXIN_CHANNEL_VERSION,
        "bot_agent": format!("CodexXRay/{}", env!("CARGO_PKG_VERSION"))
    })
}

fn endpoint_url(base_url: &str, endpoint: &str) -> Result<url::Url, String> {
    let base = normalize_weixin_base_url(base_url)?;
    let base = url::Url::parse(&format!("{}/", base.trim_end_matches('/')))
        .map_err(|error| format!("微信服务地址无效：{error}"))?;
    base.join(endpoint)
        .map_err(|error| format!("微信接口地址无效：{error}"))
}

fn normalize_weixin_base_url(value: &str) -> Result<String, String> {
    let url = url::Url::parse(value).map_err(|error| format!("微信服务地址无效：{error}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| "微信服务地址缺少域名。".to_string())?;
    if url.scheme() != "https" || !(host == "weixin.qq.com" || host.ends_with(".weixin.qq.com")) {
        return Err("微信服务地址必须使用 weixin.qq.com 的 HTTPS 域名。".to_string());
    }
    Ok(format!("https://{host}"))
}

async fn parse_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, String> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("无法读取微信通道响应：{error}"))?;
    if status != StatusCode::OK {
        return Err(format!(
            "微信通道返回 HTTP {status}：{}",
            tail_chars(&text, 800)
        ));
    }
    serde_json::from_str(&text).map_err(|error| format!("微信通道响应无法解析：{error}"))
}

fn ensure_api_success(response: &WeixinApiResponse, label: &str) -> Result<(), String> {
    let ret = response.ret.unwrap_or(0);
    let errcode = response.errcode.unwrap_or(0);
    if ret == 0 && errcode == 0 {
        Ok(())
    } else {
        Err(format!(
            "微信 {label} 失败：ret={ret} errcode={errcode} {}",
            response.errmsg.as_deref().unwrap_or_default()
        ))
    }
}

fn render_qr_svg(content: &str) -> Result<String, String> {
    let code =
        QrCode::new(content.as_bytes()).map_err(|error| format!("无法生成二维码：{error}"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(232, 232)
        .dark_color(svg::Color("#101410"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

fn split_text(value: &str, max_chars: usize) -> Vec<String> {
    if value.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if current.chars().count() >= max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(character);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn trim_front_chars(value: &mut String, max_chars: usize) {
    let count = value.chars().count();
    if count > max_chars {
        *value = value.chars().skip(count - max_chars).collect();
    }
}

fn tail_chars(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        value.to_string()
    } else {
        format!(
            "…{}",
            value.chars().skip(count - max_chars).collect::<String>()
        )
    }
}

fn format_elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds} 秒")
    } else {
        let minutes = seconds / 60;
        let remainder = seconds % 60;
        if remainder == 0 {
            format!("{minutes} 分钟")
        } else {
            format!("{minutes} 分 {remainder} 秒")
        }
    }
}

fn short_id(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}

fn path_title(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
}

fn next_client_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = CLIENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("codex-xray-{now}-{sequence}")
}

fn random_wechat_uin() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = CLIENT_SEQUENCE.fetch_add(1, Ordering::Relaxed) as u128;
    let value = ((now ^ sequence.rotate_left(17)) & u32::MAX as u128) as u32;
    base64::engine::general_purpose::STANDARD.encode(value.to_string().as_bytes())
}

fn normalize_identifier(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| match character {
            '@' | '.' => '-',
            value if value.is_ascii_alphanumeric() || matches!(value, '_' | '-') => value,
            _ => '-',
        })
        .collect()
}

fn default_base_url() -> String {
    WEIXIN_BASE_URL.to_string()
}

fn read_config(path: &Path) -> Result<RemoteChannelFile, String> {
    if !path.is_file() {
        return Ok(RemoteChannelFile::default());
    }
    let content = fs::read(path).map_err(|error| format!("无法读取微信通道配置：{error}"))?;
    let file: RemoteChannelFile = serde_json::from_slice(&content)
        .map_err(|error| format!("微信通道配置无法解析：{error}"))?;
    if file.version != 1 {
        return Err(format!("不支持微信通道配置版本 {}。", file.version));
    }
    Ok(file)
}

fn save_config(path: &Path, file: &RemoteChannelFile) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "无法确定微信通道配置目录。".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建微信通道配置目录：{error}"))?;
    let temporary = path.with_extension("tmp");
    let content = serde_json::to_vec_pretty(file)
        .map_err(|error| format!("无法序列化微信通道配置：{error}"))?;
    fs::write(&temporary, content).map_err(|error| format!("无法保存微信通道配置：{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("无法保护微信通道配置：{error}"))?;
    }
    fs::rename(&temporary, path).map_err(|error| format!("无法保存微信通道配置：{error}"))
}

fn read_secret(path: &Path) -> Result<String, String> {
    let value =
        fs::read_to_string(path).map_err(|error| format!("无法读取微信登录令牌：{error}"))?;
    let value = value.trim().to_string();
    if value.is_empty() {
        Err("微信登录令牌为空。".to_string())
    } else {
        Ok(value)
    }
}

fn save_secret(path: &Path, value: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "无法确定微信登录令牌目录。".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建微信令牌目录：{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("无法限制微信令牌目录权限：{error}"))?;
        let temporary = path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| format!("无法创建微信令牌文件：{error}"))?;
        file.write_all(value.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("无法写入微信登录令牌：{error}"))?;
        fs::rename(&temporary, path).map_err(|error| format!("无法保存微信登录令牌：{error}"))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("无法限制微信令牌文件权限：{error}"))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, value)
            .and_then(|_| fs::rename(&temporary, path))
            .map_err(|error| format!("无法保存微信登录令牌：{error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_weixin_base_urls() {
        assert!(normalize_weixin_base_url("http://ilinkai.weixin.qq.com").is_err());
        assert!(normalize_weixin_base_url("https://example.com").is_err());
        assert_eq!(
            normalize_weixin_base_url("https://ilinkai.weixin.qq.com/path").unwrap(),
            WEIXIN_BASE_URL
        );
    }

    #[test]
    fn splits_unicode_without_breaking_characters() {
        assert_eq!(split_text("你好世界", 2), vec!["你好", "世界"]);
    }

    #[test]
    fn progress_feedback_waits_deduplicates_and_sends_heartbeats() {
        let started_at = Instant::now();
        let mut feedback = ProgressFeedback {
            started_at,
            last_sent_at: None,
            last_signature: String::new(),
        };
        assert!(!feedback.should_send(
            started_at + PROGRESS_FIRST_FEEDBACK - Duration::from_millis(1),
            "正在检查"
        ));
        let first = started_at + PROGRESS_FIRST_FEEDBACK;
        assert!(feedback.should_send(first, "正在检查"));
        feedback.mark_sent(first, "正在检查".to_string());
        assert!(!feedback.should_send(first + PROGRESS_MIN_INTERVAL, "正在检查"));
        assert!(feedback.should_send(first + PROGRESS_MIN_INTERVAL, "正在运行测试"));
        feedback.mark_sent(first + PROGRESS_MIN_INTERVAL, "正在运行测试".to_string());
        assert!(feedback.should_send(
            first + PROGRESS_MIN_INTERVAL + PROGRESS_HEARTBEAT_INTERVAL,
            "正在运行测试"
        ));
    }

    #[test]
    fn formats_short_and_long_progress_elapsed_time() {
        assert_eq!(format_elapsed(Duration::from_secs(12)), "12 秒");
        assert_eq!(format_elapsed(Duration::from_secs(60)), "1 分钟");
        assert_eq!(format_elapsed(Duration::from_secs(82)), "1 分 22 秒");
    }

    #[test]
    fn formats_new_and_legacy_approval_results() {
        assert_eq!(
            approval_result("item/commandExecution/requestApproval", true),
            json!({ "decision": "accept" })
        );
        assert_eq!(
            approval_result("applyPatchApproval", false),
            json!({
                "decision": {
                    "denied": { "rejection": "用户从微信拒绝了该操作。" }
                }
            })
        );
    }

    #[test]
    fn extracts_text_and_voice_messages() {
        let text: WeixinMessage = serde_json::from_value(json!({
            "item_list": [{ "type": 1, "text_item": { "text": "/tasks" } }]
        }))
        .unwrap();
        assert_eq!(message_text(&text).as_deref(), Some("/tasks"));
        let voice: WeixinMessage = serde_json::from_value(json!({
            "item_list": [{ "type": 3, "voice_item": { "text": "查看状态" } }]
        }))
        .unwrap();
        assert_eq!(message_text(&voice).as_deref(), Some("查看状态"));
    }

    #[test]
    fn accepts_direct_messages_with_an_empty_group_id() {
        let direct: WeixinMessage = serde_json::from_value(json!({
            "message_type": 1,
            "group_id": "",
            "from_user_id": "owner@im.wechat"
        }))
        .unwrap();
        assert!(is_direct_user_message(&direct));

        let group: WeixinMessage = serde_json::from_value(json!({
            "message_type": 1,
            "group_id": "group-1",
            "from_user_id": "owner@im.wechat"
        }))
        .unwrap();
        assert!(!is_direct_user_message(&group));

        let bot: WeixinMessage = serde_json::from_value(json!({
            "message_type": 2,
            "group_id": ""
        }))
        .unwrap();
        assert!(!is_direct_user_message(&bot));
    }

    #[test]
    fn normalizes_full_width_slashes_and_suggests_typo_fixes() {
        assert_eq!(normalize_command_token("／STATUS"), "/status");
        assert!(unknown_command_reply("/stats").contains("/status"));
        assert!(unknown_command_reply("/something").contains("不要以 / 开头"));
    }

    #[test]
    fn active_writer_conflicts_never_offer_or_create_a_fork() {
        let message =
            attach_error_message("thread/resume failed: thread abc already has an active writer");
        assert!(message.contains("IPC"));
        assert!(message.contains("没有创建副本或新会话"));
        assert!(!message.contains("/new"));
    }

    #[test]
    fn desktop_connection_errors_are_reported_without_guessing_a_version_problem() {
        let message = desktop_control_unavailable_message(Some("读取 IPC 内容失败: closed"));
        assert!(message.contains("读取 IPC 内容失败: closed"));
        assert!(message.contains("重试自动连接"));
        assert!(message.contains("没有创建副本或新会话"));
        assert!(!message.contains("版本不兼容"));
    }

    #[test]
    fn codex_thread_deep_links_only_accept_safe_task_ids() {
        assert_eq!(
            codex_thread_deep_link("019fea56-2aa8-7101-aadb-d7fd340fc913").unwrap(),
            "codex://threads/019fea56-2aa8-7101-aadb-d7fd340fc913"
        );
        assert!(codex_thread_deep_link("../settings").is_err());
        assert!(codex_thread_deep_link("thread\nopen").is_err());
        assert!(codex_thread_deep_link("").is_err());
    }

    #[test]
    fn task_list_invites_a_direct_number_selection() {
        let body = format_task_list(&[RemoteTaskSummary {
            id: "thread-1".to_string(),
            title: "修复登录".to_string(),
            cwd: "/tmp/project".to_string(),
            status: "idle".to_string(),
            updated_at: 0,
            control_mode: "available".to_string(),
        }]);
        assert!(body.contains("1. [空闲] 修复登录"));
        assert!(body.contains("直接回复序号即可切换"));
        assert!(!body.contains("/attach"));
    }

    #[test]
    fn task_messages_do_not_expose_parent_paths() {
        let body = format_task_list(&[RemoteTaskSummary {
            id: "thread-1".to_string(),
            title: "Project task".to_string(),
            cwd: "/Users/private-user/Documents/SecretProject".to_string(),
            status: "idle".to_string(),
            updated_at: 0,
            control_mode: "available".to_string(),
        }]);
        assert!(body.contains("SecretProject"));
        assert!(!body.contains("/Users/"));
        assert!(!body.contains("private-user"));
    }

    #[test]
    fn system_messages_redact_paths_and_credentials() {
        let body = sanitize_weixin_system_text(
            "run --api-key actual-secret /Users/private-user/Documents/project Authorization: Bearer token-value",
            700,
        );
        assert!(body.contains("[redacted]"));
        assert!(!body.contains("actual-secret"));
        assert!(!body.contains("token-value"));
        assert!(!body.contains("private-user"));
        assert!(!body.contains("/Users/"));
        assert!(body.contains("~/Documents/project"));
    }

    #[test]
    fn detects_recent_open_turns_without_treating_completed_turns_as_active() {
        let path = std::env::temp_dir().join(format!(
            "codex-xray-remote-session-{}.jsonl",
            next_client_id()
        ));
        fs::write(
            &path,
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-1\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{}}\n"
            ),
        )
        .unwrap();
        assert!(session_has_recent_open_turn(&path));
        fs::write(
            &path,
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-1\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"turn-1\"}}\n"
            ),
        )
        .unwrap();
        assert!(!session_has_recent_open_turn(&path));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn extracts_the_matching_completed_turn_for_restart_recovery() {
        let path = std::env::temp_dir().join(format!(
            "codex-xray-recovered-turn-{}.jsonl",
            next_client_id()
        ));
        fs::write(
            &path,
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"other-turn\",\"last_agent_message\":\"wrong\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"wanted-turn\",\"last_agent_message\":\"今天是星期三。\"}}\n"
            ),
        )
        .unwrap();
        assert_eq!(
            session_turn_outcome(&path, "wanted-turn"),
            Some(SessionTurnOutcome::Completed("今天是星期三。".to_string()))
        );
        assert_eq!(session_turn_outcome(&path, "missing-turn"), None);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_older_channel_configs_without_pending_turn_fields() {
        let config: RemoteChannelFile = serde_json::from_value(json!({
            "version": 1,
            "enabled": true,
            "account_id": "bot",
            "owner_user_id": "owner",
            "base_url": "https://ilinkai.weixin.qq.com",
            "get_updates_buf": null,
            "attached_thread_id": "thread",
            "attached_thread_title": "title",
            "attached_cwd": "/tmp"
        }))
        .unwrap();
        assert_eq!(config.pending_turn_id, None);
        assert_eq!(config.pending_reply_context_token, None);
    }

    #[test]
    #[ignore = "requires a running Codex Desktop task and CODEX_XRAY_TEST_THREAD_ID"]
    fn attaches_the_exact_live_desktop_task_without_an_app_server_copy() {
        let thread_id = std::env::var("CODEX_XRAY_TEST_THREAD_ID")
            .expect("set CODEX_XRAY_TEST_THREAD_ID to a live Desktop task");
        let root = std::env::temp_dir().join(format!("codex-xray-ipc-smoke-{}", next_client_id()));
        fs::create_dir_all(&root).unwrap();
        let channel =
            RemoteChannelState::load(&root, Arc::new(Mutex::new(None)), root.join("codex-state"))
                .unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let snapshot = runtime.block_on(channel.attach_thread(&thread_id)).unwrap();
        assert_eq!(
            snapshot.attached_thread_id.as_deref(),
            Some(thread_id.as_str())
        );
        assert_eq!(snapshot.control_backend, "desktop_ipc");
        assert!(snapshot.control_ready);
        drop(channel);
        drop(runtime);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "sends a real WeChat reply and requires CODEX_XRAY_LIVE_APP_DATA"]
    fn relays_a_live_wechat_prompt_through_the_exact_desktop_task() {
        let app_data = PathBuf::from(
            std::env::var("CODEX_XRAY_LIVE_APP_DATA")
                .expect("set CODEX_XRAY_LIVE_APP_DATA to the X-Ray app data directory"),
        );
        let config = read_config(&app_data.join("remote-channel.json")).unwrap();
        let thread_id = config
            .attached_thread_id
            .expect("select a Desktop task in X-Ray before running the live test");
        let owner_user_id = config
            .owner_user_id
            .expect("bind a WeChat user before running the live test");
        let channel = RemoteChannelState::load(
            &app_data,
            Arc::new(Mutex::new(None)),
            app_data.join("codex-state"),
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async {
            let attached = channel.attach_thread(&thread_id).await.unwrap();
            assert_eq!(attached.control_backend, "desktop_ipc");
            assert!(attached.control_ready);

            let message: WeixinMessage = serde_json::from_value(json!({
                "client_id": next_client_id(),
                "message_type": 1,
                "group_id": "",
                "from_user_id": owner_user_id,
                "item_list": [{
                    "type": 1,
                    "text_item": {
                        "text": "这是 X-Ray 端到端测试。请只回复 XRAY_E2E_OK，不要修改文件。"
                    }
                }]
            }))
            .unwrap();
            channel.handle_inbound(message).await.unwrap();
            let turn_id = channel
                .snapshot()
                .active_turn_id
                .expect("the live prompt did not start a Codex turn");

            let deadline = Instant::now() + Duration::from_secs(180);
            loop {
                let snapshot = channel.snapshot();
                if snapshot.active_turn_id.is_none() {
                    assert!(snapshot.last_outbound_at.is_some());
                    assert!(snapshot.last_error.is_none(), "{:?}", snapshot.last_error);
                    assert!(
                        snapshot
                            .latest_activity
                            .as_deref()
                            .is_some_and(|activity| activity.contains("完成")),
                        "unexpected final activity: {:?}",
                        snapshot.latest_activity
                    );
                    let session_path = channel
                        .session_path_for_thread(&thread_id)
                        .await
                        .expect("live task session path was not found");
                    assert!(
                        matches!(
                            session_turn_outcome(&session_path, &turn_id),
                            Some(SessionTurnOutcome::Completed(text))
                                if text.contains("XRAY_E2E_OK")
                        ),
                        "Desktop session did not contain the expected final reply"
                    );
                    break;
                }
                assert!(Instant::now() < deadline, "live WeChat relay timed out");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }
}
