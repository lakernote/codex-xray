import { invoke } from "@tauri-apps/api/core";
import {
  ChevronRight,
  CircleHelp,
  Folder,
  FolderOpen,
  LoaderCircle,
  MessageSquareText,
  PanelLeftClose,
  PanelLeftOpen,
  RefreshCw,
  RotateCcw,
  ScanSearch,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  formatDuration,
  formatExactTokens,
  formatExactUsd,
  formatReadableTokens,
  formatSyncTime,
} from "./format";
import type { Locale, Translator } from "./i18n";
import SearchField from "./SearchField";
import type {
  TraceSessionDetail,
  TraceSessionSummary,
  TraceSnapshot,
  TraceTimelineEvent,
  TraceTurnSummary,
} from "./types";

type TraceViewProps = {
  locale: Locale;
  t: Translator;
  snapshot: TraceSnapshot | null;
  loading: boolean;
  usingCache: boolean;
  error: string | null;
  onRefresh: () => void;
};

type ProjectGroup = {
  key: string;
  name: string;
  path: string;
  sessions: TraceSessionSummary[];
  activeCount: number;
  analyzedCount: number;
};

type BatchAnalysisState = {
  projectKey: string;
  projectName: string;
  total: number;
  completed: number;
  failed: number;
  failedSessionIds: string[];
  status: "running" | "completed" | "cancelled";
};

type BatchConfirmation = {
  project: ProjectGroup;
  candidates: TraceSessionSummary[];
};

const TRACE_NAVIGATOR_COLLAPSED_KEY =
  "codex-xray.trace-navigator-collapsed.v1";
const TRACE_SELECTED_SESSION_KEY = "codex-xray.trace-selected-session.v1";
const TRACE_TARGET_SESSION_KEY = "codex-xray.trace-target-session.v1";
const TRACE_TARGET_TURN_KEY = "codex-xray.trace-target-turn.v1";
const TRACE_TARGET_CALL_KEY = "codex-xray.trace-target-call.v1";

type TraceNavigationTarget = {
  sessionId: string | null;
  turnId: string | null;
  callId: string | null;
};

function readTraceNavigationTarget(): TraceNavigationTarget {
  try {
    return {
      sessionId: window.localStorage.getItem(TRACE_TARGET_SESSION_KEY),
      turnId: window.localStorage.getItem(TRACE_TARGET_TURN_KEY),
      callId: window.localStorage.getItem(TRACE_TARGET_CALL_KEY),
    };
  } catch {
    return { sessionId: null, turnId: null, callId: null };
  }
}

function copy(locale: Locale, zh: string, en: string): string {
  return locale === "zh-CN" ? zh : en;
}

function TraceInfoTip({ text }: { text: string }) {
  return (
    <span className="info-tip trace-event-help" tabIndex={0} aria-label={text}>
      <CircleHelp aria-hidden="true" />
      <span className="info-tooltip" role="tooltip">
        {text}
      </span>
    </span>
  );
}

function formatDateTime(value: string | null, locale: Locale): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

function formatClock(value: string | null, locale: Locale): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
}

function formatAuditTimestamp(value: string | null, locale: Locale): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    fractionalSecondDigits: 3,
  }).format(new Date(value));
}

function formatBytes(value: number, locale: Locale): string {
  if (value < 1024) return `${value} B`;
  const megabytes = value >= 1024 * 1024;
  return `${new Intl.NumberFormat(locale, {
    maximumFractionDigits: 1,
  }).format(value / (megabytes ? 1024 * 1024 : 1024))} ${megabytes ? "MB" : "KB"}`;
}

function formatEventDuration(value: number, locale: Locale): string {
  if (value < 1_000) return `${Math.max(value, 0)} ms`;
  if (value < 60_000) {
    return `${new Intl.NumberFormat(locale, {
      maximumFractionDigits: 1,
    }).format(value / 1_000)} s`;
  }
  return formatDuration(value / 1_000);
}

function statusLabel(
  status: TraceSessionSummary["status"] | TraceTurnSummary["status"],
  t: Translator,
): string {
  if (status === "running") return t("trace.status.running");
  if (status === "waiting_approval") return t("trace.status.waitingApproval");
  if (status === "waiting_input") return t("trace.status.waitingInput");
  if (status === "completed") return t("trace.status.completed");
  if (status === "failed") return t("trace.status.failed");
  if (status === "interrupted") return t("trace.status.interrupted");
  return t("trace.status.unknown");
}

function isActiveStatus(status: TraceSessionSummary["status"]): boolean {
  return ["running", "waiting_approval", "waiting_input"].includes(status);
}

function conversationLabel(
  session: TraceSessionSummary,
  locale: Locale,
): string {
  if (session.conversation_name?.trim()) return session.conversation_name.trim();
  return copy(
    locale,
    `${formatDateTime(session.started_at ?? session.updated_at, locale)} 的对话`,
    `Conversation · ${formatDateTime(session.started_at ?? session.updated_at, locale)}`,
  );
}

function mergeCatalogMetadata(
  detail: TraceSessionDetail,
  catalog: TraceSessionSummary,
): TraceSessionDetail {
  return {
    ...detail,
    session: {
      ...detail.session,
      conversation_name:
        catalog.conversation_name ?? detail.session.conversation_name,
      project: catalog.project,
      project_path: catalog.project_path,
      session_path: catalog.session_path,
      started_at: catalog.started_at ?? detail.session.started_at,
      updated_at: catalog.updated_at ?? detail.session.updated_at,
      status:
        catalog.status === "unknown" ? detail.session.status : catalog.status,
      parent_id: catalog.parent_id ?? detail.session.parent_id,
      is_subagent: catalog.is_subagent || detail.session.is_subagent,
      analysis_state: catalog.analysis_state,
    },
  };
}

function toolAction(
  name: string,
  locale: Locale,
  args: TraceTimelineEvent["arguments"] = [],
): string {
  const normalized = name.toLowerCase();
  const argumentKeys = new Set(args.map((argument) => argument.key));
  if (argumentKeys.has("weather")) {
    return copy(locale, "查询天气", "Query weather");
  }
  if (argumentKeys.has("finance")) {
    return copy(locale, "查询行情", "Query markets");
  }
  if (argumentKeys.has("sports")) {
    return copy(locale, "查询赛事", "Query sports");
  }
  if (argumentKeys.has("time")) {
    return copy(locale, "查询时间", "Query time");
  }
  if (argumentKeys.has("image_query")) {
    return copy(locale, "搜索图片", "Search images");
  }
  if (argumentKeys.has("search_query")) {
    return copy(locale, "搜索网页", "Search the web");
  }
  if (
    argumentKeys.has("open") ||
    argumentKeys.has("click") ||
    argumentKeys.has("find")
  ) {
    return copy(locale, "浏览网页", "Browse pages");
  }
  if (
    normalized === "exec" ||
    normalized === "exec_command" ||
    normalized.includes("exec_command") ||
    normalized.endsWith(".exec") ||
    normalized.endsWith(".shell")
  ) {
    return copy(locale, "运行命令", "Run command");
  }
  if (normalized.includes("apply_patch")) {
    return copy(locale, "修改文件", "Edit files");
  }
  if (
    normalized.includes("read_file") ||
    normalized.includes("read_mcp_resource")
  ) {
    return copy(locale, "读取内容", "Read content");
  }
  if (normalized.includes("write_stdin") || normalized.endsWith(".wait")) {
    return copy(locale, "等待运行结果", "Wait for result");
  }
  if (
    normalized.includes("web__run") ||
    normalized === "web.run" ||
    normalized.startsWith("web.") ||
    normalized.includes("search_query") ||
    normalized.includes("browser")
  ) {
    return copy(locale, "查询网页", "Browse web");
  }
  if (normalized.includes("imagegen") || normalized.includes("image_gen")) {
    return copy(locale, "生成图片", "Generate image");
  }
  if (normalized.includes("node_repl")) {
    return copy(locale, "执行自动化脚本", "Run automation");
  }
  if (
    normalized.includes("spawn_agent") ||
    normalized.includes("create_thread")
  ) {
    return copy(locale, "启动子任务", "Start subtask");
  }
  if (normalized.includes("jira")) {
    return copy(locale, "查询 Jira", "Query Jira");
  }
  if (normalized.includes("search") || normalized.includes("query")) {
    return copy(locale, "查询数据", "Query data");
  }
  if (normalized.includes("read") || normalized.includes("get_")) {
    return copy(locale, "读取数据", "Read data");
  }
  if (
    normalized.includes("write") ||
    normalized.includes("edit") ||
    normalized.includes("update")
  ) {
    return copy(locale, "更新数据", "Update data");
  }
  return copy(locale, "调用工具", "Call tool");
}

function isHostedWebTool(event: TraceTimelineEvent): boolean {
  const normalized = event.label.toLowerCase();
  return (
    normalized === "web.run" ||
    normalized === "web__run" ||
    normalized.startsWith("web.")
  );
}

function toolRuntimeLabel(
  event: TraceTimelineEvent,
  locale: Locale,
): string {
  if (isHostedWebTool(event)) {
    return copy(locale, "Codex Web 工具", "Codex Web tool");
  }
  if (event.category === "mcp") {
    return event.server
      ? `MCP · ${event.server}`
      : copy(locale, "MCP 服务", "MCP server");
  }
  if (event.category === "cli") {
    return copy(locale, "本机命令进程", "Local command process");
  }
  if (event.category === "file") {
    return copy(locale, "本机文件系统", "Local file system");
  }
  if (event.category === "browser") {
    return copy(locale, "浏览器控制运行时", "Browser control runtime");
  }
  if (event.category === "agent") {
    return copy(locale, "Codex 任务运行时", "Codex task runtime");
  }
  return copy(locale, "工具运行时", "Tool runtime");
}

type EventFilter =
  | "all"
  | "input"
  | "model"
  | "tools"
  | "usage"
  | "lifecycle"
  | "context";

const eventFilters: EventFilter[] = [
  "all",
  "input",
  "model",
  "tools",
  "usage",
  "lifecycle",
  "context",
];

function isToolTimelineEvent(event: TraceTimelineEvent): boolean {
  return (
    event.kind === "tool_request" ||
    event.kind === "tool_execution" ||
    event.kind === "tool_result"
  );
}

function sourceLineLabel(event: TraceTimelineEvent): string | null {
  if (event.source_order <= 0) return null;
  if (
    event.source_end_order != null &&
    event.source_end_order > event.source_order
  ) {
    return `L${event.source_order}-L${event.source_end_order}`;
  }
  return `L${event.source_order}`;
}

function eventFilterLabel(filter: EventFilter, locale: Locale): string {
  const labels: Record<EventFilter, [string, string]> = {
    all: ["全部", "All"],
    input: ["输入", "Input"],
    model: ["LLM", "LLM"],
    tools: ["工具", "Tools"],
    usage: ["用量", "Usage"],
    lifecycle: ["状态", "State"],
    context: ["上下文", "Context"],
  };
  return copy(locale, ...labels[filter]);
}

function matchesEventFilter(
  event: TraceTimelineEvent,
  filter: EventFilter,
): boolean {
  if (filter === "all") return true;
  if (filter === "tools") {
    return [
      "mcp",
      "cli",
      "skill",
      "file",
      "browser",
      "automation",
      "agent",
      "tool",
    ].includes(event.category);
  }
  return event.category === filter;
}

function eventCategoryLabel(
  event: TraceTimelineEvent,
  locale: Locale,
): string {
  if (event.category === "input") return copy(locale, "输入", "INPUT");
  if (event.category === "model") return "LLM";
  if (event.category === "usage") return copy(locale, "用量", "USAGE");
  if (event.category === "mcp") return "MCP";
  if (event.category === "cli") return "CLI";
  if (event.category === "skill") return "SKILL";
  if (event.category === "file") return copy(locale, "文件", "FILE");
  if (event.category === "browser") {
    return isHostedWebTool(event)
      ? "WEB"
      : copy(locale, "浏览器", "BROWSER");
  }
  if (event.category === "automation") return copy(locale, "自动化", "AUTO");
  if (event.category === "agent") return "AGENT";
  if (event.category === "context") return copy(locale, "上下文", "CONTEXT");
  if (event.category === "lifecycle") {
    return event.kind === "started"
      ? copy(locale, "本地", "LOCAL")
      : copy(locale, "状态", "STATE");
  }
  return "TOOL";
}

function phaseEventTitle(event: TraceTimelineEvent, locale: Locale): string {
  if (event.label === "user_prompt") {
    return copy(locale, "用户输入", "User input");
  }
  if (event.label === "reasoning") {
    return copy(locale, "内部推理", "Reasoning");
  }
  if (event.label === "commentary") {
    return copy(locale, "过程说明", "Progress message");
  }
  if (event.label === "final_answer") {
    return copy(locale, "最终回复", "Final response");
  }
  return copy(locale, "助手消息", "Assistant message");
}

function eventExplanation(
  event: TraceTimelineEvent,
  turn: TraceTurnSummary,
  modelPass: number | null,
  locale: Locale,
): string {
  const mirroredRecordNote =
    event.kind === "phase" &&
    event.source_end_order != null &&
    event.source_end_order > event.source_order
      ? copy(
          locale,
          " 同一内容同时保存在事件消息和模型响应记录中，这里合并成一个语义节点。",
          " The same content is stored in both an event message and a model response record; they are merged into one semantic node here.",
        )
      : "";
  if (event.kind === "started") {
    const settings = [
      turn.model && turn.model !== "unknown" ? turn.model : null,
      turn.reasoning_effort
        ? copy(
            locale,
            `推理强度 ${turn.reasoning_effort}`,
            `reasoning ${turn.reasoning_effort}`,
          )
        : null,
      turn.summary_mode
        ? `Summary ${turn.summary_mode}`
        : null,
    ].filter(Boolean);
    return copy(
      locale,
      `这一组记录包含会话建立、回合开始、Developer 指令、自动注入环境、world_state 与 turn_context${settings.length > 0 ? `；本轮使用 ${settings.join(" · ")}` : ""}。它们共同组成首次模型处理前的上下文准备，不是 LLM 返回内容。`,
      `This record group contains session creation, turn start, developer instructions, injected environment, world_state, and turn_context${settings.length > 0 ? `; this turn uses ${settings.join(" · ")}` : ""}. Together they prepare context before the first model pass; they are not LLM output.`,
    );
  }
  if (event.kind === "completed") {
    return copy(
      locale,
      "本轮所有模型处理和工具调用已经结束，Codex 写入最终状态与耗时。",
      "All model processing and tool calls finished; Codex recorded the final state and duration.",
    );
  }
  if (event.kind === "tokens") {
    const sequence = event.sequence ?? 1;
    const includesToolOutput =
      sequence > 1 && turn.tool_calls > 0
        ? copy(
            locale,
            "，其中还包括前面工具返回的结果",
            ", including output returned by earlier tools",
          )
        : "";
    return copy(
      locale,
      `这条 token_count 确认第 ${sequence} 次 LLM 处理已经完成，并记录它读入和生成的 Token${includesToolOutput}。Session 不单独保存“开始调用 LLM”事件。`,
      `This token_count confirms that LLM pass ${sequence} finished and records the tokens it read and generated${includesToolOutput}. The Session does not store a separate “LLM call started” event.`,
    );
  }
  if (event.kind === "tool_request") {
    const passLabel = modelPass ? `LLM #${modelPass}` : "LLM";
    const action = toolAction(event.label, locale, event.arguments);
    return copy(
      locale,
      `${passLabel} 返回了 ${event.label} 的调用名称与参数，用于${action}。这条记录只表示请求已经产生，还不能证明工具已经执行完成。`,
      `${passLabel} returned the ${event.label} call name and arguments to ${action.toLowerCase()}. This record proves that the request was created, not that execution finished.`,
    );
  }
  if (event.kind === "tool_execution") {
    const runtime = toolRuntimeLabel(event, locale);
    return copy(
      locale,
      `Codex 调度 ${runtime} 执行请求；这条独立记录证明执行已经结束。耗时从同一 Call ID 的工具请求时间计算到本记录时间。`,
      `Codex dispatched the request through the ${runtime}. This independent record proves execution ended; duration is measured from the request with the same Call ID to this record.`,
    );
  }
  if (event.kind === "tool_result") {
    return copy(
      locale,
      `Codex 将工具输出写入 Session，并通过同一 Call ID 与请求关联。后续 LLM 处理会把这份结果作为上下文输入。`,
      `Codex wrote the tool output into the Session and linked it to the request through the same Call ID. The next LLM pass receives this output as context.`,
    );
  }
  if (event.kind === "compaction") {
    const windowLabel = event.sequence
      ? copy(locale, `第 ${event.sequence} 个上下文窗口`, `context window ${event.sequence}`)
      : copy(locale, "一个新的上下文窗口", "a new context window");
    const historyLabel = event.content_parts > 0
      ? copy(
          locale,
          `替换后的历史包含 ${event.content_parts} 条结构化记录`,
          `the replacement history contains ${event.content_parts} structured records`,
        )
      : copy(locale, "Session 未记录替换后的条目数", "the Session did not record the replacement item count");
    const reclaimedLabel = event.context_reclaimed_tokens != null
      ? copy(
          locale,
          `；按压缩前后相邻模型输入估算，减少了 ${formatExactTokens(event.context_reclaimed_tokens)} Token`,
          `; adjacent model-input records estimate ${formatExactTokens(event.context_reclaimed_tokens)} fewer tokens`,
        )
      : "";
    return copy(
      locale,
      `Codex 生成了${windowLabel}，${historyLabel}。压缩摘要以加密内容保存在 Session 中，X-Ray 只能确认大小，不能还原正文${reclaimedLabel}。`,
      `Codex created ${windowLabel}; ${historyLabel}. The compacted summary is encrypted in the Session, so X-Ray can verify its size but cannot recover the text${reclaimedLabel}.`,
    );
  }
  if (event.label === "user_prompt") {
    return copy(
      locale,
      "这是用户真正提交给 Codex 的内容，不包括系统自动注入的环境和权限信息。",
      "This is the user's actual message, excluding environment and permission data injected by Codex.",
    ) + mirroredRecordNote;
  }
  if (event.label === "commentary") {
    return (
      copy(
      locale,
      `${modelPass ? `LLM #${modelPass}` : "LLM"} 返回了这段过程说明；它会先显示给用户，但还不是最终回复。`,
      `${modelPass ? `LLM #${modelPass}` : "The LLM"} returned this progress message. Codex showed it to the user, but it was not the final response.`,
      ) + mirroredRecordNote
    );
  }
  if (event.label === "reasoning") {
    return copy(
      locale,
      `${modelPass ? `LLM #${modelPass}` : "LLM"} 返回了一条加密的 Reasoning 记录。Session 只能证明这一步存在并显示记录大小，不能还原推理正文。`,
      `${modelPass ? `LLM #${modelPass}` : "The LLM"} returned an encrypted reasoning record. The Session proves that the step exists and exposes its size, but not readable reasoning text.`,
    );
  }
  if (event.label === "final_answer") {
    const afterTool =
      turn.tool_calls > 0
        ? copy(
            locale,
            "结合前面工具返回的结果，",
            "Using the earlier tool output, ",
          )
        : "";
    return (
      copy(
      locale,
      `${afterTool}${modelPass ? `LLM #${modelPass}` : "LLM"} 生成了这段最终回复，Codex 将它展示给用户。`,
      `${afterTool}${modelPass ? `LLM #${modelPass}` : "The LLM"} generated this final response, which Codex displayed to the user.`,
      ) + mirroredRecordNote
    );
  }
  return copy(
    locale,
    "这是模型返回的一条消息记录。",
    "This is a message returned by the model.",
  );
}

function TimelineEventRow({
  event,
  turn,
  modelPass,
  locale,
}: {
  event: TraceTimelineEvent;
  turn: TraceTurnSummary;
  modelPass: number | null;
  locale: Locale;
}) {
  const labels: Record<TraceTimelineEvent["kind"], [string, string]> = {
    started: ["本地准备", "Local preparation"],
    completed: ["回合结束", "Turn finished"],
    tokens: ["模型用量", "Model usage"],
    phase: ["消息", "Message"],
    tool_request: ["工具请求", "Tool request"],
    tool_execution: ["工具执行完成", "Tool execution finished"],
    tool_result: ["工具结果写回", "Tool output written"],
    compaction: ["上下文压缩", "Context compacted"],
  };
  const freshInput = Math.max(
    event.input_tokens - event.cached_input_tokens,
    0,
  );
  const title = isToolTimelineEvent(event)
    ? event.kind === "tool_request"
      ? copy(
          locale,
          `${modelPass ? `LLM #${modelPass}` : "LLM"} 返回工具请求 · ${toolAction(event.label, locale, event.arguments)}`,
          `${modelPass ? `LLM #${modelPass}` : "LLM"} returned tool request · ${toolAction(event.label, locale, event.arguments)}`,
        )
      : event.kind === "tool_execution"
        ? copy(
            locale,
            `${toolRuntimeLabel(event, locale)}执行完成 · ${toolAction(event.label, locale, event.arguments)}`,
            `${toolRuntimeLabel(event, locale)} finished · ${toolAction(event.label, locale, event.arguments)}`,
          )
        : copy(
            locale,
            `工具结果写回上下文 · ${toolAction(event.label, locale, event.arguments)}`,
            `Tool output written to context · ${toolAction(event.label, locale, event.arguments)}`,
          )
    : event.kind === "phase"
        ? event.category === "model" && modelPass
          ? copy(
              locale,
              `LLM #${modelPass} 返回 · ${phaseEventTitle(event, locale)}`,
              `LLM #${modelPass} returned · ${phaseEventTitle(event, locale)}`,
            )
          : phaseEventTitle(event, locale)
        : event.kind === "tokens" && event.sequence != null
          ? copy(
              locale,
              `LLM #${event.sequence} · 用量凭据`,
              `LLM #${event.sequence} · usage record`,
            )
        : copy(locale, ...labels[event.kind]);
  const factualState =
    event.status === "failed"
      ? copy(locale, "失败", "Failed")
      : event.status === "pending"
        ? copy(locale, "未完成", "Pending")
        : event.repeated
          ? copy(locale, "重复调用", "Repeated")
        : null;
  const contextPercent =
    event.context_window && event.context_window > 0
      ? (event.input_tokens / event.context_window) * 100
      : null;
  const hasAuditDetail =
    isToolTimelineEvent(event) &&
    (event.call_id != null ||
      event.source_type != null ||
      event.completed_at != null ||
      event.arguments_json != null ||
      event.result_json != null ||
      event.result_fields.length > 0);

  return (
    <li
      className={`trace-lite-event ${event.kind} ${event.status} category-${event.category}`}
      data-trace-call={event.call_id ?? undefined}
    >
      <time>{formatClock(event.timestamp, locale)}</time>
      <span className="trace-lite-event-dot" aria-hidden="true" />
      <div className="trace-lite-event-body">
        <div className="trace-lite-event-title">
          <span className={`trace-event-kind ${event.category}`}>
            {eventCategoryLabel(event, locale)}
          </span>
          <strong>{title}</strong>
          <TraceInfoTip
            text={eventExplanation(event, turn, modelPass, locale)}
          />
          {(isToolTimelineEvent(event) ||
            event.kind === "tokens" ||
            event.kind === "phase") && (
            <code title={event.label}>{event.label}</code>
          )}
          {sourceLineLabel(event) && (
            <code
              className="trace-source-range"
              title={event.source_type ?? undefined}
            >
              {sourceLineLabel(event)}
            </code>
          )}
        </div>
        {event.server && (
          <div className="trace-lite-server">
            <span>MCP Server</span>
            <code>{event.server}</code>
          </div>
        )}
        {isToolTimelineEvent(event) && event.subject && (
          <code className="trace-lite-event-subject">{event.subject}</code>
        )}
        {event.detail && (
          <code className="trace-lite-event-detail">{event.detail}</code>
        )}
        {event.kind === "tokens" && (
          <div className="trace-lite-token-event">
            <span>
              {copy(locale, "未缓存", "Fresh")}{" "}
              <b>{formatExactTokens(freshInput)}</b>
            </span>
            <span>
              {copy(locale, "缓存", "Cache")}{" "}
              <b>{formatExactTokens(event.cached_input_tokens)}</b>
            </span>
            <span>
              {copy(locale, "输出", "Output")}{" "}
              <b>{formatExactTokens(event.output_tokens)}</b>
            </span>
            <span>
              {copy(locale, "推理", "Reasoning")}{" "}
              <b>{formatExactTokens(event.reasoning_output_tokens)}</b>
            </span>
            <span>
              {copy(locale, "合计", "Total")}{" "}
              <b>{formatExactTokens(event.total_tokens)}</b>
            </span>
            {event.context_window != null && (
              <span>
                {copy(locale, "上下文", "Context")}{" "}
                <b>
                  {formatExactTokens(event.input_tokens)} /{" "}
                  {formatExactTokens(event.context_window)}
                  {contextPercent == null
                    ? ""
                    : ` · ${contextPercent.toFixed(1)}%`}
                </b>
              </span>
            )}
            {event.context_delta_tokens != null && (
              <span>
                {copy(locale, "较上次输入", "Input delta")}{" "}
                <b>
                  {event.context_delta_tokens >= 0 ? "+" : ""}
                  {formatExactTokens(event.context_delta_tokens)}
                </b>
              </span>
            )}
            {event.cache_hit_percent != null && (
              <span>
                {copy(locale, "缓存命中", "Cache hit")}{" "}
                <b>{event.cache_hit_percent.toFixed(2)}%</b>
              </span>
            )}
            {event.estimated_cost_usd != null && (
              <span>
                {copy(locale, "API 等价成本", "API-equivalent cost")}{" "}
                <b>{formatExactUsd(event.estimated_cost_usd)}</b>
              </span>
            )}
          </div>
        )}
        {event.kind === "compaction" && (
          <div className="trace-lite-event-meta trace-compaction-meta">
            {event.sequence != null && (
              <span>
                {copy(locale, "窗口", "Window")} <b>#{event.sequence}</b>
              </span>
            )}
            {event.content_parts > 0 && (
              <span>
                {copy(locale, "替换后历史", "Replacement history")} {" "}
                <b>{event.content_parts}</b>
              </span>
            )}
            {event.encrypted_bytes > 0 && (
              <span>
                {copy(locale, "加密摘要", "Encrypted summary")} {" "}
                <b>{formatBytes(event.encrypted_bytes, locale)}</b>
              </span>
            )}
            {event.context_before_tokens != null && (
              <span>
                {copy(locale, "压缩前", "Before")} {" "}
                <b>{formatExactTokens(event.context_before_tokens)}</b>
              </span>
            )}
            {event.context_after_tokens != null && (
              <span>
                {copy(locale, "压缩后", "After")} {" "}
                <b>{formatExactTokens(event.context_after_tokens)}</b>
              </span>
            )}
            {event.context_reclaimed_tokens != null && (
              <span className="reclaimed">
                {copy(locale, "估算减少", "Estimated reduction")} {" "}
                <b>{formatExactTokens(event.context_reclaimed_tokens)}</b>
              </span>
            )}
          </div>
        )}
        {event.content && (
          <div className="trace-event-content">{event.content}</div>
        )}
        {event.kind === "phase" && !event.content && (
          <div className="trace-lite-event-meta">
            {event.content_parts > 0 && (
              <span>
                {copy(locale, "内容", "Content")}{" "}
                <b>{event.content_parts}</b>
              </span>
            )}
            {event.content_bytes > 0 && (
              <span>
                {copy(locale, "记录大小", "Record size")}{" "}
                <b>{formatBytes(event.content_bytes, locale)}</b>
              </span>
            )}
            {event.summary_parts > 0 && (
              <span>
                {copy(locale, "摘要", "Summaries")}{" "}
                <b>{event.summary_parts}</b>
              </span>
            )}
            {event.encrypted_bytes > 0 && (
              <span>
                {copy(locale, "加密内容", "Encrypted content")}{" "}
                <b>{formatBytes(event.encrypted_bytes, locale)}</b>
              </span>
            )}
          </div>
        )}
        {event.kind !== "tokens" &&
          event.kind !== "tool_request" &&
          (event.duration_ms != null ||
            event.output_bytes > 0 ||
            event.exit_code != null) && (
            <div className="trace-lite-event-meta">
              {event.duration_ms != null && (
                <span>
                  {event.kind === "tool_execution"
                    ? copy(locale, "请求到执行完成", "Request to execution end")
                    : event.kind === "tool_result"
                      ? copy(locale, "写回间隔", "Write-back interval")
                      : copy(locale, "耗时", "Duration")}{" "}
                  <b>{formatEventDuration(event.duration_ms, locale)}</b>
                </span>
              )}
              {event.exit_code != null && (
                <span>
                  Exit code <b>{event.exit_code}</b>
                </span>
              )}
              {event.output_bytes > 0 && (
                <span>
                  {copy(locale, "结果记录", "Result record")}{" "}
                  <b>{formatBytes(event.output_bytes, locale)}</b>
                </span>
              )}
              {event.kind === "tool_result" && modelPass != null && (
                <span>
                  {copy(locale, "下一步", "Next")}{" "}
                  <b>LLM #{modelPass + 1}</b>
                </span>
              )}
            </div>
          )}
        {hasAuditDetail && (
          <details className="trace-call-inspector">
            <summary>
              <span>{copy(locale, "详情", "Details")}</span>
            </summary>
            <div className="trace-call-inspector-body">
              <dl className="trace-call-identity">
                {event.source_type && (
                  <div className="trace-call-identity-wide">
                    <dt>
                      {event.kind === "tool_execution"
                        ? copy(locale, "执行记录", "Execution record")
                        : event.kind === "tool_result"
                          ? copy(locale, "结果记录", "Output record")
                          : copy(locale, "请求记录", "Request record")}
                    </dt>
                    <dd><code>{event.source_type}</code></dd>
                  </div>
                )}
                {event.execution_end_source_type && (
                  <div className="trace-call-identity-wide">
                    <dt>{copy(locale, "执行结束", "Execution ended")}</dt>
                    <dd><code>{event.execution_end_source_type}</code></dd>
                  </div>
                )}
                {event.result_source_type && (
                  <div className="trace-call-identity-wide">
                    <dt>{copy(locale, "结果记录", "Output record")}</dt>
                    <dd><code>{event.result_source_type}</code></dd>
                  </div>
                )}
                {event.call_id && (
                  <div className="trace-call-identity-wide">
                    <dt>Call ID</dt>
                    <dd><code>{event.call_id}</code></dd>
                  </div>
                )}
                <div className="trace-call-identity-time">
                  <dt>
                    {event.kind === "tool_execution"
                      ? copy(locale, "执行完成", "Execution completed")
                      : event.kind === "tool_result"
                        ? copy(locale, "结果写回", "Output written")
                        : copy(locale, "请求产生", "Request created")}
                  </dt>
                  <dd>{formatAuditTimestamp(event.timestamp, locale)}</dd>
                </div>
                {event.execution_completed_at && (
                  <div className="trace-call-identity-time">
                    <dt>{copy(locale, "执行完成", "Execution completed")}</dt>
                    <dd>
                      {formatAuditTimestamp(
                        event.execution_completed_at,
                        locale,
                      )}
                    </dd>
                  </div>
                )}
                {event.completed_at && (
                  <div className="trace-call-identity-time">
                    <dt>{copy(locale, "结果写回", "Output written")}</dt>
                    <dd>{formatAuditTimestamp(event.completed_at, locale)}</dd>
                  </div>
                )}
                {event.server && (
                  <div className="trace-call-identity-wide">
                    <dt>MCP Server</dt>
                    <dd><code>{event.server}</code></dd>
                  </div>
                )}
              </dl>
              {event.arguments_json && (
                <section>
                  <h5>
                    {copy(
                      locale,
                      "模型交给工具的参数",
                      "Arguments from model to tool",
                    )}
                  </h5>
                  <pre className="trace-call-json">{event.arguments_json}</pre>
                </section>
              )}
              {event.result_json && (
                <section>
                  <h5>
                    {copy(
                      locale,
                      "工具返回给模型的结果",
                      "Output returned to model",
                    )}
                  </h5>
                  <pre className="trace-call-json">{event.result_json}</pre>
                </section>
              )}
              {!event.result_json && event.result_fields.length > 0 && (
                <section>
                  <h5>{copy(locale, "返回", "Output")}</h5>
                  <dl className="trace-call-result">
                    {event.result_fields.map((field, index) => (
                      <div key={`${field.key}-${index}`}>
                        <dt>{field.key}</dt>
                        <dd>
                          <code>{field.value}</code>
                        </dd>
                      </div>
                    ))}
                  </dl>
                </section>
              )}
            </div>
          </details>
        )}
      </div>
      {factualState && (
        <span className={`trace-lite-event-state ${event.status}`}>
          {factualState}
        </span>
      )}
    </li>
  );
}

function TurnTimeline({
  turn,
  locale,
  t,
  eventFilter,
  open,
  onToggle,
}: {
  turn: TraceTurnSummary;
  locale: Locale;
  t: Translator;
  eventFilter: EventFilter;
  open: boolean;
  onToggle: () => void;
}) {
  const contextLabel =
    turn.context_window && turn.context_window > 0
      ? `${formatReadableTokens(turn.peak_input_tokens)} / ${formatReadableTokens(turn.context_window)}`
      : formatReadableTokens(turn.peak_input_tokens);
  const visibleEvents = turn.timeline.filter((event) =>
    matchesEventFilter(event, eventFilter),
  );
  const modelPassByEvent = new Map<TraceTimelineEvent, number>();
  let completedModelPasses = 0;
  for (const event of turn.timeline) {
    if (event.kind === "tokens") {
      const pass = event.sequence ?? completedModelPasses + 1;
      modelPassByEvent.set(event, pass);
      completedModelPasses = Math.max(completedModelPasses, pass);
    } else if (event.category === "model" || isToolTimelineEvent(event)) {
      modelPassByEvent.set(event, completedModelPasses + 1);
    }
  }
  return (
    <article
      className={`trace-lite-turn${open ? " open" : ""}`}
      data-trace-turn={turn.id}
    >
      <button
        className="trace-lite-turn-summary"
        onClick={onToggle}
        aria-expanded={open}
      >
        <span className="trace-lite-turn-number">{turn.sequence}</span>
        <span className="trace-lite-turn-name">
          <strong>
            {copy(locale, `回合 ${turn.sequence}`, `Turn ${turn.sequence}`)}
          </strong>
          <small>
            {formatDateTime(turn.started_at, locale)}
            {turn.model && turn.model !== "unknown" ? ` · ${turn.model}` : ""}
            {turn.reasoning_effort
              ? ` · ${copy(locale, "推理", "reasoning")} ${turn.reasoning_effort}`
              : ""}
            {turn.summary_mode ? ` · Summary ${turn.summary_mode}` : ""}
          </small>
        </span>
        <span className="trace-lite-turn-stat">
          <strong>{formatReadableTokens(turn.total_tokens)}</strong>
          <small>Token</small>
        </span>
        <span className="trace-lite-turn-stat context">
          <strong>{contextLabel}</strong>
          <small>{copy(locale, "上下文峰值 / 窗口", "Context peak / window")}</small>
        </span>
        <span className="trace-lite-turn-stat tools">
          <strong>{turn.tool_calls}</strong>
          <small>{copy(locale, "工具调用", "tool calls")}</small>
        </span>
        <span className="trace-lite-turn-status">
          <i className={`trace-status-dot ${turn.status}`} />
          {statusLabel(turn.status, t)}
        </span>
        <ChevronRight
          className="trace-lite-turn-chevron"
          aria-hidden="true"
        />
      </button>

      {open && (
        <div className="trace-lite-turn-detail">
          <div className="trace-lite-resources">
            <span>
              <small>{copy(locale, "输入", "Input")}</small>
              <strong>{formatExactTokens(turn.input_tokens)}</strong>
              <em>
                {copy(locale, "未缓存", "Fresh")}{" "}
                {formatExactTokens(turn.uncached_input_tokens)}
              </em>
            </span>
            <span>
              <small>{copy(locale, "缓存读取", "Cache read")}</small>
              <strong>{formatExactTokens(turn.cached_input_tokens)}</strong>
              <em>{turn.cache_hit_percent.toFixed(1)}%</em>
            </span>
            <span>
              <small>{copy(locale, "输出", "Output")}</small>
              <strong>{formatExactTokens(turn.output_tokens)}</strong>
              <em>
                {copy(locale, "推理", "Reasoning")}{" "}
                {formatExactTokens(turn.reasoning_output_tokens)}
              </em>
            </span>
            <span>
              <small>{copy(locale, "上下文峰值", "Context peak")}</small>
              <strong>{formatExactTokens(turn.peak_input_tokens)}</strong>
              <em>
                {turn.context_utilization_percent == null
                  ? copy(locale, "窗口未知", "Window unknown")
                  : `${turn.context_utilization_percent.toFixed(1)}% · ${formatReadableTokens(turn.context_window ?? 0)}`}
              </em>
            </span>
            <span>
              <small>{copy(locale, "输入增长", "Input growth")}</small>
              <strong>{formatExactTokens(turn.context_growth_tokens)}</strong>
              <em>
                {turn.context_compactions}{" "}
                {copy(locale, "次压缩", "compactions")}
              </em>
            </span>
            <span>
              <small>{copy(locale, "耗时", "Duration")}</small>
              <strong>
                {turn.duration_ms == null
                  ? "—"
                  : formatEventDuration(turn.duration_ms, locale)}
              </strong>
              <em>{formatExactUsd(turn.estimated_cost_usd)}</em>
            </span>
          </div>

          <section className="trace-context-flow">
            <header>
              <strong>{copy(locale, "本轮上下文", "Context in this turn")}</strong>
              <TraceInfoTip
                text={copy(
                  locale,
                  "首次、峰值和末次输入来自每次 token_count 的 input_tokens；它们是模型实际读入的上下文。压缩减少量只在压缩前后都有相邻用量记录时估算。",
                  "First, peak, and final input come from input_tokens in each token_count record—the context actually read by the model. Compaction reduction is estimated only when adjacent usage records exist on both sides.",
                )}
              />
            </header>
            <div className="trace-context-stages">
              <span>
                <small>{copy(locale, "首次模型输入", "First model input")}</small>
                <strong>{formatExactTokens(turn.first_input_tokens)}</strong>
              </span>
              <i aria-hidden="true" />
              <span>
                <small>{copy(locale, "上下文峰值", "Context peak")}</small>
                <strong>{formatExactTokens(turn.peak_input_tokens)}</strong>
              </span>
              <i aria-hidden="true" />
              <span>
                <small>{copy(locale, "末次模型输入", "Final model input")}</small>
                <strong>{formatExactTokens(turn.last_input_tokens)}</strong>
              </span>
            </div>
            <dl>
              <div>
                <dt>{copy(locale, "模型处理", "Model passes")}</dt>
                <dd>{turn.model_passes}</dd>
              </div>
              <div>
                <dt>{copy(locale, "上下文压缩", "Compactions")}</dt>
                <dd>
                  {turn.context_compactions}
                  {turn.estimated_reclaimed_tokens > 0
                    ? copy(
                        locale,
                        ` · 估算减少 ${formatReadableTokens(turn.estimated_reclaimed_tokens)}`,
                        ` · ~${formatReadableTokens(turn.estimated_reclaimed_tokens)} reduced`,
                      )
                    : ""}
                </dd>
              </div>
              <div>
                <dt>
                  {copy(locale, "本地准备记录", "Local preparation")}
                  <TraceInfoTip
                    text={copy(
                      locale,
                      `这是 Session 中可识别的基础指令、工具定义、Developer 指令、world_state 与 turn_context 的记录大小，不是 Token 数。基础与工具 ${formatBytes(turn.session_context_bytes, locale)} · Developer ${formatBytes(turn.developer_context_bytes, locale)} · world_state ${formatBytes(turn.world_state_bytes, locale)} · turn_context ${formatBytes(turn.turn_context_bytes, locale)}。`,
                      `This is the recorded size of recognizable base instructions, tool definitions, developer instructions, world_state, and turn_context—not a token count. Base and tools ${formatBytes(turn.session_context_bytes, locale)} · Developer ${formatBytes(turn.developer_context_bytes, locale)} · world_state ${formatBytes(turn.world_state_bytes, locale)} · turn_context ${formatBytes(turn.turn_context_bytes, locale)}.`,
                    )}
                  />
                </dt>
                <dd>{formatBytes(turn.local_context_bytes, locale)}</dd>
              </div>
              <div>
                <dt>
                  Memory
                  <TraceInfoTip
                    text={copy(
                      locale,
                      "这里只统计 Session 中明确出现的 Memory 注入或 memory_citation。未发现不等于 Memory 被关闭，也不会把压缩摘要算作长期 Memory。",
                      "This counts only explicit Memory injection or memory_citation evidence in the Session. Not observed does not mean Memory is disabled, and compacted summaries are not counted as long-term Memory.",
                    )}
                  />
                </dt>
                <dd>
                  {turn.memory_context_bytes > 0 || turn.memory_citations > 0
                    ? copy(
                        locale,
                        `${turn.memory_citations} 条引用${turn.memory_context_bytes > 0 ? ` · ${formatBytes(turn.memory_context_bytes, locale)} 注入` : ""}`,
                        `${turn.memory_citations} citations${turn.memory_context_bytes > 0 ? ` · ${formatBytes(turn.memory_context_bytes, locale)} injected` : ""}`,
                      )
                    : copy(locale, "未发现使用记录", "No usage evidence")}
                </dd>
              </div>
            </dl>
          </section>

          <section className="trace-lite-events">
            {visibleEvents.length > 0 ? (
              <ol>
                {visibleEvents.map((event, index) => (
                  <TimelineEventRow
                    key={`${event.timestamp}-${event.kind}-${index}`}
                    event={event}
                    turn={turn}
                    modelPass={modelPassByEvent.get(event) ?? null}
                    locale={locale}
                  />
                ))}
              </ol>
            ) : (
              <p className="trace-lite-muted">
                {copy(
                  locale,
                  "这个回合没有符合当前筛选的事件。",
                  "No events in this turn match the current filter.",
                )}
              </p>
            )}
          </section>
        </div>
      )}
    </article>
  );
}

export default function TraceView({
  locale,
  t,
  snapshot,
  loading,
  usingCache,
  error,
  onRefresh,
}: TraceViewProps) {
  const [query, setQuery] = useState("");
  const [navigatorCollapsed, setNavigatorCollapsed] = useState(() => {
    try {
      return (
        window.localStorage.getItem(TRACE_NAVIGATOR_COLLAPSED_KEY) === "true"
      );
    } catch {
      return false;
    }
  });
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () => new Set(),
  );
  const [selectedId, setSelectedId] = useState<string | null>(() => {
    try {
      return window.localStorage.getItem(TRACE_SELECTED_SESSION_KEY);
    } catch {
      return null;
    }
  });
  const [openTurnId, setOpenTurnId] = useState<string | null>(null);
  const [eventFilter, setEventFilter] = useState<EventFilter>("all");
  const [detail, setDetail] = useState<TraceSessionDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [analyzingId, setAnalyzingId] = useState<string | null>(null);
  const [batchAnalysis, setBatchAnalysis] =
    useState<BatchAnalysisState | null>(null);
  const [batchConfirmation, setBatchConfirmation] =
    useState<BatchConfirmation | null>(null);
  const [batchError, setBatchError] = useState<string | null>(null);
  const [analyzedSessions, setAnalyzedSessions] = useState<
    Map<string, TraceSessionSummary>
  >(() => new Map());
  const detailCache = useRef(new Map<string, TraceSessionDetail>());
  const autoAnalysisAttempted = useRef(new Set<string>());
  const seededProject = useRef(false);
  const cancelBatchAnalysis = useRef(false);
  const navigationTarget = useRef<TraceNavigationTarget>(
    readTraceNavigationTarget(),
  );

  const openRequestedTurn = (
    next: TraceSessionDetail,
    sessionId: string,
  ) => {
    const target = navigationTarget.current;
    const requested =
      target.sessionId === sessionId &&
      next.turns.some((turn) => turn.id === target.turnId)
        ? target.turnId
        : null;
    setOpenTurnId(requested ?? next.turns[0]?.id ?? null);
  };

  const allSessions = useMemo(
    () =>
      (snapshot?.sessions ?? []).map(
        (session) => analyzedSessions.get(session.id) ?? session,
      ),
    [analyzedSessions, snapshot?.sessions],
  );
  const eventCounts = useMemo(() => {
    const counts = new Map<EventFilter, number>(
      eventFilters.map((filter) => [filter, 0]),
    );
    for (const turn of detail?.turns ?? []) {
      for (const event of turn.timeline) {
        for (const filter of eventFilters) {
          if (matchesEventFilter(event, filter)) {
            counts.set(filter, (counts.get(filter) ?? 0) + 1);
          }
        }
      }
    }
    return counts;
  }, [detail]);

  const sessions = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase(locale);
    return allSessions
      .filter((session) => {
        if (!normalized) return true;
        return [
          session.conversation_name,
          session.project,
          session.project_path,
          session.id,
          session.model,
        ]
          .join(" ")
          .toLocaleLowerCase(locale)
          .includes(normalized);
      })
      .sort((left, right) =>
        (right.updated_at ?? "").localeCompare(left.updated_at ?? ""),
      );
  }, [allSessions, locale, query]);

  const projectGroups = useMemo<ProjectGroup[]>(() => {
    const groups = new Map<string, ProjectGroup>();
    for (const session of sessions) {
      const key = session.project_path || session.project;
      const group = groups.get(key) ?? {
        key,
        name: session.project,
        path: session.project_path,
        sessions: [],
        activeCount: 0,
        analyzedCount: 0,
      };
      group.sessions.push(session);
      group.activeCount += Number(isActiveStatus(session.status));
      group.analyzedCount += Number(session.analysis_state !== "not_analyzed");
      groups.set(key, group);
    }
    return [...groups.values()];
  }, [sessions]);

  useEffect(() => {
    if (seededProject.current || projectGroups.length === 0) return;
    seededProject.current = true;
    setExpandedProjects(new Set([projectGroups[0].key]));
  }, [projectGroups]);

  useEffect(() => {
    if (!selectedId) return;
    const selectedProject = projectGroups.find((project) =>
      project.sessions.some((session) => session.id === selectedId),
    );
    if (!selectedProject) return;
    setExpandedProjects((current) => {
      if (current.has(selectedProject.key)) return current;
      const next = new Set(current);
      next.add(selectedProject.key);
      return next;
    });
  }, [projectGroups, selectedId]);

  useEffect(() => {
    detailCache.current.clear();
    setAnalyzedSessions(new Map());
  }, [snapshot?.generated_at]);

  useEffect(() => {
    if (
      !snapshot ||
      !selectedId ||
      snapshot.sessions.some((session) => session.id === selectedId)
    ) {
      return;
    }
    setSelectedId(null);
    setDetail(null);
    setNavigatorCollapsed(false);
  }, [selectedId, snapshot?.sessions]);

  const selectedSummary =
    allSessions.find((session) => session.id === selectedId) ?? null;

  useEffect(() => {
    if (!selectedId) {
      setDetail(null);
      setDetailLoading(false);
      return;
    }
    const summary = (snapshot?.sessions ?? []).find(
      (session) => session.id === selectedId,
    );
    if (!summary || summary.analysis_state === "not_analyzed") {
      setDetail(null);
      setDetailLoading(false);
      setDetailError(null);
      return;
    }
    const cached = detailCache.current.get(selectedId);
    if (cached) {
      setDetail(cached);
      openRequestedTurn(cached, selectedId);
      setDetailError(null);
      return;
    }
    let active = true;
    setDetail(null);
    setDetailLoading(true);
    setDetailError(null);
    invoke<TraceSessionDetail | null>("get_trace_session", {
      sessionId: selectedId,
    })
      .then((next) => {
        if (!active) return;
        const normalized = next ? mergeCatalogMetadata(next, summary) : null;
        if (normalized) detailCache.current.set(selectedId, normalized);
        setDetail(normalized);
        if (normalized) openRequestedTurn(normalized, selectedId);
      })
      .catch((reason: unknown) => {
        if (active) setDetailError(String(reason));
      })
      .finally(() => {
        if (active) setDetailLoading(false);
      });
    return () => {
      active = false;
    };
  }, [selectedId, snapshot?.generated_at, snapshot?.sessions]);

  useEffect(() => {
    setEventFilter("all");
  }, [selectedId]);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        TRACE_NAVIGATOR_COLLAPSED_KEY,
        String(navigatorCollapsed),
      );
    } catch {
      // Persistence is optional when the webview blocks local storage.
    }
  }, [navigatorCollapsed]);

  useEffect(() => {
    if (snapshot && !selectedId && navigatorCollapsed) {
      setNavigatorCollapsed(false);
    }
  }, [navigatorCollapsed, selectedId, snapshot]);

  useEffect(() => {
    try {
      if (selectedId) {
        window.localStorage.setItem(TRACE_SELECTED_SESSION_KEY, selectedId);
      } else {
        window.localStorage.removeItem(TRACE_SELECTED_SESSION_KEY);
      }
    } catch {
      // Persistence is optional when the webview blocks local storage.
    }
  }, [selectedId]);

  const analyzeSelected = async () => {
    if (
      !selectedSummary ||
      analyzingId ||
      batchAnalysis?.status === "running"
    ) {
      return;
    }
    setAnalyzingId(selectedSummary.id);
    setDetailLoading(true);
    setDetailError(null);
    try {
      const next = await invoke<TraceSessionDetail | null>(
        "analyze_trace_session",
        {
          sessionId: selectedSummary.id,
          sessionPath: selectedSummary.session_path,
        },
      );
      if (!next) {
        throw new Error(
          copy(
            locale,
            "该 Session 没有可读取的结构化事件。",
            "This session has no readable structured events.",
          ),
        );
      }
      const normalizedDetail = mergeCatalogMetadata(next, {
        ...selectedSummary,
        analysis_state: "ready",
      });
      detailCache.current.set(selectedSummary.id, normalizedDetail);
      setAnalyzedSessions((current) => {
        const updated = new Map(current);
        updated.set(selectedSummary.id, normalizedDetail.session);
        return updated;
      });
      setDetail(normalizedDetail);
      openRequestedTurn(normalizedDetail, selectedSummary.id);
    } catch (reason) {
      setDetailError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setDetailLoading(false);
      setAnalyzingId(null);
    }
  };

  useEffect(() => {
    const target = navigationTarget.current;
    if (
      !selectedSummary ||
      target.sessionId !== selectedSummary.id ||
      selectedSummary.analysis_state !== "not_analyzed" ||
      autoAnalysisAttempted.current.has(selectedSummary.id)
    ) {
      return;
    }
    autoAnalysisAttempted.current.add(selectedSummary.id);
    void analyzeSelected();
  }, [selectedSummary]);

  const requestProjectAnalysis = (project: ProjectGroup) => {
    if (batchAnalysis?.status === "running") {
      if (batchAnalysis.projectKey === project.key) {
        cancelBatchAnalysis.current = true;
      }
      return;
    }

    const candidates = project.sessions.filter(
      (session) =>
        session.analysis_state !== "ready" && Boolean(session.session_path),
    );
    if (candidates.length === 0) {
      setBatchAnalysis(null);
      return;
    }
    setBatchConfirmation({ project, candidates });
  };

  const analyzeProject = async (
    project: ProjectGroup,
    candidates: TraceSessionSummary[],
  ) => {
    setBatchConfirmation(null);
    cancelBatchAnalysis.current = false;
    setBatchError(null);
    setBatchAnalysis({
      projectKey: project.key,
      projectName: project.name,
      total: candidates.length,
      completed: 0,
      failed: 0,
      failedSessionIds: [],
      status: "running",
    });

    let completed = 0;
    let failed = 0;
    const failedSessionIds: string[] = [];
    for (const session of candidates) {
      if (cancelBatchAnalysis.current) break;
      setAnalyzingId(session.id);
      if (session.id === selectedId) {
        setDetailLoading(true);
        setDetailError(null);
      }
      try {
        const next = await invoke<TraceSessionDetail | null>(
          "analyze_trace_session",
          {
            sessionId: session.id,
            sessionPath: session.session_path,
          },
        );
        if (!next) {
          throw new Error(
            copy(
              locale,
              "没有可读取的结构化事件",
              "No readable structured events",
            ),
          );
        }
        const normalizedDetail = mergeCatalogMetadata(next, {
          ...session,
          analysis_state: "ready",
        });
        detailCache.current.set(session.id, normalizedDetail);
        setAnalyzedSessions((current) => {
          const updated = new Map(current);
          updated.set(session.id, normalizedDetail.session);
          return updated;
        });
        if (session.id === selectedId) {
          setDetail(normalizedDetail);
          openRequestedTurn(normalizedDetail, session.id);
        }
      } catch (reason) {
        failed += 1;
        failedSessionIds.push(session.id);
        setBatchError((current) =>
          current ??
          `${conversationLabel(session, locale)}：${
            reason instanceof Error ? reason.message : String(reason)
          }`,
        );
      } finally {
        completed += 1;
        if (session.id === selectedId) setDetailLoading(false);
        setBatchAnalysis((current) =>
          current
            ? {
                ...current,
                completed,
                failed,
                failedSessionIds: [...failedSessionIds],
              }
            : current,
        );
      }
    }

    const cancelled = cancelBatchAnalysis.current;
    setAnalyzingId(null);
    setBatchAnalysis((current) =>
      current
        ? {
            ...current,
            completed,
            failed,
            failedSessionIds: [...failedSessionIds],
            status: cancelled ? "cancelled" : "completed",
          }
        : current,
    );
    onRefresh();
  };

  const retryFailedProjectSessions = () => {
    if (!batchAnalysis || batchAnalysis.failedSessionIds.length === 0) return;
    const project = projectGroups.find(
      (candidate) => candidate.key === batchAnalysis.projectKey,
    );
    if (!project) return;
    const failed = new Set(batchAnalysis.failedSessionIds);
    const candidates = project.sessions.filter(
      (session) => failed.has(session.id) && Boolean(session.session_path),
    );
    if (candidates.length > 0) {
      void analyzeProject(project, candidates);
    }
  };

  useEffect(() => {
    const target = navigationTarget.current;
    if (
      !detail ||
      target.sessionId !== detail.session.id ||
      openTurnId !== target.turnId
    ) {
      return;
    }
    const frame = window.requestAnimationFrame(() => {
      const element = target.callId
        ? [
            ...document.querySelectorAll<HTMLElement>("[data-trace-call]"),
          ].find((candidate) => candidate.dataset.traceCall === target.callId)
        : [
            ...document.querySelectorAll<HTMLElement>("[data-trace-turn]"),
          ].find((candidate) => candidate.dataset.traceTurn === target.turnId);
      if (!element) return;
      element.scrollIntoView({ block: "center", behavior: "smooth" });
      element.classList.add("trace-event-target");
      window.setTimeout(
        () => element.classList.remove("trace-event-target"),
        1_800,
      );
      try {
        window.localStorage.removeItem(TRACE_TARGET_SESSION_KEY);
        window.localStorage.removeItem(TRACE_TARGET_TURN_KEY);
        window.localStorage.removeItem(TRACE_TARGET_CALL_KEY);
      } catch {
        // The in-memory target is enough for this navigation.
      }
      navigationTarget.current = {
        sessionId: null,
        turnId: null,
        callId: null,
      };
    });
    return () => window.cancelAnimationFrame(frame);
  }, [detail, openTurnId]);

  if (!snapshot && loading) {
    return (
      <main className="workspace trace-workspace">
        <div className="trace-empty-state">
          <span className="trace-index-pulse" />
          <div>
            <strong>{t("trace.indexing")}</strong>
            <p>{t("trace.indexingNote")}</p>
          </div>
        </div>
      </main>
    );
  }
  if (!snapshot && error) {
    return (
      <main className="workspace trace-workspace">
        <div className="trace-empty-state error">
          <strong>{t("error.traceTitle")}</strong>
          <p>{error}</p>
          <button onClick={onRefresh}>{t("error.traceRetry")}</button>
        </div>
      </main>
    );
  }
  if (!snapshot) return null;

  const projectCount = new Set(
    allSessions.map((session) => session.project_path || session.project),
  ).size;
  const analyzedCount = allSessions.filter(
    (session) => session.analysis_state !== "not_analyzed",
  ).length;
  const selected = detail?.session ?? selectedSummary;
  const contextPeak = Math.max(
    0,
    ...(detail?.turns.map((turn) => turn.peak_input_tokens) ?? []),
  );
  const contextWindow = Math.max(
    0,
    ...(detail?.turns.map((turn) => turn.context_window ?? 0) ?? []),
  );

  return (
    <main className="workspace trace-workspace trace-lite">
      <header className="topbar">
        <div>
          <h1>{t("trace.title")}</h1>
          <span className="header-note">{t("trace.subtitle")}</span>
        </div>
        <div className="topbar-actions">
          <span>
            {usingCache ? t("trace.cached") : t("trace.updated")}
            <i />
            {formatSyncTime(snapshot.generated_at)}
          </span>
          <button
            className={`refresh-button${loading ? " spinning" : ""}`}
            onClick={onRefresh}
            disabled={loading}
            aria-label={t("trace.refresh")}
          >
            <RefreshCw aria-hidden="true" />
          </button>
        </div>
      </header>

      <div className="trace-lite-catalog-summary">
        <span>
          <b>{projectCount}</b>{" "}
          {copy(locale, "个项目", projectCount === 1 ? "project" : "projects")}
        </span>
        <span>
          <b>{allSessions.length}</b>{" "}
          {copy(
            locale,
            "个对话",
            allSessions.length === 1 ? "conversation" : "conversations",
          )}
        </span>
        <span>
          <b>{analyzedCount}</b> {copy(locale, "个已分析", "analyzed")}
        </span>
        <small>
          {copy(
            locale,
            "范围：Codex 对话目录；仅已分析项有完整 Timeline",
            "Scope: Codex conversation catalog; full timelines exist only for analyzed items",
          )}
        </small>
        <button
          className="trace-lite-navigator-toggle"
          type="button"
          aria-controls="trace-session-index"
          aria-expanded={!navigatorCollapsed}
          onClick={() => setNavigatorCollapsed((current) => !current)}
        >
          {navigatorCollapsed ? (
            <PanelLeftOpen aria-hidden="true" />
          ) : (
            <PanelLeftClose aria-hidden="true" />
          )}
          <span>
            {navigatorCollapsed
              ? copy(locale, "显示项目", "Show projects")
              : copy(locale, "收起列表", "Hide list")}
          </span>
        </button>
      </div>

      <section
        className={`trace-lite-layout${navigatorCollapsed ? " navigator-collapsed" : ""}`}
      >
        {!navigatorCollapsed && (
          <aside
            id="trace-session-index"
            className="trace-lite-index"
            aria-label={copy(
              locale,
              "项目与对话",
              "Projects and conversations",
            )}
          >
          <header>
            <div>
              <h2>{copy(locale, "项目与对话", "Projects & conversations")}</h2>
              <small>
                {copy(locale, "最近活跃优先", "Most recently active first")}
              </small>
            </div>
            <span>{sessions.length}</span>
          </header>

          <SearchField
            className="trace-lite-search"
            value={query}
            onChange={setQuery}
            placeholder={copy(
              locale,
              "搜索项目或对话",
              "Search projects or conversations",
            )}
            ariaLabel={copy(
              locale,
              "搜索项目或对话",
              "Search projects or conversations",
            )}
            clearLabel={copy(locale, "清除搜索", "Clear search")}
          />

          {batchConfirmation && (
            <div
              className="trace-batch-confirmation"
              role="dialog"
              aria-label={copy(
                locale,
                "确认批量分析",
                "Confirm batch analysis",
              )}
            >
              <strong>
                {copy(
                  locale,
                  `分析 ${batchConfirmation.candidates.length} 个对话？`,
                  `Analyze ${batchConfirmation.candidates.length} conversations?`,
                )}
              </strong>
              <span>{batchConfirmation.project.name}</span>
              <p>
                {copy(
                  locale,
                  "逐个读取本地 Session 并把结构化结果写入 SQLite；不调用 LLM，也不消耗 Codex 额度。可随时停止。",
                  "Reads local sessions one by one and persists structured results to SQLite. It does not call an LLM or consume Codex quota, and can be stopped.",
                )}
              </p>
              <div>
                <button
                  type="button"
                  onClick={() => setBatchConfirmation(null)}
                >
                  {copy(locale, "取消", "Cancel")}
                </button>
                <button
                  type="button"
                  className="primary-action"
                  onClick={() =>
                    void analyzeProject(
                      batchConfirmation.project,
                      batchConfirmation.candidates,
                    )
                  }
                >
                  {copy(locale, "开始分析", "Start analysis")}
                </button>
              </div>
            </div>
          )}

          {batchAnalysis && (
            <div
              className={`trace-batch-status ${batchAnalysis.status}`}
              role="status"
            >
              <div>
                <strong>
                  {batchAnalysis.status === "running"
                    ? copy(locale, "正在分析项目", "Analyzing project")
                    : batchAnalysis.status === "cancelled"
                      ? copy(locale, "分析已停止", "Analysis stopped")
                      : copy(locale, "项目分析完成", "Project analysis complete")}
                </strong>
                <span>
                  {batchAnalysis.projectName} · {batchAnalysis.completed}/
                  {batchAnalysis.total}
                  {batchAnalysis.failed > 0
                    ? copy(
                        locale,
                        ` · ${batchAnalysis.failed} 个失败`,
                        ` · ${batchAnalysis.failed} failed`,
                      )
                    : ""}
                </span>
              </div>
              <i>
                <span
                  style={{
                    width: `${
                      batchAnalysis.total > 0
                        ? (batchAnalysis.completed / batchAnalysis.total) * 100
                        : 0
                    }%`,
                  }}
                />
              </i>
              <div className="trace-batch-actions">
                {batchAnalysis.status !== "running" &&
                  batchAnalysis.failedSessionIds.length > 0 && (
                    <button
                      type="button"
                      onClick={retryFailedProjectSessions}
                    >
                      {copy(
                        locale,
                        `重试 ${batchAnalysis.failedSessionIds.length} 个失败`,
                        `Retry ${batchAnalysis.failedSessionIds.length} failed`,
                      )}
                    </button>
                  )}
                <button
                  type="button"
                  onClick={() => {
                    if (batchAnalysis.status === "running") {
                      cancelBatchAnalysis.current = true;
                    } else {
                      setBatchAnalysis(null);
                      setBatchError(null);
                    }
                  }}
                >
                  {batchAnalysis.status === "running"
                    ? copy(locale, "停止", "Stop")
                    : copy(locale, "关闭", "Dismiss")}
                </button>
              </div>
              {batchError && <small title={batchError}>{batchError}</small>}
            </div>
          )}

          <div className="trace-lite-projects">
            {projectGroups.length === 0 ? (
              <p className="trace-lite-muted">
                {copy(locale, "没有匹配的对话", "No matching conversations")}
              </p>
            ) : (
              projectGroups.map((project) => {
                const collapsed =
                  !query && !expandedProjects.has(project.key);
                const remaining = project.sessions.filter(
                  (session) =>
                    session.analysis_state !== "ready" &&
                    Boolean(session.session_path),
                ).length;
                const isCurrentBatch =
                  batchAnalysis?.projectKey === project.key;
                return (
                  <section className="trace-lite-project" key={project.key}>
                    <div className="trace-lite-project-heading">
                      <button
                        className="trace-lite-project-row"
                        onClick={() =>
                          setExpandedProjects((current) => {
                            const next = new Set(current);
                            if (next.has(project.key)) next.delete(project.key);
                            else next.add(project.key);
                            return next;
                          })
                        }
                        aria-expanded={!collapsed}
                        title={project.path || project.name}
                      >
                        <ChevronRight
                          className={`trace-lite-chevron${collapsed ? "" : " open"}`}
                          aria-hidden="true"
                        />
                        {collapsed ? (
                          <Folder
                            className="trace-lite-folder-icon"
                            aria-hidden="true"
                          />
                        ) : (
                          <FolderOpen
                            className="trace-lite-folder-icon"
                            aria-hidden="true"
                          />
                        )}
                        <span>
                          <strong>{project.name}</strong>
                          <small>
                            {project.sessions.length}{" "}
                            {copy(
                              locale,
                              "个对话",
                              project.sessions.length === 1
                                ? "conversation"
                                : "conversations",
                            )}
                            {project.activeCount > 0 &&
                              copy(
                                locale,
                                ` · ${project.activeCount} 活跃`,
                                ` · ${project.activeCount} active`,
                              )}
                          </small>
                        </span>
                        <em>
                          {project.analyzedCount}/{project.sessions.length}
                        </em>
                      </button>
                      {(remaining > 0 ||
                        (isCurrentBatch &&
                          batchAnalysis?.status === "running")) && (
                        <button
                          className="trace-project-analyze"
                          type="button"
                          disabled={
                            batchAnalysis?.status === "running" &&
                            !isCurrentBatch
                          }
                          onClick={() => requestProjectAnalysis(project)}
                          aria-label={copy(
                            locale,
                            `分析 ${project.name} 的 ${remaining} 个对话`,
                            `Analyze ${remaining} conversations in ${project.name}`,
                          )}
                        >
                          {isCurrentBatch &&
                          batchAnalysis?.status === "running"
                            ? copy(locale, "停止", "Stop")
                            : copy(locale, `分析 ${remaining}`, `Analyze ${remaining}`)}
                        </button>
                      )}
                    </div>

                    {!collapsed && (
                      <div className="trace-lite-conversations">
                        {project.sessions.map((session) => (
                          <button
                            key={session.id}
                            className={
                              session.id === selected?.id ? "selected" : ""
                            }
                            onClick={() => {
                              setSelectedId(session.id);
                              setOpenTurnId(null);
                            }}
                            title={conversationLabel(session, locale)}
                          >
                            <i className={`trace-status-dot ${session.status}`} />
                            <span>
                              <strong>
                                {conversationLabel(session, locale)}
                              </strong>
                              <small>
                                {formatDateTime(session.updated_at, locale)}
                                {session.is_subagent
                                  ? copy(locale, " · 子任务", " · subtask")
                                  : ""}
                              </small>
                            </span>
                            <em>
                              {session.analysis_state === "not_analyzed"
                                ? copy(locale, "分析", "Analyze")
                                : session.analysis_state === "stale"
                                  ? copy(locale, "更新", "Update")
                                  : formatReadableTokens(session.total_tokens)}
                            </em>
                          </button>
                        ))}
                      </div>
                    )}
                  </section>
                );
              })
            )}
          </div>
          </aside>
        )}

        <div className="trace-lite-workbench">
          {!selected ? (
            <div className="trace-lite-empty">
              <MessageSquareText
                className="trace-lite-empty-icon"
                aria-hidden="true"
              />
              <strong>
                {copy(
                  locale,
                  "选择一个对话查看执行详情",
                  "Choose a conversation to view its execution",
                )}
              </strong>
            </div>
          ) : (
            <>
              <header className="trace-lite-session-header">
                <div>
                  <span>
                    {selected.project} ·{" "}
                    <code>{selected.id.slice(0, 12)}</code>
                  </span>
                  <h2>{conversationLabel(selected, locale)}</h2>
                  <p>
                    <i className={`trace-status-dot ${selected.status}`} />
                    {statusLabel(selected.status, t)}
                    {selected.model !== "—" ? ` · ${selected.model}` : ""}
                    {" · "}
                    {formatDateTime(selected.updated_at, locale)}
                  </p>
                  {selected.project_path && (
                    <code className="trace-lite-project-path">
                      {selected.project_path}
                    </code>
                  )}
                </div>
                {detail && !detailLoading && (
                  <div className="trace-lite-session-action">
                    <button
                      type="button"
                      className="secondary-action"
                      onClick={() => void analyzeSelected()}
                      disabled={analyzingId !== null}
                      aria-busy={analyzingId === selected.id}
                    >
                      {analyzingId === selected.id ? (
                        <LoaderCircle className="spinning" aria-hidden="true" />
                      ) : (
                        <RotateCcw aria-hidden="true" />
                      )}
                      {analyzingId === selected.id
                        ? copy(locale, "正在分析…", "Analyzing…")
                        : copy(locale, "重新分析", "Analyze again")}
                    </button>
                  </div>
                )}
              </header>

              {detailLoading && (
                <div className="trace-lite-loading" role="status">
                  <span className="trace-index-pulse" />
                  <div>
                    <strong>
                      {analyzingId === selected.id
                        ? copy(locale, "正在分析 Session…", "Analyzing session…")
                        : copy(locale, "正在读取分析结果…", "Loading analysis…")}
                    </strong>
                    <p>
                      {copy(
                        locale,
                        "读取结构化回合、Token、上下文和工具事件。",
                        "Reading structured turns, tokens, context, and tool events.",
                      )}
                    </p>
                  </div>
                </div>
              )}

              {detailError && (
                <div className="trace-lite-error" role="alert">
                  <strong>
                    {copy(locale, "执行分析失败", "Analysis failed")}
                  </strong>
                  <p>{detailError}</p>
                  <button
                    type="button"
                    className="primary-action"
                    onClick={() => void analyzeSelected()}
                    disabled={analyzingId !== null}
                    aria-busy={analyzingId === selected.id}
                  >
                    {copy(locale, "重试", "Retry")}
                  </button>
                </div>
              )}

              {!detailLoading && !detailError && !detail && (
                <section className="trace-lite-analyze">
                  <h3>
                    {selected.analysis_state === "stale"
                      ? copy(locale, "对话已更新，需要重新分析", "Conversation changed; analyze again")
                      : copy(locale, "分析执行记录", "Analyze execution")}
                  </h3>
                  <p>
                    {copy(
                      locale,
                      "读取这个对话的回合、消息、工具调用与 Token 记录。",
                      "Read this conversation's turns, messages, tool calls, and token records.",
                    )}
                  </p>
                  <dl>
                    <div>
                      <dt>{copy(locale, "项目目录", "Project directory")}</dt>
                      <dd>
                        <code>{selected.project_path || selected.project}</code>
                      </dd>
                    </div>
                    <div>
                      <dt>{copy(locale, "最后更新", "Last updated")}</dt>
                      <dd>{formatDateTime(selected.updated_at, locale)}</dd>
                    </div>
                    <div>
                      <dt>Session ID</dt>
                      <dd>
                        <code>{selected.id}</code>
                      </dd>
                    </div>
                  </dl>
                  <button
                    type="button"
                    className="primary-action"
                    onClick={() => void analyzeSelected()}
                    disabled={analyzingId !== null}
                    aria-busy={analyzingId === selected.id}
                  >
                    {analyzingId === selected.id ? (
                      <LoaderCircle className="spinning" aria-hidden="true" />
                    ) : (
                      <ScanSearch aria-hidden="true" />
                    )}
                    {analyzingId === selected.id
                      ? copy(locale, "正在分析…", "Analyzing…")
                      : copy(locale, "分析这个对话", "Analyze this conversation")}
                  </button>
                </section>
              )}

              {detail && (
                <>
                  <section className="trace-lite-session-summary">
                    <span className="primary">
                      <small>{copy(locale, "总 Token", "Total tokens")}</small>
                      <strong>{formatReadableTokens(selected.total_tokens)}</strong>
                      <em>{formatExactUsd(selected.estimated_cost_usd)}</em>
                    </span>
                    <span>
                      <small>{copy(locale, "回合", "Turns")}</small>
                      <strong>{detail.turns.length}</strong>
                      <em>
                        {selected.tool_calls}{" "}
                        {copy(locale, "次工具调用", "tool calls")}
                      </em>
                    </span>
                    <span>
                      <small>{copy(locale, "输入 / 缓存", "Input / cache")}</small>
                      <strong>
                        {formatReadableTokens(selected.input_tokens)} /{" "}
                        {formatReadableTokens(selected.cached_input_tokens)}
                      </strong>
                      <em>{selected.cache_hit_percent.toFixed(1)}%</em>
                    </span>
                    <span>
                      <small>{copy(locale, "输出 / 推理", "Output / reasoning")}</small>
                      <strong>
                        {formatReadableTokens(selected.output_tokens)} /{" "}
                        {formatReadableTokens(selected.reasoning_output_tokens)}
                      </strong>
                    </span>
                    <span>
                      <small>{copy(locale, "上下文峰值 / 窗口", "Context peak / window")}</small>
                      <strong>
                        {formatReadableTokens(contextPeak)} /{" "}
                        {contextWindow > 0
                          ? formatReadableTokens(contextWindow)
                          : "—"}
                      </strong>
                      <em>
                        {selected.context_compactions}{" "}
                        {copy(locale, "次压缩", "compactions")}
                      </em>
                    </span>
                  </section>

                  <section className="trace-lite-turns">
                    <header>
                      <div>
                        <h3>{copy(locale, "执行记录", "Execution")}</h3>
                        <p>
                          {copy(
                            locale,
                            `${detail.turns.length} 个回合 · ${detail.model_passes} 次模型处理 · ${eventCounts.get("all") ?? 0} 条事件${detail.estimated_reclaimed_tokens > 0 ? ` · 压缩估算减少 ${formatReadableTokens(detail.estimated_reclaimed_tokens)}` : ""}`,
                            `${detail.turns.length} turns · ${detail.model_passes} model passes · ${eventCounts.get("all") ?? 0} events${detail.estimated_reclaimed_tokens > 0 ? ` · ~${formatReadableTokens(detail.estimated_reclaimed_tokens)} reduced by compaction` : ""}`,
                          )}
                        </p>
                      </div>
                    </header>

                    <nav
                      className="trace-event-filters"
                      aria-label={copy(
                        locale,
                        "筛选 Timeline 事件",
                        "Filter timeline events",
                      )}
                    >
                      {eventFilters
                        .filter(
                          (filter) =>
                            filter === "all" || (eventCounts.get(filter) ?? 0) > 0,
                        )
                        .map((filter) => (
                          <button
                            key={filter}
                            className={eventFilter === filter ? "selected" : ""}
                            aria-pressed={eventFilter === filter}
                            onClick={() => setEventFilter(filter)}
                          >
                            <span>{eventFilterLabel(filter, locale)}</span>
                            <b>{eventCounts.get(filter) ?? 0}</b>
                          </button>
                        ))}
                    </nav>

                    {detail.turns.length === 0 ? (
                      <p className="trace-lite-muted">
                        {copy(
                          locale,
                          "这个 Session 没有可读取的回合。",
                          "No readable turns in this session.",
                        )}
                      </p>
                    ) : (
                      <div className="trace-lite-turn-list">
                        {detail.turns.map((turn) => (
                          <TurnTimeline
                            key={turn.id}
                            turn={turn}
                            locale={locale}
                            t={t}
                            eventFilter={eventFilter}
                            open={openTurnId === turn.id}
                            onToggle={() =>
                              setOpenTurnId((current) =>
                                current === turn.id ? null : turn.id,
                              )
                            }
                          />
                        ))}
                      </div>
                    )}
                  </section>
                </>
              )}
            </>
          )}
        </div>
      </section>

      {error && <p className="cost-error">{error}</p>}
    </main>
  );
}
