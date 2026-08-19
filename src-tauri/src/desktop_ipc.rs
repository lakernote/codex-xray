//! Follower client for the private local IPC router exposed by Codex Desktop.
//!
//! This is deliberately fail-closed: it never starts or resumes a second App Server
//! when Desktop owns a task. If the local protocol changes, callers receive an error
//! instead of silently creating a duplicate conversation.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

const HOST_ID: &str = "local";
// Desktop sends a complete conversation snapshot when a follower attaches. Long-lived
// tasks can legitimately exceed 64 MiB, so keep a generous but finite local safety cap.
const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(20);
static IPC_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopTurnOutcome {
    Completed(String),
    Aborted,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct DesktopPendingRequest {
    pub request_id: Value,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Default)]
pub struct DesktopThreadView {
    pub active_turn_id: Option<String>,
    pub agent_preview: Option<String>,
    pub pending_request: Option<DesktopPendingRequest>,
    pub outcome: Option<DesktopTurnOutcome>,
}

#[derive(Debug, Clone)]
struct ThreadMirror {
    owner_client_id: String,
    revision: u64,
    state: Value,
}

#[cfg(unix)]
pub struct DesktopIpcClient {
    writer: Arc<Mutex<std::os::unix::net::UnixStream>>,
    receiver: mpsc::Receiver<Result<Value, String>>,
    deferred: VecDeque<Value>,
    client_id: String,
    mirrors: HashMap<String, ThreadMirror>,
}

#[cfg(not(unix))]
pub struct DesktopIpcClient;

impl DesktopIpcClient {
    #[cfg(unix)]
    pub fn connect() -> Result<Self, String> {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        use std::os::unix::net::UnixStream;

        let socket_path = ipc_socket_path()?;
        let metadata = fs::symlink_metadata(&socket_path).map_err(|error| {
            format!(
                "没有找到 Codex Desktop IPC（{}）：{error}",
                socket_path.display()
            )
        })?;
        if !metadata.file_type().is_socket() {
            return Err(format!(
                "Codex Desktop IPC 路径不是本机套接字：{}",
                socket_path.display()
            ));
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "拒绝连接权限过宽的 Codex Desktop IPC：{}",
                socket_path.display()
            ));
        }

        let stream = UnixStream::connect(&socket_path)
            .map_err(|error| format!("无法连接 Codex Desktop IPC：{error}"))?;
        stream
            .set_write_timeout(Some(DEFAULT_TIMEOUT))
            .map_err(|error| format!("无法配置 Codex Desktop IPC：{error}"))?;
        let reader = stream
            .try_clone()
            .map_err(|error| format!("无法复制 Codex Desktop IPC：{error}"))?;
        let writer = Arc::new(Mutex::new(stream));
        let (sender, receiver) = mpsc::channel();
        spawn_reader(reader, Arc::clone(&writer), sender);

        let mut client = Self {
            writer,
            receiver,
            deferred: VecDeque::new(),
            client_id: "initializing-client".to_string(),
            mirrors: HashMap::new(),
        };
        let response = client.request_with_source(
            "initialize",
            0,
            json!({ "clientType": "codex-xray" }),
            None,
            DEFAULT_TIMEOUT,
            "initializing-client",
        )?;
        client.client_id = response
            .get("clientId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Codex Desktop IPC 初始化响应缺少 clientId".to_string())?
            .to_string();
        Ok(client)
    }

    #[cfg(not(unix))]
    pub fn connect() -> Result<Self, String> {
        Err("当前系统暂不支持 Codex Desktop IPC 控制".to_string())
    }

    #[cfg(unix)]
    pub fn discover_owner(&mut self, conversation_id: &str) -> Result<Option<String>, String> {
        let request_id = next_ipc_id();
        let message = json!({
            "type": "request",
            "requestId": request_id,
            "sourceClientId": self.client_id,
            "version": 1,
            "method": "thread-owner-discovery",
            "params": thread_params(conversation_id),
            "timeoutMs": 3_000
        });
        send_value(&self.writer, &message)?;
        match self.wait_for_response(&request_id, Duration::from_secs(3)) {
            Ok(response) => {
                if response.get("resultType").and_then(Value::as_str) == Some("error") {
                    let error = response
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex Desktop owner discovery failed");
                    return if is_no_owner_error(error) {
                        Ok(None)
                    } else {
                        Err(error.to_string())
                    };
                }
                Ok(response
                    .get("handledByClientId")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned))
            }
            Err(error) if is_no_owner_error(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    #[cfg(not(unix))]
    pub fn discover_owner(&mut self, _conversation_id: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    #[cfg(unix)]
    pub fn follow(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
    ) -> Result<DesktopThreadView, String> {
        self.broadcast_following(conversation_id, owner_client_id, true)?;
        let deadline = SystemTime::now() + SNAPSHOT_TIMEOUT;
        loop {
            if let Some(mirror) = self.mirrors.get(conversation_id) {
                return Ok(thread_view(&mirror.state, None));
            }
            let remaining = deadline
                .duration_since(SystemTime::now())
                .unwrap_or_default();
            if remaining.is_zero() {
                return Err(
                    "Codex Desktop 已声明任务 owner，但 20 秒内没有返回任务快照".to_string()
                );
            }
            let message = self
                .receiver
                .recv_timeout(remaining)
                .map_err(|_| "等待 Codex Desktop 任务快照超时".to_string())??;
            self.process_message(message)?;
        }
    }

    #[cfg(not(unix))]
    pub fn follow(
        &mut self,
        _conversation_id: &str,
        _owner_client_id: &str,
    ) -> Result<DesktopThreadView, String> {
        Err("当前系统暂不支持 Codex Desktop IPC 控制".to_string())
    }

    #[cfg(unix)]
    pub fn unfollow(&mut self, conversation_id: &str) -> Result<(), String> {
        if let Some(mirror) = self.mirrors.remove(conversation_id) {
            self.broadcast_following(conversation_id, &mirror.owner_client_id, false)?;
        }
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn unfollow(&mut self, _conversation_id: &str) -> Result<(), String> {
        Ok(())
    }

    #[cfg(unix)]
    pub fn start_turn(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
        prompt: &str,
        client_message_id: &str,
    ) -> Result<String, String> {
        let response = self.request(
            "thread-follower-start-turn",
            1,
            json!({
                "conversationId": conversation_id,
                "turnStartParams": {
                    "input": [{
                        "type": "text",
                        "text": prompt,
                        "text_elements": []
                    }],
                    "clientUserMessageId": client_message_id,
                    "approvalPolicy": "on-request"
                },
                "localTurnMetadata": { "fileAttachmentCount": 0 },
                "mcpAppModelContextAttachments": []
            }),
            owner_client_id,
            Duration::from_secs(15),
        )?;
        if let Some(turn_id) = find_turn_id(&response) {
            return Ok(turn_id);
        }
        self.refresh_and_wait_for_active(conversation_id, owner_client_id)
    }

    #[cfg(not(unix))]
    pub fn start_turn(
        &mut self,
        _conversation_id: &str,
        _owner_client_id: &str,
        _prompt: &str,
        _client_message_id: &str,
    ) -> Result<String, String> {
        Err("当前系统暂不支持 Codex Desktop IPC 控制".to_string())
    }

    #[cfg(unix)]
    pub fn steer_turn(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
        prompt: &str,
        client_message_id: &str,
    ) -> Result<(), String> {
        self.request(
            "thread-follower-steer-turn",
            1,
            json!({
                "conversationId": conversation_id,
                "input": [{
                    "type": "text",
                    "text": prompt,
                    "text_elements": []
                }],
                "restoreMessage": false,
                "serviceTier": null,
                "attachments": [],
                "clientUserMessageId": client_message_id,
                "additionalContext": null
            }),
            owner_client_id,
            Duration::from_secs(15),
        )?;
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn steer_turn(
        &mut self,
        _conversation_id: &str,
        _owner_client_id: &str,
        _prompt: &str,
        _client_message_id: &str,
    ) -> Result<(), String> {
        Err("当前系统暂不支持 Codex Desktop IPC 控制".to_string())
    }

    #[cfg(unix)]
    pub fn interrupt_turn(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
        turn_id: &str,
    ) -> Result<(), String> {
        self.request(
            "thread-follower-interrupt-turn",
            4,
            json!({
                "conversationId": conversation_id,
                "mode": "user-stop",
                "expectedTurnId": turn_id
            }),
            owner_client_id,
            DEFAULT_TIMEOUT,
        )?;
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn interrupt_turn(
        &mut self,
        _conversation_id: &str,
        _owner_client_id: &str,
        _turn_id: &str,
    ) -> Result<(), String> {
        Err("当前系统暂不支持 Codex Desktop IPC 控制".to_string())
    }

    #[cfg(unix)]
    pub fn resolve_approval(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
        request_id: Value,
        request_method: &str,
        approve: bool,
    ) -> Result<(), String> {
        let (method, params) = if request_method.to_ascii_lowercase().contains("file") {
            (
                "thread-follower-file-approval-decision",
                json!({
                    "conversationId": conversation_id,
                    "requestId": request_id,
                    "decision": if approve { "accept" } else { "decline" }
                }),
            )
        } else if request_method.to_ascii_lowercase().contains("permission") {
            (
                "thread-follower-permissions-request-approval-response",
                json!({
                    "conversationId": conversation_id,
                    "requestId": request_id,
                    "response": { "decision": if approve { "accept" } else { "decline" } }
                }),
            )
        } else {
            (
                "thread-follower-command-approval-decision",
                json!({
                    "conversationId": conversation_id,
                    "requestId": request_id,
                    "decision": if approve { "accept" } else { "decline" }
                }),
            )
        };
        self.request(method, 1, params, owner_client_id, DEFAULT_TIMEOUT)?;
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn resolve_approval(
        &mut self,
        _conversation_id: &str,
        _owner_client_id: &str,
        _request_id: Value,
        _request_method: &str,
        _approve: bool,
    ) -> Result<(), String> {
        Err("当前系统暂不支持 Codex Desktop IPC 控制".to_string())
    }

    #[cfg(unix)]
    pub fn receive_update(
        &mut self,
        conversation_id: &str,
        expected_turn_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Option<DesktopThreadView>, String> {
        let deadline = SystemTime::now() + timeout;
        loop {
            let remaining = deadline
                .duration_since(SystemTime::now())
                .unwrap_or_default();
            if remaining.is_zero() {
                return Ok(None);
            }
            let message = match self.receiver.recv_timeout(remaining) {
                Ok(message) => message?,
                Err(mpsc::RecvTimeoutError::Timeout) => return Ok(None),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Codex Desktop IPC 已断开".to_string());
                }
            };
            let changed = self.process_message(message)?;
            if changed.as_deref() == Some(conversation_id)
                && let Some(mirror) = self.mirrors.get(conversation_id)
            {
                return Ok(Some(thread_view(&mirror.state, expected_turn_id)));
            }
        }
    }

    #[cfg(not(unix))]
    pub fn receive_update(
        &mut self,
        _conversation_id: &str,
        _expected_turn_id: Option<&str>,
        _timeout: Duration,
    ) -> Result<Option<DesktopThreadView>, String> {
        Ok(None)
    }

    #[cfg(unix)]
    pub fn current_view(
        &self,
        conversation_id: &str,
        expected_turn_id: Option<&str>,
    ) -> Option<DesktopThreadView> {
        self.mirrors
            .get(conversation_id)
            .map(|mirror| thread_view(&mirror.state, expected_turn_id))
    }

    #[cfg(not(unix))]
    pub fn current_view(
        &self,
        _conversation_id: &str,
        _expected_turn_id: Option<&str>,
    ) -> Option<DesktopThreadView> {
        None
    }
}

#[cfg(unix)]
impl DesktopIpcClient {
    fn request(
        &mut self,
        method: &str,
        version: u32,
        params: Value,
        target_client_id: &str,
        timeout: Duration,
    ) -> Result<Value, String> {
        let source = self.client_id.clone();
        self.request_with_source(
            method,
            version,
            params,
            Some(target_client_id),
            timeout,
            &source,
        )
    }

    fn request_with_source(
        &mut self,
        method: &str,
        version: u32,
        params: Value,
        target_client_id: Option<&str>,
        timeout: Duration,
        source_client_id: &str,
    ) -> Result<Value, String> {
        let request_id = next_ipc_id();
        let mut message = json!({
            "type": "request",
            "requestId": request_id,
            "sourceClientId": source_client_id,
            "version": version,
            "method": method,
            "params": params,
            "timeoutMs": timeout.as_millis().min(u64::MAX as u128) as u64
        });
        if let Some(target) = target_client_id {
            message["targetClientId"] = Value::String(target.to_string());
        }
        send_value(&self.writer, &message)?;
        let response = self.wait_for_response(&request_id, timeout)?;
        if response.get("resultType").and_then(Value::as_str) == Some("error") {
            return Err(response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Codex Desktop IPC 请求失败")
                .to_string());
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    fn wait_for_response(&mut self, request_id: &str, timeout: Duration) -> Result<Value, String> {
        if let Some(index) = self.deferred.iter().position(|message| {
            message.get("requestId").and_then(Value::as_str) == Some(request_id)
        }) {
            return Ok(self
                .deferred
                .remove(index)
                .expect("deferred response exists"));
        }
        let deadline = SystemTime::now() + timeout;
        loop {
            let remaining = deadline
                .duration_since(SystemTime::now())
                .unwrap_or_default();
            if remaining.is_zero() {
                return Err("Codex Desktop IPC 请求超时".to_string());
            }
            let message =
                self.receiver
                    .recv_timeout(remaining)
                    .map_err(|error| match error {
                        mpsc::RecvTimeoutError::Timeout => "Codex Desktop IPC 请求超时".to_string(),
                        mpsc::RecvTimeoutError::Disconnected => {
                            "Codex Desktop IPC 已断开".to_string()
                        }
                    })??;
            if message.get("requestId").and_then(Value::as_str) == Some(request_id)
                && message.get("type").and_then(Value::as_str) == Some("response")
            {
                return Ok(message);
            }
            if message.get("type").and_then(Value::as_str) == Some("broadcast") {
                self.process_message(message)?;
            } else {
                self.deferred.push_back(message);
            }
        }
    }

    fn broadcast_following(
        &self,
        conversation_id: &str,
        owner_client_id: &str,
        following: bool,
    ) -> Result<(), String> {
        send_value(
            &self.writer,
            &json!({
                "type": "broadcast",
                "method": "thread-stream-following-changed",
                "sourceClientId": self.client_id,
                "targetClientIds": [owner_client_id],
                "params": {
                    "hostId": HOST_ID,
                    "conversationId": conversation_id,
                    "following": following
                },
                "version": 1
            }),
        )
    }

    fn refresh_and_wait_for_active(
        &mut self,
        conversation_id: &str,
        owner_client_id: &str,
    ) -> Result<String, String> {
        self.broadcast_following(conversation_id, owner_client_id, true)?;
        let deadline = SystemTime::now() + DEFAULT_TIMEOUT;
        loop {
            if let Some(turn_id) = self
                .current_view(conversation_id, None)
                .and_then(|view| view.active_turn_id)
            {
                return Ok(turn_id);
            }
            let remaining = deadline
                .duration_since(SystemTime::now())
                .unwrap_or_default();
            if remaining.is_zero() {
                return Err("Codex Desktop 已接收指令，但未返回新回合 ID".to_string());
            }
            let message = self
                .receiver
                .recv_timeout(remaining)
                .map_err(|_| "等待 Codex Desktop 新回合超时".to_string())??;
            self.process_message(message)?;
        }
    }

    fn process_message(&mut self, mut message: Value) -> Result<Option<String>, String> {
        if message.get("type").and_then(Value::as_str) != Some("broadcast")
            || message.get("method").and_then(Value::as_str) != Some("thread-stream-state-changed")
        {
            return Ok(None);
        }
        let source_client_id = message
            .get("sourceClientId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let params = message
            .get_mut("params")
            .ok_or_else(|| "Codex Desktop 任务状态通知缺少 params".to_string())?;
        let conversation_id = params
            .get("conversationId")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex Desktop 任务状态通知缺少 conversationId".to_string())?
            .to_string();
        let owner_client_id = source_client_id
            .or_else(|| {
                self.mirrors
                    .get(&conversation_id)
                    .map(|mirror| mirror.owner_client_id.clone())
            })
            .ok_or_else(|| "Codex Desktop 任务状态通知缺少 owner".to_string())?
            .to_string();
        let change = params
            .get_mut("change")
            .ok_or_else(|| "Codex Desktop 任务状态通知缺少 change".to_string())?;
        let revision = change.get("revision").and_then(Value::as_u64).unwrap_or(0);
        match change.get("type").and_then(Value::as_str) {
            Some("snapshot") => {
                let state = change
                    .get_mut("conversationState")
                    .map(Value::take)
                    .ok_or_else(|| "Codex Desktop 任务快照缺少 conversationState".to_string())?;
                self.mirrors.insert(
                    conversation_id.clone(),
                    ThreadMirror {
                        owner_client_id,
                        revision,
                        state,
                    },
                );
            }
            Some(kind) if is_incremental_state_change(kind) => {
                let mirror = self
                    .mirrors
                    .get_mut(&conversation_id)
                    .ok_or_else(|| "先收到 Codex Desktop 增量状态，但尚无完整快照".to_string())?;
                let patches = change
                    .get("patches")
                    .or_else(|| change.get("operations"))
                    .and_then(Value::as_array)
                    .ok_or_else(|| "Codex Desktop 增量状态缺少 patches".to_string())?;
                apply_patches(&mut mirror.state, patches)?;
                mirror.revision = revision;
                mirror.owner_client_id = owner_client_id;
            }
            Some(kind) => return Err(format!("不支持的 Codex Desktop 状态类型：{kind}")),
            None => return Err("Codex Desktop 状态通知缺少类型".to_string()),
        }
        Ok(Some(conversation_id))
    }
}

#[cfg(unix)]
fn ipc_socket_path() -> Result<PathBuf, String> {
    let root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or_else(|| "无法确定 Codex Desktop 数据目录".to_string())?;
    Ok(root.join("ipc").join("ipc.sock"))
}

#[cfg(unix)]
fn spawn_reader(
    mut reader: std::os::unix::net::UnixStream,
    writer: Arc<Mutex<std::os::unix::net::UnixStream>>,
    sender: mpsc::Sender<Result<Value, String>>,
) {
    std::thread::Builder::new()
        .name("codex-xray-desktop-ipc".to_string())
        .spawn(move || {
            loop {
                let message = match read_value(&mut reader) {
                    Ok(message) => message,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        return;
                    }
                };
                if message.get("type").and_then(Value::as_str) == Some("client-discovery-request") {
                    let Some(request_id) = message.get("requestId").cloned() else {
                        continue;
                    };
                    let response = json!({
                        "type": "client-discovery-response",
                        "requestId": request_id,
                        "response": { "canHandle": false }
                    });
                    if let Err(error) = send_value(&writer, &response) {
                        let _ = sender.send(Err(error));
                        return;
                    }
                    continue;
                }
                if sender.send(Ok(message)).is_err() {
                    return;
                }
            }
        })
        .expect("spawn Codex Desktop IPC reader");
}

#[cfg(unix)]
fn send_value(
    writer: &Arc<Mutex<std::os::unix::net::UnixStream>>,
    value: &Value,
) -> Result<(), String> {
    let payload = serde_json::to_vec(value)
        .map_err(|error| format!("无法编码 Codex Desktop IPC：{error}"))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err("Codex Desktop IPC 消息过大".to_string());
    }
    let length =
        u32::try_from(payload.len()).map_err(|_| "Codex Desktop IPC 消息长度溢出".to_string())?;
    let mut stream = writer
        .lock()
        .map_err(|_| "Codex Desktop IPC 写入锁已损坏".to_string())?;
    stream
        .write_all(&length.to_le_bytes())
        .and_then(|_| stream.write_all(&payload))
        .and_then(|_| stream.flush())
        .map_err(|error| format!("写入 Codex Desktop IPC 失败：{error}"))
}

#[cfg(unix)]
fn read_value(reader: &mut std::os::unix::net::UnixStream) -> Result<Value, String> {
    let mut length = [0_u8; 4];
    reader
        .read_exact(&mut length)
        .map_err(|error| format!("读取 Codex Desktop IPC 长度失败：{error}"))?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 {
        return Err("Codex Desktop IPC 帧长度无效：0".to_string());
    }
    if length > MAX_FRAME_BYTES {
        return Err(format!(
            "Codex Desktop IPC 快照过大：{:.1} MiB，X-Ray 安全上限为 {} MiB",
            length as f64 / (1024.0 * 1024.0),
            MAX_FRAME_BYTES / (1024 * 1024)
        ));
    }

    // Parse directly from the bounded socket reader. This avoids holding both a large
    // byte buffer and the deserialized JSON tree at the same time.
    let mut frame = reader.take(length as u64);
    let value = serde_json::from_reader(&mut frame)
        .map_err(|error| format!("解析 Codex Desktop IPC 内容失败：{error}"))?;
    if frame.limit() != 0 {
        return Err(format!(
            "读取 Codex Desktop IPC 内容不完整：还缺 {} 字节",
            frame.limit()
        ));
    }
    Ok(value)
}

fn thread_params(conversation_id: &str) -> Value {
    json!({ "hostId": HOST_ID, "conversationId": conversation_id })
}

fn next_ipc_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = IPC_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("codex-xray-ipc-{millis}-{sequence}")
}

fn is_no_owner_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("no client")
        || error.contains("no handler")
        || error.contains("not handled")
        || error.contains("timed out")
        || error.contains("超时")
}

fn find_turn_id(value: &Value) -> Option<String> {
    [
        "/turnId",
        "/turn/id",
        "/result/turnId",
        "/result/turn/id",
        "/result/result/turn/id",
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
    .filter(|value| !value.is_empty())
    .map(ToOwned::to_owned)
}

fn thread_view(state: &Value, expected_turn_id: Option<&str>) -> DesktopThreadView {
    let turns = ordered_turns(state);
    let active = turns.iter().rev().copied().find(|turn| {
        matches!(
            turn.get("status").and_then(Value::as_str),
            Some("inProgress" | "running" | "waitingApproval" | "waitingInput")
        )
    });
    let active_turn_id = active.and_then(turn_id);
    let agent_preview = active
        .and_then(last_agent_text)
        .or_else(|| turns.last().and_then(|turn| last_agent_text(turn)));
    let pending_request = pending_request(state);
    let outcome = expected_turn_id.and_then(|expected| {
        turns
            .iter()
            .copied()
            .find(|turn| turn_id(turn).as_deref() == Some(expected))
            .and_then(turn_outcome)
    });
    DesktopThreadView {
        active_turn_id,
        agent_preview,
        pending_request,
        outcome,
    }
}

fn ordered_turns(state: &Value) -> Vec<&Value> {
    if let Some(turns) = state
        .get("turns")
        .and_then(Value::as_array)
        .filter(|turns| !turns.is_empty())
    {
        return turns.iter().collect();
    }
    let Some(entities) = state
        .pointer("/turnHistory/history/entitiesByKey")
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut turns: Vec<&Value> = Vec::new();
    if let Some(islands) = state
        .pointer("/turnHistory/history/islands")
        .and_then(Value::as_array)
    {
        for island in islands {
            let Some(entries) = island.get("entries").and_then(Value::as_array) else {
                continue;
            };
            for entry in entries {
                if let Some(entity) = resolve_history_entry(entry.get("value"), entities)
                    && turn_id(entity).is_some()
                    && !turns.iter().any(|turn| turn_id(turn) == turn_id(entity))
                {
                    turns.push(entity);
                }
            }
        }
    }
    if turns.is_empty() {
        turns.extend(entities.values().filter(|entity| turn_id(entity).is_some()));
    }
    turns
}

fn resolve_history_entry<'a>(
    value: Option<&'a Value>,
    entities: &'a Map<String, Value>,
) -> Option<&'a Value> {
    let value = value?;
    if turn_id(value).is_some() {
        return Some(value);
    }
    if let Some(key) = value.as_str() {
        return entities.get(key);
    }
    value
        .get("key")
        .or_else(|| value.get("entityKey"))
        .and_then(Value::as_str)
        .and_then(|key| entities.get(key))
}

fn turn_id(turn: &Value) -> Option<String> {
    turn.get("turnId")
        .or_else(|| turn.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn turn_items(turn: &Value) -> Option<&Vec<Value>> {
    turn.get("items").and_then(Value::as_array)
}

fn last_agent_text(turn: &Value) -> Option<String> {
    turn_items(turn)?
        .iter()
        .rev()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage"))
        .and_then(|item| item.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn turn_outcome(turn: &Value) -> Option<DesktopTurnOutcome> {
    let status = turn.get("status").and_then(Value::as_str)?;
    match status {
        "completed" => Some(DesktopTurnOutcome::Completed(
            last_agent_text(turn).unwrap_or_else(|| "Codex 回合已结束。".to_string()),
        )),
        "interrupted" | "aborted" | "cancelled" => Some(DesktopTurnOutcome::Aborted),
        "failed" => Some(DesktopTurnOutcome::Failed(
            turn.get("error")
                .and_then(|error| {
                    error
                        .as_str()
                        .map(ToOwned::to_owned)
                        .or_else(|| error.get("message")?.as_str().map(ToOwned::to_owned))
                })
                .unwrap_or_else(|| "Codex 回合失败".to_string()),
        )),
        _ => None,
    }
}

fn pending_request(state: &Value) -> Option<DesktopPendingRequest> {
    let requests = state.get("requests")?;
    let (fallback_id, request) = match requests {
        Value::Object(requests) => requests
            .iter()
            .next_back()
            .map(|(id, value)| (Some(id), value))?,
        Value::Array(requests) => (None, requests.last()?),
        _ => return None,
    };
    let method = request
        .get("method")
        .or_else(|| request.get("type"))
        .and_then(Value::as_str)?
        .to_string();
    if !method.to_ascii_lowercase().contains("approval")
        && !method.to_ascii_lowercase().contains("permission")
    {
        return None;
    }
    let request_id = request
        .get("requestId")
        .or_else(|| request.get("id"))
        .cloned()
        .or_else(|| fallback_id.map(|id| Value::String(id.to_string())))?;
    Some(DesktopPendingRequest {
        request_id,
        method,
        params: request
            .get("params")
            .cloned()
            .unwrap_or_else(|| request.clone()),
    })
}

fn apply_patches(target: &mut Value, patches: &[Value]) -> Result<(), String> {
    for patch in patches {
        let operation = patch
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex Desktop patch 缺少 op".to_string())?;
        let path = patch
            .get("path")
            .and_then(Value::as_array)
            .ok_or_else(|| "Codex Desktop patch 缺少 path".to_string())?;
        let value = patch.get("value").cloned();
        apply_patch(target, path, operation, value)?;
    }
    Ok(())
}

fn is_incremental_state_change(kind: &str) -> bool {
    matches!(kind, "patch" | "patches")
}

fn apply_patch(
    target: &mut Value,
    path: &[Value],
    operation: &str,
    value: Option<Value>,
) -> Result<(), String> {
    if path.is_empty() {
        return match operation {
            "add" | "replace" => {
                *target = value.ok_or_else(|| "Codex Desktop patch 缺少 value".to_string())?;
                Ok(())
            }
            "remove" => {
                *target = Value::Null;
                Ok(())
            }
            _ => Err(format!("不支持的 Codex Desktop patch 操作：{operation}")),
        };
    }
    let (parent_path, leaf) = path.split_at(path.len() - 1);
    let mut parent = target;
    for component in parent_path {
        parent = child_mut(parent, component)?;
    }
    match parent {
        Value::Object(object) => {
            let key = path_key(&leaf[0])?;
            match operation {
                "add" | "replace" => {
                    object.insert(
                        key,
                        value.ok_or_else(|| "Codex Desktop patch 缺少 value".to_string())?,
                    );
                }
                "remove" => {
                    object.remove(&key);
                }
                _ => return Err(format!("不支持的 Codex Desktop patch 操作：{operation}")),
            }
        }
        Value::Array(array) => {
            let index = path_index(&leaf[0], array.len(), operation == "add")?;
            match operation {
                "add" => array.insert(
                    index,
                    value.ok_or_else(|| "Codex Desktop patch 缺少 value".to_string())?,
                ),
                "replace" => {
                    let slot = array
                        .get_mut(index)
                        .ok_or_else(|| format!("Codex Desktop patch 数组下标越界：{index}"))?;
                    *slot = value.ok_or_else(|| "Codex Desktop patch 缺少 value".to_string())?;
                }
                "remove" => {
                    if index >= array.len() {
                        return Err(format!("Codex Desktop patch 数组下标越界：{index}"));
                    }
                    array.remove(index);
                }
                _ => return Err(format!("不支持的 Codex Desktop patch 操作：{operation}")),
            }
        }
        _ => return Err("Codex Desktop patch 的父节点不是对象或数组".to_string()),
    }
    Ok(())
}

fn child_mut<'a>(target: &'a mut Value, component: &Value) -> Result<&'a mut Value, String> {
    match target {
        Value::Object(object) => object
            .get_mut(&path_key(component)?)
            .ok_or_else(|| "Codex Desktop patch 对象路径不存在".to_string()),
        Value::Array(array) => {
            let index = path_index(component, array.len(), false)?;
            array
                .get_mut(index)
                .ok_or_else(|| format!("Codex Desktop patch 数组下标越界：{index}"))
        }
        _ => Err("Codex Desktop patch 路径经过非容器节点".to_string()),
    }
}

fn path_key(value: &Value) -> Result<String, String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| value.as_u64().map(|value| value.to_string()))
        .ok_or_else(|| "Codex Desktop patch 对象键无效".to_string())
}

fn path_index(value: &Value, length: usize, allow_end: bool) -> Result<usize, String> {
    if value.as_str() == Some("-") && allow_end {
        return Ok(length);
    }
    let index = value
        .as_u64()
        .map(|value| value as usize)
        .or_else(|| value.as_str()?.parse::<usize>().ok())
        .ok_or_else(|| "Codex Desktop patch 数组下标无效".to_string())?;
    if index > length || (!allow_end && index == length) {
        return Err(format!("Codex Desktop patch 数组下标越界：{index}"));
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_desktop_patch_state_names_from_old_and_new_clients() {
        assert!(is_incremental_state_change("patch"));
        assert!(is_incremental_state_change("patches"));
        assert!(!is_incremental_state_change("snapshot"));
    }

    #[test]
    fn applies_object_and_array_patches() {
        let mut value = json!({ "items": ["a", "c"], "status": "running" });
        apply_patches(
            &mut value,
            &[
                json!({ "op": "add", "path": ["items", 1], "value": "b" }),
                json!({ "op": "replace", "path": ["status"], "value": "completed" }),
                json!({ "op": "remove", "path": ["items", 0] }),
            ],
        )
        .unwrap();
        assert_eq!(value, json!({ "items": ["b", "c"], "status": "completed" }));
    }

    #[test]
    fn derives_active_turn_preview_and_completion() {
        let state = json!({
            "turns": [{
                "turnId": "turn-1",
                "status": "inProgress",
                "items": [{ "type": "agentMessage", "phase": "commentary", "text": "正在检查" }]
            }],
            "requests": {}
        });
        let view = thread_view(&state, Some("turn-1"));
        assert_eq!(view.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(view.agent_preview.as_deref(), Some("正在检查"));
        assert_eq!(view.outcome, None);

        let completed = json!({
            "turns": [{
                "turnId": "turn-1",
                "status": "completed",
                "items": [{ "type": "agentMessage", "phase": "final_answer", "text": "处理完成" }]
            }]
        });
        assert_eq!(
            thread_view(&completed, Some("turn-1")).outcome,
            Some(DesktopTurnOutcome::Completed("处理完成".to_string()))
        );
    }

    #[test]
    fn derives_pending_desktop_approval() {
        let state = json!({
            "requests": {
                "request-1": {
                    "method": "item/commandExecution/requestApproval",
                    "params": { "command": "cargo test" }
                }
            }
        });
        let request = pending_request(&state).unwrap();
        assert_eq!(request.request_id, json!("request-1"));
        assert!(request.method.contains("commandExecution"));
        assert_eq!(request.params["command"], "cargo test");
    }

    #[test]
    #[ignore = "requires a running Codex Desktop task and CODEX_XRAY_TEST_THREAD_ID"]
    fn follows_a_live_desktop_task_without_mutating_it() {
        let conversation_id = std::env::var("CODEX_XRAY_TEST_THREAD_ID")
            .expect("set CODEX_XRAY_TEST_THREAD_ID to a live Desktop task");
        let mut client = DesktopIpcClient::connect().unwrap();
        let owner = client
            .discover_owner(&conversation_id)
            .unwrap()
            .expect("Desktop did not claim the task");
        client.follow(&conversation_id, &owner).unwrap();
        client
            .current_view(&conversation_id, None)
            .expect("Desktop snapshot was not mirrored");
        client.unfollow(&conversation_id).unwrap();
    }

    #[test]
    #[ignore = "requires a running idle Codex Desktop task and CODEX_XRAY_TEST_THREAD_ID"]
    fn completes_a_live_desktop_turn_through_incremental_state_updates() {
        let conversation_id = std::env::var("CODEX_XRAY_TEST_THREAD_ID")
            .expect("set CODEX_XRAY_TEST_THREAD_ID to a live idle Desktop task");
        let mut client = DesktopIpcClient::connect().unwrap();
        let owner = client
            .discover_owner(&conversation_id)
            .unwrap()
            .expect("Desktop did not claim the task");
        let initial = client.follow(&conversation_id, &owner).unwrap();
        assert!(
            initial.active_turn_id.is_none(),
            "refusing to mutate a Desktop task with an active turn"
        );

        let expected = "IPC patches 兼容验证通过";
        let prompt = std::env::var("CODEX_XRAY_TEST_PROMPT")
            .unwrap_or_else(|_| format!("只回复这句话，不要添加其他内容：{expected}"));
        let message_id = format!("xray-live-smoke-{}", next_ipc_id());
        let turn_id = client
            .start_turn(&conversation_id, &owner, &prompt, &message_id)
            .unwrap();
        let deadline = SystemTime::now() + Duration::from_secs(120);

        loop {
            let remaining = deadline
                .duration_since(SystemTime::now())
                .unwrap_or_default();
            assert!(
                !remaining.is_zero(),
                "Desktop turn did not complete in 120s"
            );
            let Some(view) = client
                .receive_update(
                    &conversation_id,
                    Some(&turn_id),
                    remaining.min(Duration::from_secs(5)),
                )
                .unwrap()
            else {
                continue;
            };
            assert!(
                view.pending_request.is_none(),
                "unexpected approval request during fixed-response smoke test"
            );
            match view.outcome {
                Some(DesktopTurnOutcome::Completed(answer)) => {
                    assert!(
                        !answer.trim().is_empty(),
                        "Desktop returned an empty answer"
                    );
                    if std::env::var_os("CODEX_XRAY_TEST_PROMPT").is_none() {
                        assert!(
                            answer.contains(expected),
                            "Desktop returned an unexpected answer: {answer}"
                        );
                    }
                    println!("Desktop answer: {answer}");
                    break;
                }
                Some(DesktopTurnOutcome::Aborted) => panic!("Desktop turn was aborted"),
                Some(DesktopTurnOutcome::Failed(error)) => {
                    panic!("Desktop turn failed: {error}")
                }
                None => {}
            }
        }

        client.unfollow(&conversation_id).unwrap();
    }
}
