import { invoke } from "@tauri-apps/api/core";
import { ChevronRight, LoaderCircle } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Locale } from "./i18n";
import type { ExtensionUsageItem, ExtensionUsageSnapshot } from "./types";

type Props = {
  locale: Locale;
  refreshSignal: number;
  totalSessions: number;
  onOpenTrace: () => void;
};

function copy(locale: Locale, zh: string, en: string): string {
  return locale === "zh-CN" ? zh : en;
}

function integer(locale: Locale, value: number): string {
  return new Intl.NumberFormat(locale).format(value);
}

function countLabel(
  locale: Locale,
  value: number,
  zhUnit: string,
  singular: string,
  plural: string,
): string {
  return locale === "zh-CN"
    ? `${integer(locale, value)} ${zhUnit}`
    : `${integer(locale, value)} ${value === 1 ? singular : plural}`;
}

function duration(locale: Locale, value: number): string {
  if (value < 1_000) return `${integer(locale, value)} ms`;
  if (value < 60_000) {
    return `${new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(value / 1_000)} s`;
  }
  const minutes = Math.floor(value / 60_000);
  const seconds = Math.round((value % 60_000) / 1_000);
  return locale === "zh-CN"
    ? `${integer(locale, minutes)} 分 ${seconds} 秒`
    : `${integer(locale, minutes)}m ${seconds}s`;
}

function bytes(locale: Locale, value: number): string {
  if (value < 1_024) return `${integer(locale, value)} B`;
  if (value < 1_048_576) {
    return `${new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(value / 1_024)} KB`;
  }
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(value / 1_048_576)} MB`;
}

function categoryLabel(locale: Locale, category: string): string {
  const labels: Record<string, [string, string]> = {
    mcp: ["MCP", "MCP"],
    skill: ["Skill", "Skill"],
    cli: ["CLI", "CLI"],
    browser: ["浏览器", "Browser"],
    automation: ["自动化", "Automation"],
    agent: ["子 Agent", "Agent"],
    file: ["文件", "File"],
    tool: ["其他工具", "Other tool"],
  };
  const label = labels[category];
  return label ? (locale === "zh-CN" ? label[0] : label[1]) : category;
}

function lastUsed(locale: Locale, value: string | null): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

function itemIdentity(item: ExtensionUsageItem): string {
  if (item.category === "mcp" && item.server) {
    const prefix = `mcp__${item.server}.`;
    const tool = item.name.startsWith(prefix)
      ? item.name.slice(prefix.length)
      : item.name;
    return `${item.server} / ${tool}`;
  }
  return item.name;
}

function localizedWarning(locale: Locale, warning: string): string {
  if (locale === "zh-CN") return warning;
  if (warning.startsWith("尚无已解剖 Session")) {
    return "No inspected sessions yet. Open Execution and inspect a conversation first.";
  }
  const stale = warning.match(/^(\d+) 个已持久化分析/);
  if (stale) {
    const count = Number(stale[1]);
    return `${stale[1]} persisted session ${count === 1 ? "analysis is" : "analyses are"} stale and may undercount current usage.`;
  }
  return warning;
}

const TRACE_SELECTED_SESSION_KEY = "codex-xray.trace-selected-session.v1";
const TRACE_TARGET_SESSION_KEY = "codex-xray.trace-target-session.v1";
const TRACE_TARGET_TURN_KEY = "codex-xray.trace-target-turn.v1";
const TRACE_TARGET_CALL_KEY = "codex-xray.trace-target-call.v1";

export default function ExtensionUsageView({
  locale,
  refreshSignal,
  totalSessions,
  onOpenTrace,
}: Props) {
  const [snapshot, setSnapshot] = useState<ExtensionUsageSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("all");

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    void invoke<ExtensionUsageSnapshot>("get_extension_usage")
      .then((value) => {
        if (!cancelled) setSnapshot(value);
      })
      .catch((cause) => {
        if (!cancelled) setError(String(cause));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [refreshSignal]);

  const visibleItems = useMemo(
    () =>
      snapshot?.items.filter((item) => filter === "all" || item.category === filter) ??
      [],
    [filter, snapshot],
  );

  const openLatestOccurrence = (item: ExtensionUsageItem) => {
    const occurrence = item.occurrences[0];
    if (!occurrence) return;
    try {
      window.localStorage.setItem(
        TRACE_SELECTED_SESSION_KEY,
        occurrence.session_id,
      );
      window.localStorage.setItem(
        TRACE_TARGET_SESSION_KEY,
        occurrence.session_id,
      );
      window.localStorage.setItem(TRACE_TARGET_TURN_KEY, occurrence.turn_id);
      if (occurrence.call_id) {
        window.localStorage.setItem(
          TRACE_TARGET_CALL_KEY,
          occurrence.call_id,
        );
      } else {
        window.localStorage.removeItem(TRACE_TARGET_CALL_KEY);
      }
    } catch {
      // Opening Trace still works if persistence is unavailable.
    }
    onOpenTrace();
  };

  if (!snapshot && loading) {
    return (
      <div className="extension-usage-loading">
        <LoaderCircle className="standard-loader" aria-hidden="true" />
        <strong>{copy(locale, "正在汇总已分析 Session", "Aggregating analyzed sessions")}</strong>
        <small>
          {copy(
            locale,
            "只读取 Codex X-Ray 已持久化的结构化索引，不扫描其他会话。",
            "Reading only the persisted Codex X-Ray index; no other sessions are scanned.",
          )}
        </small>
      </div>
    );
  }

  if (!snapshot) {
    return (
      <div className="extension-usage-empty">
        <strong>{copy(locale, "扩展统计暂不可用", "Extension usage is unavailable")}</strong>
        <p>{error}</p>
      </div>
    );
  }

  const failureRate =
    snapshot.calls > 0 ? (snapshot.failures / snapshot.calls) * 100 : 0;
  const coveragePercent =
    totalSessions > 0
      ? (snapshot.analyzed_sessions / totalSessions) * 100
      : 0;

  return (
    <div className="extension-usage-workbench">
      <section className="extension-scope-note">
        <div>
          <strong>{copy(locale, "真实调用证据", "Observed call evidence")}</strong>
          <p>
            {copy(
              locale,
              `来自 ${snapshot.analyzed_sessions} / ${totalSessions} 个 Session（${coveragePercent.toFixed(2)}%）`,
              `Built from ${snapshot.analyzed_sessions} / ${totalSessions} sessions (${coveragePercent.toFixed(2)}%) analyzed on demand`,
            )}
          </p>
        </div>
        <span>
          {copy(
            locale,
            "Session 没有“单个工具 Token”字段，因此不伪造 Token 归因",
            "Sessions expose no per-tool token field, so token attribution is not invented",
          )}
        </span>
      </section>

      <section className="extension-usage-rail" aria-label={copy(locale, "扩展使用概览", "Extension usage overview")}>
        <div>
          <span>{copy(locale, "分析覆盖", "Analysis coverage")}</span>
          <strong>
            {countLabel(
              locale,
              snapshot.analyzed_sessions,
              "个 Session",
              "Session",
              "Sessions",
            )}
          </strong>
          <small>
            {countLabel(locale, snapshot.projects, "个项目", "project", "projects")} ·{" "}
            {countLabel(locale, snapshot.turns, "个 Turn", "Turn", "Turns")}
          </small>
        </div>
        <div>
          <span>{copy(locale, "工具调用", "Tool calls")}</span>
          <strong>{integer(locale, snapshot.calls)}</strong>
          <small>
            {countLabel(
              locale,
              snapshot.items.length,
              "个调用入口",
              "call identity",
              "call identities",
            )}
          </small>
        </div>
        <div>
          <span>{copy(locale, "可计时调用", "Timed calls")}</span>
          <strong>{duration(locale, snapshot.duration_ms)}</strong>
          <small>
            {integer(locale, snapshot.timed_calls)} / {integer(locale, snapshot.calls)}
          </small>
        </div>
        <div>
          <span>{copy(locale, "失败", "Failures")}</span>
          <strong className={snapshot.failures ? "warning" : "good"}>
            {integer(locale, snapshot.failures)}
          </strong>
          <small>{failureRate.toFixed(1)}%</small>
        </div>
        <div>
          <span>{copy(locale, "结果记录体积", "Result record bytes")}</span>
          <strong>{bytes(locale, snapshot.output_bytes)}</strong>
          <small>{copy(locale, "不是终端实际输出字符数", "Not terminal output characters")}</small>
        </div>
      </section>

      {snapshot.warnings.length > 0 && (
        <section className="extension-usage-warnings">
          {snapshot.warnings.map((warning) => (
            <p key={warning}>{localizedWarning(locale, warning)}</p>
          ))}
        </section>
      )}

      {snapshot.analyzed_sessions === 0 ? (
        <section className="extension-usage-empty">
          <p className="eyebrow">{copy(locale, "还没有证据", "NO EVIDENCE YET")}</p>
          <h2>{copy(locale, "先分析一个对话", "Analyze a conversation first")}</h2>
          <p>
            {copy(
              locale,
              "完成对话分析后，这里会显示 MCP、Skill、CLI 和工具调用。",
              "MCP, Skill, CLI, and tool calls appear here after analysis.",
            )}
          </p>
          <button onClick={onOpenTrace}>
            {copy(locale, "打开执行追踪", "Open execution trace")}
          </button>
        </section>
      ) : (
        <>
          <section className="extension-category-section">
            <header>
              <div>
                <p className="eyebrow">{copy(locale, "调用面", "CALL SURFACE")}</p>
                <h2>{copy(locale, "哪类能力真正被用到", "Which capabilities were actually used")}</h2>
              </div>
              <span>{copy(locale, "按真实结构化调用分类", "Classified from structured calls")}</span>
            </header>
            <div className="extension-category-rows">
              {snapshot.categories.map((category) => (
                <button
                  key={category.category}
                  className={filter === category.category ? "selected" : ""}
                  onClick={() =>
                    setFilter((current) =>
                      current === category.category ? "all" : category.category,
                    )
                  }
                >
                  <span>{categoryLabel(locale, category.category)}</span>
                  <strong>{integer(locale, category.calls)}</strong>
                  <small>
                    {countLabel(
                      locale,
                      category.unique_items,
                      "个入口",
                      "identity",
                      "identities",
                    )}{" "}
                    ·{" "}
                    {duration(locale, category.duration_ms)}
                  </small>
                  <i style={{ width: `${Math.max(3, (category.calls / snapshot.calls) * 100)}%` }} />
                </button>
              ))}
            </div>
          </section>

          <section className="extension-ledger-section">
            <header>
              <div>
                <p className="eyebrow">{copy(locale, "调用账本", "CALL LEDGER")}</p>
                <h2>
                  {filter === "all"
                    ? copy(locale, "全部工具与扩展", "All tools and extensions")
                    : categoryLabel(locale, filter)}
                </h2>
              </div>
              <button
                className={filter === "all" ? "selected" : ""}
                onClick={() => setFilter("all")}
              >
                {copy(locale, "全部", "All")} · {integer(locale, snapshot.items.length)}
              </button>
            </header>
            <div className="extension-ledger">
              <div className="extension-ledger-head">
                <span>{copy(locale, "调用入口", "Identity")}</span>
                <span>{copy(locale, "调用", "Calls")}</span>
                <span>{copy(locale, "覆盖", "Coverage")}</span>
                <span>{copy(locale, "总耗时 / 平均", "Total / average")}</span>
                <span>{copy(locale, "失败 / 重复", "Failed / repeated")}</span>
                <span>{copy(locale, "结果体积", "Result bytes")}</span>
                <span>{copy(locale, "最后使用", "Last used")}</span>
              </div>
              {visibleItems.map((item) => (
                <button
                  type="button"
                  className="extension-ledger-row"
                  key={`${item.category}:${item.server ?? ""}:${item.name}`}
                  disabled={item.occurrences.length === 0}
                  onClick={() => openLatestOccurrence(item)}
                  title={copy(
                    locale,
                    "打开最近一次调用",
                    "Open the latest call",
                  )}
                >
                  <div>
                    <span className={`extension-kind ${item.category}`}>
                      {categoryLabel(locale, item.category)}
                    </span>
                    <strong title={itemIdentity(item)}>{itemIdentity(item)}</strong>
                  </div>
                  <strong>{integer(locale, item.calls)}</strong>
                  <span>
                    {integer(locale, item.projects)}P · {integer(locale, item.sessions)}S ·{" "}
                    {integer(locale, item.turns)}T
                  </span>
                  <span>
                    {item.timed_calls > 0
                      ? `${duration(locale, item.duration_ms)} / ${duration(locale, item.average_duration_ms ?? 0)}`
                      : "—"}
                  </span>
                  <span className={item.failures ? "warning" : ""}>
                    {integer(locale, item.failures)} / {integer(locale, item.repeated_calls)}
                  </span>
                  <span>{bytes(locale, item.output_bytes)}</span>
                  <span className="extension-last-used">
                    {lastUsed(locale, item.last_used_at)}
                    <ChevronRight aria-hidden="true" />
                  </span>
                </button>
              ))}
            </div>
          </section>
        </>
      )}

      {error && <div className="inline-error">{error}</div>}
    </div>
  );
}
