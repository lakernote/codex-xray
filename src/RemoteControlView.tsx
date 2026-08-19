import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUpDown,
  CheckCircle2,
  CircleAlert,
  FolderPlus,
  Link2,
  LoaderCircle,
  LogOut,
  MessageCircle,
  Power,
  RefreshCw,
  Search,
  ShieldCheck,
  Square,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { Locale } from "./i18n";
import type { RemoteChannelSnapshot, RemoteTaskSummary } from "./types";

function copy(locale: Locale, zh: string, en: string): string {
  return locale === "zh-CN" ? zh : en;
}

function statusLabel(locale: Locale, status: string): string {
  const labels: Record<string, [string, string]> = {
    login_required: ["等待扫码", "Login required"],
    connected: ["已连接", "Connected"],
    stopped: ["已停止", "Stopped"],
    degraded: ["连接异常", "Degraded"],
    idle: ["空闲", "Idle"],
    running: ["运行中", "Running"],
    waiting_approval: ["等待审批", "Awaiting approval"],
    waiting_input: ["等待输入", "Awaiting input"],
    failed: ["失败", "Failed"],
  };
  const label = labels[status];
  return label ? label[locale === "zh-CN" ? 0 : 1] : status;
}

function shortId(value: string): string {
  return value.slice(0, 8);
}

type TaskSortMode = "updated" | "title" | "cwd";

const MAX_RENDERED_TASKS = 100;

function taskDate(value: number): Date | null {
  if (!Number.isFinite(value) || value <= 0) return null;
  const milliseconds = value < 10_000_000_000 ? value * 1_000 : value;
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime()) ? null : date;
}

function formatTaskUpdatedAt(value: number, locale: Locale): string {
  const date = taskDate(value);
  if (!date) return copy(locale, "更新时间未知", "Update time unknown");

  const delta = date.getTime() - Date.now();
  const absoluteDelta = Math.abs(delta);
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  if (absoluteDelta < 60_000) {
    return formatter.format(Math.round(delta / 1_000), "second");
  }
  if (absoluteDelta < 3_600_000) {
    return formatter.format(Math.round(delta / 60_000), "minute");
  }
  if (absoluteDelta < 86_400_000) {
    return formatter.format(Math.round(delta / 3_600_000), "hour");
  }
  if (absoluteDelta < 7 * 86_400_000) {
    return formatter.format(Math.round(delta / 86_400_000), "day");
  }
  return new Intl.DateTimeFormat(locale, {
    year: date.getFullYear() === new Date().getFullYear() ? undefined : "numeric",
    month: "short",
    day: "numeric",
  }).format(date);
}

function formatTaskUpdatedAtFull(value: number, locale: Locale): string {
  const date = taskDate(value);
  if (!date) return copy(locale, "更新时间未知", "Update time unknown");
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function compareTasks(
  left: RemoteTaskSummary,
  right: RemoteTaskSummary,
  mode: TaskSortMode,
  locale: Locale,
): number {
  if (mode === "title") {
    return (
      left.title.localeCompare(right.title, locale, { numeric: true }) ||
      right.updated_at - left.updated_at ||
      left.id.localeCompare(right.id)
    );
  }
  if (mode === "cwd") {
    return (
      left.cwd.localeCompare(right.cwd, locale, { numeric: true }) ||
      left.title.localeCompare(right.title, locale, { numeric: true }) ||
      right.updated_at - left.updated_at ||
      left.id.localeCompare(right.id)
    );
  }
  return right.updated_at - left.updated_at || left.id.localeCompare(right.id);
}

function formatTime(value: string | null, locale: Locale): string {
  if (!value) return copy(locale, "暂无", "None");
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return new Intl.DateTimeFormat(locale, {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(parsed);
}

export default function RemoteControlView({ locale }: { locale: Locale }) {
  const [snapshot, setSnapshot] = useState<RemoteChannelSnapshot | null>(null);
  const [tasks, setTasks] = useState<RemoteTaskSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [verifyCode, setVerifyCode] = useState("");
  const [newTaskCwd, setNewTaskCwd] = useState("");
  const [taskQuery, setTaskQuery] = useState("");
  const [taskSortMode, setTaskSortMode] = useState<TaskSortMode>("updated");

  const loadSnapshot = useCallback(async (quiet = false) => {
    if (!quiet) setLoading(true);
    try {
      const next = await invoke<RemoteChannelSnapshot>(
        "get_remote_channel_snapshot",
      );
      setSnapshot(next);
      if (!quiet) setError(null);
    } catch (cause) {
      if (!quiet) setError(String(cause));
    } finally {
      if (!quiet) setLoading(false);
    }
  }, []);

  const loadTasks = useCallback(async () => {
    setBusy("tasks");
    try {
      setTasks(await invoke<RemoteTaskSummary[]>("get_remote_tasks"));
      setError(null);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  useEffect(() => {
    void loadSnapshot();
    void loadTasks();
  }, [loadSnapshot, loadTasks]);

  useEffect(() => {
    if (!snapshot?.enabled && !snapshot?.active_turn_id) return;
    const timer = window.setInterval(() => void loadSnapshot(true), 2_000);
    return () => window.clearInterval(timer);
  }, [loadSnapshot, snapshot?.active_turn_id, snapshot?.enabled]);

  useEffect(() => {
    if (
      !snapshot?.qr_svg ||
      snapshot.state === "connected" ||
      snapshot.verify_code_required
    ) {
      return;
    }
    let cancelled = false;
    let timer: number | undefined;
    const poll = async () => {
      if (cancelled) return;
      try {
        const next = await invoke<RemoteChannelSnapshot>("poll_weixin_login", {
          verifyCode: null,
        });
        if (cancelled) return;
        setSnapshot(next);
        setError(null);
        if (next.state !== "connected" && !next.verify_code_required) {
          timer = window.setTimeout(poll, 1_200);
        }
      } catch (cause) {
        if (!cancelled) {
          setError(String(cause));
          timer = window.setTimeout(poll, 2_000);
        }
      }
    };
    void poll();
    return () => {
      cancelled = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [snapshot?.qr_svg, snapshot?.state, snapshot?.verify_code_required]);

  const invokeSnapshotAction = async (
    name: string,
    command: string,
    args?: Record<string, unknown>,
  ) => {
    setBusy(name);
    setError(null);
    try {
      setSnapshot(await invoke<RemoteChannelSnapshot>(command, args));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  const submitVerifyCode = async () => {
    const code = verifyCode.trim();
    if (!code) return;
    setBusy("verify");
    setError(null);
    try {
      const next = await invoke<RemoteChannelSnapshot>("poll_weixin_login", {
        verifyCode: code,
      });
      setSnapshot(next);
      if (!next.verify_code_required) setVerifyCode("");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  const attachTask = async (threadId: string) => {
    setBusy(`attach-${threadId}`);
    setError(null);
    const target = tasks.find((task) => task.id === threadId);
    if (target) {
      setSnapshot((current) =>
        current
          ? {
              ...current,
              attached_thread_id: target.id,
              attached_thread_title: target.title,
              attached_cwd: target.cwd,
              control_ready: false,
              control_backend: "none",
              active_turn_id: null,
              latest_activity: copy(
                locale,
                "正在后台唤醒 Codex Desktop 原任务…",
                "Waking the original Codex Desktop task in the background…",
              ),
              agent_preview: null,
              pending_approval: null,
              last_error: null,
            }
          : current,
      );
    }
    try {
      setSnapshot(
        await invoke<RemoteChannelSnapshot>("attach_remote_task", { threadId }),
      );
      await loadTasks();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  const createTask = async () => {
    const cwd = newTaskCwd.trim();
    if (!cwd) return;
    setBusy("create-task");
    setError(null);
    try {
      setSnapshot(
        await invoke<RemoteChannelSnapshot>("create_remote_task", { cwd }),
      );
      setNewTaskCwd("");
      await loadTasks();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  const connected = Boolean(snapshot?.account_id);
  const activeTask = useMemo(
    () => tasks.find((task) => task.id === snapshot?.attached_thread_id),
    [snapshot?.attached_thread_id, tasks],
  );
  const projectDirectories = useMemo(
    () => Array.from(new Set(tasks.map((task) => task.cwd))),
    [tasks],
  );
  const matchingTasks = useMemo(() => {
    const query = taskQuery.trim().toLocaleLowerCase(locale);
    return tasks
      .filter((task) => {
        if (!query) return true;
        return [task.title, task.cwd, task.id].some((value) =>
          value.toLocaleLowerCase(locale).includes(query),
        );
      })
      .sort((left, right) => {
        const leftSelected = left.id === snapshot?.attached_thread_id;
        const rightSelected = right.id === snapshot?.attached_thread_id;
        if (leftSelected !== rightSelected) return leftSelected ? -1 : 1;
        return compareTasks(left, right, taskSortMode, locale);
      });
  }, [locale, snapshot?.attached_thread_id, taskQuery, taskSortMode, tasks]);
  const visibleTasks = useMemo(
    () => matchingTasks.slice(0, MAX_RENDERED_TASKS),
    [matchingTasks],
  );
  const targetOccupied = Boolean(
    snapshot?.attached_thread_id &&
      !snapshot.control_ready &&
      (snapshot.latest_activity?.includes("owner") ||
        snapshot.latest_activity?.includes("IPC")),
  );

  if (loading && !snapshot) {
    return (
      <main className="workspace remote-workspace remote-loading">
        <LoaderCircle className="standard-loader" aria-hidden="true" />
        <span>{copy(locale, "正在读取远程通道", "Loading remote channel")}</span>
      </main>
    );
  }

  return (
    <main className="workspace remote-workspace">
      <header className="topbar remote-topbar">
        <div>
          <h1>{copy(locale, "远程通道", "Remote channel")}</h1>
          <span className="header-note">
            {copy(
              locale,
              "从微信查看和控制 Codex；退出 X-Ray 后连接立即停止",
              "Monitor and control Codex from WeChat; the connection stops when X-Ray exits",
            )}
          </span>
        </div>
        <div className="remote-state-chip" data-state={snapshot?.state ?? "unknown"}>
          <i />
          {statusLabel(locale, snapshot?.state ?? "unknown")}
        </div>
      </header>

      {error && (
        <div className="remote-alert error" role="alert">
          <CircleAlert aria-hidden="true" />
          <span>{error}</span>
        </div>
      )}

      <section className="remote-overview">
        <article className="remote-channel-card">
          <header>
            <span className="remote-card-icon weixin">
              <MessageCircle aria-hidden="true" />
            </span>
            <div>
              <p>{copy(locale, "通道", "Channel")}</p>
              <h2>{copy(locale, "微信", "WeChat")}</h2>
            </div>
            <span className={`remote-badge ${snapshot?.enabled ? "online" : ""}`}>
              {snapshot?.enabled
                ? copy(locale, "正在监听", "Listening")
                : connected
                  ? copy(locale, "已暂停", "Paused")
                  : copy(locale, "未绑定", "Not paired")}
            </span>
          </header>

          {connected ? (
            <div className="remote-account">
              <dl>
                <div>
                  <dt>{copy(locale, "微信 Bot", "WeChat bot")}</dt>
                  <dd>{snapshot?.account_id}</dd>
                </div>
                <div>
                  <dt>{copy(locale, "最近收到", "Last inbound")}</dt>
                  <dd>{formatTime(snapshot?.last_inbound_at ?? null, locale)}</dd>
                </div>
                <div>
                  <dt>{copy(locale, "最近回复", "Last outbound")}</dt>
                  <dd>{formatTime(snapshot?.last_outbound_at ?? null, locale)}</dd>
                </div>
              </dl>
              <div className="remote-actions">
                <button
                  className={snapshot?.enabled ? "secondary-action" : "primary-action"}
                  disabled={Boolean(busy)}
                  onClick={() =>
                    void invokeSnapshotAction("toggle", "set_remote_channel_enabled", {
                      enabled: !snapshot?.enabled,
                    })
                  }
                >
                  <Power aria-hidden="true" />
                  {snapshot?.enabled
                    ? copy(locale, "暂停通道", "Pause channel")
                    : copy(locale, "启用通道", "Enable channel")}
                </button>
                <button
                  className="text-action danger"
                  disabled={Boolean(busy)}
                  onClick={() =>
                    void invokeSnapshotAction(
                      "disconnect",
                      "disconnect_weixin_channel",
                    )
                  }
                >
                  <LogOut aria-hidden="true" />
                  {copy(locale, "解除绑定", "Disconnect")}
                </button>
              </div>
            </div>
          ) : (
            <div className="remote-login">
              {snapshot?.qr_svg ? (
                <div className="remote-qr-stage">
                  <img
                    className="remote-qr"
                    src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(snapshot.qr_svg)}`}
                    alt={copy(locale, "微信登录二维码", "WeChat login QR code")}
                  />
                  <div>
                    <strong>{copy(locale, "使用手机微信扫码", "Scan with WeChat")}</strong>
                    <span>
                      {copy(
                        locale,
                        "扫码并在手机上确认后，X-Ray 会自动完成连接。",
                        "Confirm on your phone and X-Ray will connect automatically.",
                      )}
                    </span>
                    {snapshot.verify_code_required && (
                      <div className="remote-verify-code">
                        <label htmlFor="weixin-verify-code">
                          {copy(locale, "手机显示的配对数字", "Pairing code shown on phone")}
                        </label>
                        <div>
                          <input
                            id="weixin-verify-code"
                            inputMode="numeric"
                            value={verifyCode}
                            onChange={(event) => setVerifyCode(event.target.value)}
                            onKeyDown={(event) => {
                              if (event.key === "Enter") void submitVerifyCode();
                            }}
                          />
                          <button
                            className="primary-action"
                            disabled={!verifyCode.trim() || busy === "verify"}
                            onClick={() => void submitVerifyCode()}
                          >
                            {copy(locale, "确认", "Confirm")}
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              ) : (
                <>
                  <p>
                    {copy(
                      locale,
                      "扫码绑定你的微信。只接受绑定账号发来的私聊文字和可用的语音转写。",
                      "Pair your WeChat account. Only direct text and available voice transcripts from the paired owner are accepted.",
                    )}
                  </p>
                  <button
                    className="primary-action"
                    disabled={Boolean(busy)}
                    onClick={() =>
                      void invokeSnapshotAction("login", "start_weixin_login")
                    }
                  >
                    {busy === "login" ? (
                      <LoaderCircle className="standard-loader" aria-hidden="true" />
                    ) : (
                      <Link2 aria-hidden="true" />
                    )}
                    {copy(locale, "扫码连接微信", "Pair WeChat")}
                  </button>
                </>
              )}
            </div>
          )}
        </article>

        <article className="remote-task-card">
          <header>
            <div>
              <p>{copy(locale, "当前控制目标", "Control target")}</p>
              <h2>
                {snapshot?.attached_thread_title ??
                  copy(locale, "尚未选择目标", "No target selected")}
              </h2>
            </div>
            {snapshot?.attached_thread_id && (
              <span className={`remote-badge ${snapshot.control_ready ? "online" : "observe"}`}>
                {snapshot.control_ready
                  ? snapshot.control_backend === "desktop_ipc"
                    ? copy(locale, "Desktop 原任务可控", "Desktop task controllable")
                    : copy(locale, "X-Ray 独立任务", "X-Ray-owned task")
                  : targetOccupied
                    ? copy(locale, "IPC 未连接", "IPC not connected")
                    : snapshot.last_error
                      ? copy(locale, "暂不可用", "Unavailable")
                      : copy(locale, "正在检测", "Checking")}
              </span>
            )}
          </header>
          {snapshot?.attached_thread_id ? (
            <div className="remote-task-current">
              <code>{snapshot.attached_cwd}</code>
              <dl>
                <div>
                  <dt>{copy(locale, "任务 ID", "Task ID")}</dt>
                  <dd>{shortId(snapshot.attached_thread_id)}</dd>
                </div>
                <div>
                  <dt>{copy(locale, "状态", "Status")}</dt>
                  <dd>
                    {snapshot.active_turn_id
                      ? copy(locale, "正在执行", "Running")
                      : statusLabel(locale, activeTask?.status ?? "idle")}
                  </dd>
                </div>
              </dl>
              <div className="remote-activity">
                <span>{snapshot.latest_activity ?? copy(locale, "等待微信指令", "Waiting for WeChat")}</span>
                {snapshot.agent_preview && <p>{snapshot.agent_preview}</p>}
              </div>
              {!snapshot.control_ready && (
                <div className="remote-control-limit" role="status">
                  <CircleAlert aria-hidden="true" />
                  <div>
                    <strong>
                      {targetOccupied
                        ? copy(
                            locale,
                            "尚未连接 Codex Desktop 原任务",
                            "Not connected to the original Desktop task",
                          )
                        : copy(
                            locale,
                            "X-Ray 暂时无法控制这个任务",
                            "X-Ray cannot control this task yet",
                          )}
                    </strong>
                    <span>
                      {snapshot.last_error ??
                        (targetOccupied
                          ? copy(
                              locale,
                              "X-Ray 会先自动唤醒并连接原任务；只有自动连接仍失败时，才需要手动打开完全相同的 Codex 任务。不会创建副本或新会话。",
                              "X-Ray first wakes and connects the original task automatically. Open the exact Codex task manually only if automatic connection still fails. No copy or new task is created.",
                            )
                          : copy(
                              locale,
                              "正在确认任务是否可以控制，请稍后重新检测。",
                              "Checking whether this task can be controlled. Try again shortly.",
                            ))}
                    </span>
                  </div>
                  <button
                    className="secondary-action"
                    disabled={Boolean(busy)}
                    onClick={() => void attachTask(snapshot.attached_thread_id!)}
                  >
                    {busy === `attach-${snapshot.attached_thread_id}` ? (
                      <LoaderCircle className="standard-loader" aria-hidden="true" />
                    ) : (
                      <RefreshCw aria-hidden="true" />
                    )}
                    {busy === `attach-${snapshot.attached_thread_id}`
                      ? copy(locale, "正在自动连接", "Connecting automatically")
                      : copy(locale, "重试自动连接", "Retry automatic connection")}
                  </button>
                </div>
              )}
              {snapshot.pending_approval && (
                <div className="remote-approval">
                  <ShieldCheck aria-hidden="true" />
                  <div>
                    <strong>
                      {copy(locale, "等待微信审批", "Awaiting WeChat approval")} · {snapshot.pending_approval.kind}
                    </strong>
                    <span>{snapshot.pending_approval.summary}</span>
                  </div>
                </div>
              )}
              {snapshot.active_turn_id && (
                <button
                  className="secondary-action remote-stop"
                  disabled={busy === "stop"}
                  onClick={() =>
                    void invokeSnapshotAction("stop", "interrupt_remote_task")
                  }
                >
                  <Square aria-hidden="true" />
                  {copy(locale, "停止当前回合", "Stop current turn")}
                </button>
              )}
            </div>
          ) : (
            <div className="remote-task-empty">
              <CheckCircle2 aria-hidden="true" />
              <span>
                {copy(
                  locale,
                  "请从下方选择一个现有任务，或明确新建任务。选好后微信直接发送普通文字。",
                  "Choose an existing task below or explicitly create one. After that, send plain text in WeChat.",
                )}
              </span>
            </div>
          )}
        </article>
      </section>

      {connected && (
        <section className="remote-quick-guide">
          <div>
            <p>{copy(locale, "现在这样用", "How to use it")}</p>
            <strong>
              {copy(
                locale,
                "X-Ray 选目标，微信直接聊天",
                "Pick a target in X-Ray, then chat directly in WeChat",
              )}
            </strong>
          </div>
          <ol>
            <li>
              {copy(
                locale,
                "点击“控制”会自动唤醒并连接 Codex 原任务；也可以明确“新建并控制”",
                "Control automatically wakes and connects the original Codex task; you can also explicitly Create and control",
              )}
            </li>
            <li>
              {copy(
                locale,
                "微信直接发普通文字；长任务会自动同步处理中状态",
                "Send plain text in WeChat; long tasks proactively report progress",
              )}
            </li>
            <li>
              {copy(
                locale,
                "外出时才用 /list 临时切换，/status 查看进度",
                "Use /list only to switch remotely, and /status to check progress",
              )}
            </li>
          </ol>
          <span>
            {copy(
              locale,
              "Desktop 任务通过本机 IPC 控制同一会话；明确新建的任务才由 X-Ray 独立运行。",
              "Desktop tasks use local IPC to control the same conversation; only explicitly created tasks run independently in X-Ray.",
            )}
          </span>
        </section>
      )}

      <section className="remote-task-list-section">
        <header>
          <div>
            <h2>{copy(locale, "选择微信控制目标", "Choose the WeChat control target")}</h2>
            <span>
              {copy(
                locale,
                "选择后微信普通文字会直接进入该任务；不会自动复制或新建会话",
                "After selection, plain WeChat messages go directly to this task; X-Ray never copies or creates a task automatically",
              )}
            </span>
          </div>
          <button
            className={`refresh-button${busy === "tasks" ? " spinning" : ""}`}
            aria-label={copy(locale, "刷新任务", "Refresh tasks")}
            disabled={busy === "tasks"}
            onClick={() => void loadTasks()}
          >
            <RefreshCw aria-hidden="true" />
          </button>
        </header>
        <div className="remote-create-task">
          <div>
            <FolderPlus aria-hidden="true" />
            <span>
              <strong>{copy(locale, "明确新建任务", "Explicitly create a task")}</strong>
              <small>
                {copy(
                  locale,
                  "输入一个曾在 Codex 中打开过的项目目录",
                  "Enter a project directory previously opened in Codex",
                )}
              </small>
            </span>
          </div>
          <input
            aria-label={copy(locale, "新任务项目目录", "New task project directory")}
            list="remote-project-directories"
            placeholder="/Users/name/project"
            value={newTaskCwd}
            onChange={(event) => setNewTaskCwd(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void createTask();
            }}
          />
          <datalist id="remote-project-directories">
            {projectDirectories.map((cwd) => (
              <option key={cwd} value={cwd} />
            ))}
          </datalist>
          <button
            className="primary-action"
            disabled={!newTaskCwd.trim() || Boolean(busy)}
            onClick={() => void createTask()}
          >
            {busy === "create-task" ? (
              <LoaderCircle className="standard-loader" aria-hidden="true" />
            ) : (
              <FolderPlus aria-hidden="true" />
            )}
            {copy(locale, "新建并控制", "Create and control")}
          </button>
        </div>
        <div className="remote-task-toolbar" role="search">
          <label className="remote-task-search">
            <span>{copy(locale, "搜索对话", "Search tasks")}</span>
            <div>
              <Search aria-hidden="true" />
              <input
                type="search"
                value={taskQuery}
                placeholder={copy(
                  locale,
                  "输入标题、项目目录或任务 ID",
                  "Search title, project directory, or task ID",
                )}
                onChange={(event) => setTaskQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") setTaskQuery("");
                }}
              />
              {taskQuery && (
                <button
                  type="button"
                  aria-label={copy(locale, "清空搜索", "Clear search")}
                  onClick={() => setTaskQuery("")}
                >
                  <X aria-hidden="true" />
                </button>
              )}
            </div>
          </label>
          <label className="remote-task-sort">
            <span>{copy(locale, "排序", "Sort")}</span>
            <div>
              <ArrowUpDown aria-hidden="true" />
              <select
                value={taskSortMode}
                onChange={(event) =>
                  setTaskSortMode(event.target.value as TaskSortMode)
                }
              >
                <option value="updated">
                  {copy(locale, "最近更新", "Recently updated")}
                </option>
                <option value="title">
                  {copy(locale, "任务名称", "Task name")}
                </option>
                <option value="cwd">
                  {copy(locale, "项目目录", "Project directory")}
                </option>
              </select>
            </div>
          </label>
          <span className="remote-task-result-count" aria-live="polite">
            {taskQuery.trim()
              ? copy(
                  locale,
                  `找到 ${matchingTasks.length} 个，共 ${tasks.length} 个`,
                  `${matchingTasks.length} of ${tasks.length} tasks`,
                )
              : copy(locale, `共 ${tasks.length} 个对话`, `${tasks.length} tasks`)}
            {matchingTasks.length > MAX_RENDERED_TASKS &&
              copy(
                locale,
                ` · 当前显示前 ${MAX_RENDERED_TASKS} 个`,
                ` · showing the first ${MAX_RENDERED_TASKS}`,
              )}
          </span>
        </div>
        <div className="remote-task-list">
          {tasks.length === 0 ? (
            <div className="remote-task-list-empty">
              {copy(locale, "没有找到 Codex 任务", "No Codex tasks found")}
            </div>
          ) : matchingTasks.length === 0 ? (
            <div className="remote-task-list-empty filtered">
              <Search aria-hidden="true" />
              <strong>{copy(locale, "没有匹配的对话", "No matching tasks")}</strong>
              <span>
                {copy(
                  locale,
                  "换个标题、目录或任务 ID 试试",
                  "Try another title, directory, or task ID",
                )}
              </span>
              <button type="button" className="text-action" onClick={() => setTaskQuery("")}>
                {copy(locale, "清空搜索", "Clear search")}
              </button>
            </div>
          ) : (
            visibleTasks.map((task) => {
              const selected = task.id === snapshot?.attached_thread_id;
              const selectedAndReady = selected && snapshot?.control_ready;
              const updatedDate = taskDate(task.updated_at);
              return (
                <article className={selected ? "selected" : ""} key={task.id}>
                  <div className="remote-task-main">
                    <span className={`remote-task-status ${task.status}`} />
                    <div>
                      <strong>{task.title}</strong>
                      <code>{task.cwd}</code>
                    </div>
                  </div>
                  <div className="remote-task-meta">
                    <span>{statusLabel(locale, task.status)}</span>
                    <time
                      dateTime={updatedDate?.toISOString()}
                      title={formatTaskUpdatedAtFull(task.updated_at, locale)}
                    >
                      {formatTaskUpdatedAt(task.updated_at, locale)}
                    </time>
                    <small>{shortId(task.id)}</small>
                  </div>
                  <button
                    className="secondary-action"
                    disabled={Boolean(busy) || selectedAndReady}
                    onClick={() => void attachTask(task.id)}
                  >
                    {busy === `attach-${task.id}` ? (
                      <LoaderCircle className="standard-loader" aria-hidden="true" />
                    ) : (
                      <Link2 aria-hidden="true" />
                    )}
                    {selectedAndReady
                      ? copy(locale, "正在控制", "Controlling")
                      : busy === `attach-${task.id}`
                        ? copy(locale, "正在自动连接", "Connecting automatically")
                        : selected
                          ? copy(locale, "重试自动连接", "Retry automatic connection")
                          : copy(locale, "控制", "Control")}
                  </button>
                </article>
              );
            })
          )}
        </div>
      </section>

      <footer className="remote-safety-note">
        <ShieldCheck aria-hidden="true" />
        <span>
          {copy(
            locale,
            "Desktop 原任务只通过本机 IPC 转发；独立任务使用 workspace-write 与按需审批。不会开放公网端口，也不会在退出后留后台进程。",
            "Desktop tasks are forwarded only over local IPC; independent tasks use workspace-write with on-request approvals. No public port or background process remains after exit.",
          )}
        </span>
      </footer>
    </main>
  );
}
