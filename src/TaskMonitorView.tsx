import {
  CircleAlert,
  CircleCheck,
  CircleDashed,
  Clock3,
  RefreshCw,
  Search,
} from "lucide-react";
import { useMemo, useState } from "react";
import {
  formatDuration,
  formatReadableTokens,
  formatSyncTime,
} from "./format";
import type { Locale } from "./i18n";
import type { TraceSessionSummary, TraceSnapshot } from "./types";

type TaskFilter = "active" | "attention" | "completed" | "all";

function copy(locale: Locale, zh: string, en: string): string {
  return locale === "zh-CN" ? zh : en;
}

function taskTitle(session: TraceSessionSummary): string {
  return session.conversation_name?.trim() || session.id;
}

function statusLabel(session: TraceSessionSummary, locale: Locale): string {
  if (session.status === "running") return copy(locale, "运行中", "Running");
  if (session.status === "waiting_approval") {
    return copy(locale, "等待审批", "Waiting for approval");
  }
  if (session.status === "waiting_input") {
    return copy(locale, "等待输入", "Waiting for input");
  }
  if (session.status === "completed") return copy(locale, "已完成", "Completed");
  if (session.status === "failed") return copy(locale, "失败", "Failed");
  if (session.status === "interrupted") {
    return copy(locale, "已中断", "Interrupted");
  }
  return copy(locale, "状态未知", "Unknown");
}

function statusSourceLabel(
  source: TraceSessionSummary["status_source"],
  locale: Locale,
): string {
  if (source === "app_server") {
    return copy(locale, "App Server 状态", "App Server status");
  }
  if (source === "local_events") {
    return copy(locale, "本地事件推断", "Inferred from local events");
  }
  return copy(locale, "未提供状态", "Status unavailable");
}

function isActive(status: TraceSessionSummary["status"]): boolean {
  return ["running", "waiting_approval", "waiting_input"].includes(status);
}

function isAttention(status: TraceSessionSummary["status"]): boolean {
  return ["failed", "interrupted"].includes(status);
}

function updatedTime(session: TraceSessionSummary): number {
  const parsed = Date.parse(session.updated_at ?? session.started_at ?? "");
  return Number.isFinite(parsed) ? parsed : 0;
}

export default function TaskMonitorView({
  locale,
  snapshot,
  loading,
  usingCache,
  error,
  onRefresh,
  onOpenSession,
}: {
  locale: Locale;
  snapshot: TraceSnapshot | null;
  loading: boolean;
  usingCache: boolean;
  error: string | null;
  onRefresh: () => void;
  onOpenSession: (sessionId: string) => void;
}) {
  const [filter, setFilter] = useState<TaskFilter>("active");
  const [query, setQuery] = useState("");
  const sessions = snapshot?.sessions ?? [];
  const counts = useMemo(
    () => ({
      active: sessions.filter((session) => isActive(session.status)).length,
      attention: sessions.filter((session) => isAttention(session.status))
        .length,
      completed: sessions.filter((session) => session.status === "completed")
        .length,
      all: sessions.length,
    }),
    [sessions],
  );
  const visibleSessions = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    return sessions
      .filter((session) => {
        if (filter === "active") return isActive(session.status);
        if (filter === "attention") return isAttention(session.status);
        if (filter === "completed") return session.status === "completed";
        return true;
      })
      .filter((session) => {
        if (!normalizedQuery) return true;
        return [
          taskTitle(session),
          session.project,
          session.project_path,
          session.model,
        ].some((value) => value.toLocaleLowerCase().includes(normalizedQuery));
      })
      .sort((left, right) => updatedTime(right) - updatedTime(left));
  }, [filter, query, sessions]);

  return (
    <main className="workspace task-workspace">
      <header className="topbar task-topbar">
        <div>
          <h1>{copy(locale, "任务", "Tasks")}</h1>
          <span className="header-note">
            {copy(
              locale,
              "查看当前状态与最近任务，点击进入执行追踪",
              "Inspect current and recent tasks, then open their execution trace",
            )}
          </span>
        </div>
        <div className="topbar-actions">
          <span>
            {snapshot
              ? `${usingCache ? copy(locale, "缓存", "Cached") : copy(locale, "更新", "Updated")} ${formatSyncTime(snapshot.generated_at)}`
              : copy(locale, "尚未读取", "Not loaded")}
          </span>
          <button
            className={loading ? "refresh-button spinning" : "refresh-button"}
            onClick={onRefresh}
            disabled={loading}
            aria-label={copy(locale, "刷新任务状态", "Refresh task status")}
          >
            <RefreshCw aria-hidden="true" />
          </button>
        </div>
      </header>

      <section className="task-summary" aria-label={copy(locale, "任务概览", "Task overview")}>
        <div>
          <CircleDashed aria-hidden="true" />
          <span>{copy(locale, "活动", "Active")}</span>
          <strong>{counts.active}</strong>
        </div>
        <div>
          <CircleAlert aria-hidden="true" />
          <span>{copy(locale, "需处理", "Attention")}</span>
          <strong>{counts.attention}</strong>
        </div>
        <div>
          <CircleCheck aria-hidden="true" />
          <span>{copy(locale, "已完成", "Completed")}</span>
          <strong>{counts.completed}</strong>
        </div>
      </section>

      <section className="task-table-section">
        <div className="task-toolbar">
          <div className="task-filter" role="tablist" aria-label={copy(locale, "任务筛选", "Task filter")}>
            {(
              [
                ["active", copy(locale, "活动", "Active")],
                ["attention", copy(locale, "需处理", "Attention")],
                ["completed", copy(locale, "已完成", "Completed")],
                ["all", copy(locale, "全部", "All")],
              ] as const
            ).map(([value, label]) => (
              <button
                key={value}
                role="tab"
                aria-selected={filter === value}
                className={filter === value ? "selected" : ""}
                onClick={() => setFilter(value)}
              >
                {label}
                <span>{counts[value]}</span>
              </button>
            ))}
          </div>
          <label className="task-search">
            <Search aria-hidden="true" />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={copy(locale, "搜索任务或项目", "Search tasks or projects")}
            />
          </label>
        </div>

        <p className="task-source-note">
          {copy(
            locale,
            "App Server 能返回的状态直接展示；跨进程不可见时，运行、完成与中断根据本地 Session 事件和最近写入时间推断。",
            "App Server states are shown directly. When cross-process activity is invisible, running, completed, and interrupted states are inferred from local session events and recent writes.",
          )}
        </p>

        {error && (
          <div className="task-error" role="alert">
            <CircleAlert aria-hidden="true" />
            <span>{error}</span>
            <button onClick={onRefresh}>{copy(locale, "重试", "Retry")}</button>
          </div>
        )}

        {!error && loading && !snapshot && (
          <div className="task-empty">
            <CircleDashed className="spinning" aria-hidden="true" />
            <strong>{copy(locale, "正在读取任务目录", "Loading task catalog")}</strong>
          </div>
        )}

        {!loading && !error && visibleSessions.length === 0 && (
          <div className="task-empty">
            <CircleCheck aria-hidden="true" />
            <strong>
              {filter === "active"
                ? copy(locale, "当前没有活动任务", "No active tasks")
                : copy(locale, "没有符合条件的任务", "No matching tasks")}
            </strong>
          </div>
        )}

        {visibleSessions.length > 0 && (
          <div className="task-table" aria-label={copy(locale, "任务列表", "Task list")}>
            <div className="task-row task-table-head" aria-hidden="true">
              <span>{copy(locale, "任务", "Task")}</span>
              <span>{copy(locale, "状态", "Status")}</span>
              <span>{copy(locale, "项目", "Project")}</span>
              <span>{copy(locale, "用量", "Usage")}</span>
              <span>{copy(locale, "更新时间", "Updated")}</span>
            </div>
            {visibleSessions.map((session) => (
              <button
                key={session.id}
                className="task-row task-data-row"
                onClick={() => onOpenSession(session.id)}
                aria-label={`${taskTitle(session)} · ${statusLabel(session, locale)} · ${session.project}`}
              >
                <span className="task-name-cell">
                  <strong>{taskTitle(session)}</strong>
                  <small>{session.model === "—" ? session.id : session.model}</small>
                </span>
                <span className="task-status-cell">
                  <i className={`trace-status-dot ${session.status}`} />
                  <span>
                    <strong>{statusLabel(session, locale)}</strong>
                    <small>{statusSourceLabel(session.status_source, locale)}</small>
                  </span>
                </span>
                <span className="task-project-cell">
                  <strong>{session.project}</strong>
                  <small>{session.project_path}</small>
                </span>
                <span className="task-usage-cell">
                  <strong>{formatReadableTokens(session.total_tokens)}</strong>
                  <small>
                    {session.turns > 0
                      ? copy(locale, `${session.turns} 回合`, `${session.turns} turns`)
                      : copy(locale, "未分析", "Not analyzed")}
                  </small>
                </span>
                <span className="task-time-cell">
                  <Clock3 aria-hidden="true" />
                  <span>
                    <strong>
                      {session.updated_at
                        ? formatSyncTime(session.updated_at)
                        : "—"}
                    </strong>
                    <small>
                      {session.duration_ms != null
                        ? formatDuration(session.duration_ms / 1000)
                        : ""}
                    </small>
                  </span>
                </span>
              </button>
            ))}
          </div>
        )}
      </section>
    </main>
  );
}
