import { invoke } from "@tauri-apps/api/core";
import {
  ChevronRight,
  FileText,
  Network,
  Radio,
  RefreshCw,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { formatDuration } from "./format";
import type { Locale } from "./i18n";
import type {
  ProtocolCaptureSnapshot,
  ProtocolFrame,
  TraceSourcePage,
  TraceSourceRecord,
} from "./types";

type ProtocolInspectorMode = "session" | "app_server" | "provider_wire";

type TraceProtocolInspectorProps = {
  sessionId: string;
  locale: Locale;
};

function copy(locale: Locale, zh: string, en: string): string {
  return locale === "zh-CN" ? zh : en;
}

function formatClock(value: string | null, locale: Locale): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
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

function protocolDirectionLabel(frame: ProtocolFrame, locale: Locale): string {
  const labels: Record<string, [string, string]> = {
    xray_to_server: ["X-Ray → App Server", "X-Ray → App Server"],
    server_to_xray: ["App Server → X-Ray", "App Server → X-Ray"],
    codex_to_bridge: ["Codex → 兼容桥", "Codex → bridge"],
    bridge_to_upstream: ["兼容桥 → Chat 上游", "Bridge → Chat upstream"],
    upstream_to_bridge: ["Chat 上游 → 兼容桥", "Chat upstream → bridge"],
    bridge_to_codex: ["兼容桥 → Codex", "Bridge → Codex"],
  };
  return copy(
    locale,
    ...(labels[frame.direction] ?? [frame.direction, frame.direction]),
  );
}

function SourceRecordRow({
  record,
  locale,
}: {
  record: TraceSourceRecord;
  locale: Locale;
}) {
  return (
    <details className="trace-protocol-record">
      <summary>
        <code>L{record.line}</code>
        <span>
          <strong>{record.payload_type ?? record.record_type}</strong>
          <small>
            {record.payload_type ? `${record.record_type} · ` : ""}
            {record.bytes.toLocaleString(locale)} B
          </small>
        </span>
        {record.call_id && <code>{record.call_id}</code>}
        <time>{formatClock(record.timestamp, locale)}</time>
        <ChevronRight aria-hidden="true" />
      </summary>
      <pre>{record.json}</pre>
      {record.truncated && (
        <p>
          {copy(
            locale,
            "这行过长，预览已截断。",
            "This long line is truncated in the preview.",
          )}
        </p>
      )}
    </details>
  );
}

function ProtocolFrameRow({
  frame,
  locale,
}: {
  frame: ProtocolFrame;
  locale: Locale;
}) {
  return (
    <details className={`trace-protocol-record kind-${frame.kind}`}>
      <summary>
        <code>#{frame.sequence}</code>
        <span>
          <strong>{protocolDirectionLabel(frame, locale)}</strong>
          <small>
            {[
              frame.method,
              frame.kind,
              frame.status ? `HTTP ${frame.status}` : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </small>
        </span>
        {frame.correlation_id && <code>{frame.correlation_id}</code>}
        <time>{formatClock(frame.captured_at, locale)}</time>
        <ChevronRight aria-hidden="true" />
      </summary>
      <div className="trace-protocol-meta">
        <span>{frame.bytes.toLocaleString(locale)} B</span>
        {frame.duration_ms != null && (
          <span>{formatEventDuration(frame.duration_ms, locale)}</span>
        )}
      </div>
      <pre>{frame.body}</pre>
      {frame.truncated && (
        <p>
          {copy(
            locale,
            "正文过长，内存预览已截断。",
            "The in-memory body preview is truncated.",
          )}
        </p>
      )}
    </details>
  );
}

export default function TraceProtocolInspector({
  sessionId,
  locale,
}: TraceProtocolInspectorProps) {
  const [mode, setMode] = useState<ProtocolInspectorMode>("session");
  const [source, setSource] = useState<TraceSourcePage | null>(null);
  const [capture, setCapture] = useState<ProtocolCaptureSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestSequence = useRef(0);
  const sourceRef = useRef<TraceSourcePage | null>(null);

  const load = useCallback(
    async (append = false) => {
      const requestId = ++requestSequence.current;
      setLoading(true);
      setError(null);
      try {
        if (mode === "session") {
          const offset = append ? (sourceRef.current?.next_offset ?? 0) : 0;
          const next = await invoke<TraceSourcePage>("get_trace_source", {
            sessionId,
            offset,
            limit: 60,
          });
          if (requestSequence.current !== requestId) return;
          const merged =
            append && sourceRef.current
              ? {
                  ...next,
                  records: [...sourceRef.current.records, ...next.records],
                }
              : next;
          sourceRef.current = merged;
          setSource(merged);
          setCapture(null);
        } else {
          const next = await invoke<ProtocolCaptureSnapshot>(
            "get_protocol_capture",
            {
              channel: mode,
              afterSequence: null,
            },
          );
          if (requestSequence.current !== requestId) return;
          setCapture(next);
          setSource(null);
        }
      } catch (reason) {
        if (requestSequence.current !== requestId) return;
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        if (requestSequence.current === requestId) setLoading(false);
      }
    },
    [mode, sessionId],
  );

  useEffect(() => {
    sourceRef.current = null;
    setSource(null);
    setCapture(null);
    void load(false);
  }, [load]);

  const revealSource = async () => {
    if (!source) return;
    try {
      await invoke("reveal_local_path", { path: source.session_path });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const boundary =
    mode === "session"
      ? copy(
          locale,
          "Codex 持久化的 Session JSONL，可按行核对 Timeline；它是结构化事件，不是 HTTP 抓包。",
          "Codex's persisted Session JSONL, line-addressable from the Timeline. These are structured events, not an HTTP capture.",
        )
      : mode === "app_server"
        ? copy(
            locale,
            "本次 X-Ray 进程的全局记录：X-Ray 自己与独立 Codex App Server 之间的 JSON 消息；不会截取 Codex 桌面端已有的连接。",
            "Global records for this X-Ray process: JSON messages between X-Ray and its own Codex App Server; this does not intercept the Codex desktop connection.",
          )
        : copy(
            locale,
            "本次 X-Ray 进程的全局记录：仅包含经过 Chat 兼容桥的 Responses 入站、转换后 Chat 请求、上游响应与回传 Responses。原生 Provider 直连不经过这里；凭据字段自动隐藏。",
            "Global records for this X-Ray process: inbound Responses, converted Chat requests, upstream output, and returned Responses that crossed the Chat bridge. Native provider traffic bypasses it; credential fields are redacted.",
          );
  const frames = capture?.frames.slice(-200) ?? [];

  return (
    <section className="trace-protocol-inspector">
      <header>
        <nav aria-label={copy(locale, "协议来源", "Protocol source")}>
          <button
            className={mode === "session" ? "selected" : ""}
            onClick={() => setMode("session")}
          >
            <FileText aria-hidden="true" /> Session
          </button>
          <button
            className={mode === "app_server" ? "selected" : ""}
            onClick={() => setMode("app_server")}
          >
            <Radio aria-hidden="true" /> App Server
          </button>
          <button
            className={mode === "provider_wire" ? "selected" : ""}
            onClick={() => setMode("provider_wire")}
          >
            <Network aria-hidden="true" /> Chat Bridge
          </button>
        </nav>
        <button
          type="button"
          className="trace-protocol-refresh"
          onClick={() => void load(false)}
          disabled={loading}
          aria-label={copy(locale, "刷新原始记录", "Refresh raw records")}
        >
          <RefreshCw className={loading ? "spinning" : ""} aria-hidden="true" />
          {copy(locale, "刷新", "Refresh")}
        </button>
      </header>
      <p className="trace-protocol-boundary">{boundary}</p>
      {error && (
        <p className="trace-lite-error" role="alert">
          {error}
        </p>
      )}
      {mode === "session" && source && (
        <>
          <div className="trace-protocol-source-head">
            <span>
              {copy(locale, "已显示", "Showing")} {source.records.length} /{" "}
              {source.total_lines}
            </span>
            <button type="button" onClick={() => void revealSource()}>
              {copy(locale, "在文件夹中显示", "Reveal in folder")}
            </button>
          </div>
          <div className="trace-protocol-records">
            {source.records.map((record) => (
              <SourceRecordRow
                key={record.line}
                record={record}
                locale={locale}
              />
            ))}
          </div>
          {source.next_offset != null && (
            <button
              className="trace-protocol-load-more"
              onClick={() => void load(true)}
              disabled={loading}
            >
              {copy(locale, "继续读取 60 行", "Load 60 more lines")}
            </button>
          )}
        </>
      )}
      {mode !== "session" && capture && (
        <div className="trace-protocol-records">
          {frames.length > 0 ? (
            frames.map((frame) => (
              <ProtocolFrameRow
                key={frame.sequence}
                frame={frame}
                locale={locale}
              />
            ))
          ) : (
            <p className="trace-lite-muted">
              {mode === "app_server"
                ? copy(
                    locale,
                    "启动后尚未发生新的 App Server 请求。刷新用量或模型接入后再看。",
                    "No new App Server requests since launch. Refresh Usage or Model Access, then return here.",
                  )
                : copy(
                    locale,
                    "启动后尚无请求经过 Chat 兼容桥。启用 Chat 接入并在 Codex 发起一次请求后再看。",
                    "No requests have crossed the Chat bridge since launch. Enable a Chat connection and run one Codex request first.",
                  )}
            </p>
          )}
        </div>
      )}
    </section>
  );
}
