import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ArrowDown,
  ArrowUp,
  Cable,
  ChartNoAxesCombined,
  ChevronRight,
  CircleAlert,
  CircleHelp,
  LoaderCircle,
  Moon,
  RadioTower,
  RefreshCw,
  ScanSearch,
  SlidersHorizontal,
  Sun,
} from "lucide-react";
import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import {
  countdownLabel,
  formatDuration,
  formatExactTokens,
  formatExactUsd,
  formatPlan,
  formatReadableTokens,
  formatReadableUsd,
  formatSyncTime,
  formatWindowLabel,
  localDateKey,
  setFormatLocale,
} from "./format";
import {
  createTranslator,
  readLocale,
  writeLocale,
  type Locale,
  type Translator,
} from "./i18n";
import ProviderView from "./ProviderView";
import RemoteControlView from "./RemoteControlView";
import PricingSettings from "./PricingSettings";
import ProjectUsageView from "./ProjectUsageView";
import TraceView from "./TraceView";
import UpdateControl from "./UpdateControl";
import type {
  CostEstimateSnapshot,
  DailyUsage,
  ModelCostEstimate,
  ProjectTurnUsageDetail,
  ProjectUsageSnapshot,
  RateLimit,
  RateLimitWindow,
  TraceSnapshot,
  UsageSnapshot,
} from "./types";
import BrandMark from "./BrandMark";
import { readTheme, writeTheme, type Theme } from "./theme";

type HistoryRange = "14d" | "30d" | "monthly";
type UsageReport = "overview" | "daily" | "monthly" | "projects" | "models";
type HistoryDisplay = "table" | "trend";
type HistoryOrder = "desc" | "asc";
type UsageRefreshMode = "initial" | "background" | "manual";
type ActiveView = "usage" | "trace" | "remote" | "access" | "console";

const USAGE_BOOT_CACHE_KEY = "codex-xray.usage-snapshot.v1";
const COST_BOOT_CACHE_KEY = "codex-xray.cost-snapshot.v2";
const TRACE_BOOT_CACHE_KEY = "codex-xray.trace-snapshot.v3";
const TRACE_SELECTED_SESSION_KEY = "codex-xray.trace-selected-session.v1";
const TRACE_TARGET_SESSION_KEY = "codex-xray.trace-target-session.v1";
const TRACE_TARGET_TURN_KEY = "codex-xray.trace-target-turn.v1";
const TRACE_TARGET_CALL_KEY = "codex-xray.trace-target-call.v1";

function readBootCache<T>(
  key: string,
  isValid: (value: unknown) => value is T,
): T | null {
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return null;
    const value: unknown = JSON.parse(raw);
    return isValid(value) ? value : null;
  } catch {
    return null;
  }
}

function writeBootCache(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // The Rust cache remains the fallback if WebView storage is unavailable.
  }
}

function isUsageSnapshot(value: unknown): value is UsageSnapshot {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<UsageSnapshot>;
  return (
    typeof candidate.fetched_at === "string" &&
    Array.isArray(candidate.rate_limits) &&
    Array.isArray(candidate.daily_usage) &&
    Array.isArray(candidate.warnings)
  );
}

function isCostEstimateSnapshot(
  value: unknown,
): value is CostEstimateSnapshot {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<CostEstimateSnapshot>;
  return (
    typeof candidate.report_schema_version === "number" &&
    candidate.report_schema_version >= 2 &&
    typeof candidate.generated_at === "string" &&
    typeof candidate.total_cost_usd === "number" &&
    Array.isArray(candidate.daily) &&
    Array.isArray(candidate.models) &&
    Array.isArray(candidate.warnings)
  );
}

function readUsageBootCache(): UsageSnapshot | null {
  return readBootCache(USAGE_BOOT_CACHE_KEY, isUsageSnapshot);
}

function readCostBootCache(): CostEstimateSnapshot | null {
  return readBootCache(COST_BOOT_CACHE_KEY, isCostEstimateSnapshot);
}

function isTraceSnapshot(value: unknown): value is TraceSnapshot {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<TraceSnapshot>;
  return (
    typeof candidate.generated_at === "string" &&
    typeof candidate.totals === "object" &&
    candidate.totals !== null &&
    Array.isArray(candidate.sessions) &&
    Array.isArray(candidate.warnings)
  );
}

function readTraceBootCache(): TraceSnapshot | null {
  return readBootCache(TRACE_BOOT_CACHE_KEY, isTraceSnapshot);
}

type TokenDaySource = "local" | "official" | "live";

type PeriodPoint = {
  key: string;
  label: string;
  shortLabel: string;
  tokens: number;
  source: TokenDaySource | "mixed" | "none";
  isCurrent: boolean;
  incomplete: boolean;
  activeDays: number;
  officialDays: number;
  localDays: number;
  localLiveDays: number;
  costUsd: number | null;
  pricedTokens: number;
  unpricedTokens: number;
};

type PeriodCost = {
  costUsd: number;
  pricedTokens: number;
  unpricedTokens: number;
};

type UsageLedgerRow = {
  key: string;
  label: string;
  shortLabel: string;
  isCurrent: boolean;
  incomplete: boolean;
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteInputTokens: number;
  outputTokens: number;
  totalTokens: number;
  costUsd: number;
  models: ModelCostEstimate[];
  accountPoint: PeriodPoint | null;
};

function InfoTip({
  text,
  align = "center",
}: {
  text: string;
  align?: "start" | "center" | "end";
}) {
  return (
    <span
      className={`info-tip align-${align}`}
      tabIndex={0}
      aria-label={text}
    >
      <CircleHelp aria-hidden="true" />
      <span className="info-tooltip" role="tooltip">
        {text}
      </span>
    </span>
  );
}

function SourceBadge({
  type,
  children,
}: {
  type: "local" | "official" | "neutral";
  children: React.ReactNode;
}) {
  return <span className={`source-badge ${type}`}>{children}</span>;
}

function friendlyLimitName(limit: RateLimit, t: Translator): string {
  if (limit.limit_id === "codex") return t("quota.main");
  if (/spark/i.test(limit.limit_name ?? limit.limit_id)) return t("quota.spark");
  return limit.limit_name ?? limit.limit_id;
}

function formatResetTime(
  value: number | null,
  locale: Locale,
  t: Translator,
): string {
  if (value == null) return t("quota.officialUnavailable");
  return new Intl.DateTimeFormat(locale, {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value * 1_000));
}

function formatEventTime(
  value: string | null | undefined,
  locale: Locale,
): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
}

function QuotaWindowRow({
  window,
  fallbackMinutes,
  now,
  locale,
  t,
}: {
  window: RateLimitWindow;
  fallbackMinutes: number;
  now: number;
  locale: Locale;
  t: Translator;
}) {
  const used = Math.min(100, Math.max(0, window.used_percent));
  const label = formatWindowLabel(window.window_duration_mins ?? fallbackMinutes);

  return (
    <div className="quota-window-row">
      <div className="quota-window-heading">
        <span>{t("quota.used", { label })}</span>
        <strong>{used.toFixed(used % 1 === 0 ? 0 : 1)}%</strong>
      </div>
      <div className="quota-progress" aria-hidden="true">
        <i style={{ width: `${used}%` }} />
      </div>
      <div className="quota-window-detail">
        <span>
          {t("quota.remaining", { value: (100 - used).toFixed(1) })}
        </span>
        <span>
          {countdownLabel(window.resets_at, now)}
          {" · "}
          {formatResetTime(window.resets_at, locale, t)}
        </span>
      </div>
    </div>
  );
}

function Metric({
  label,
  value,
  note,
  secondary,
  exact,
  help,
  accent = false,
}: {
  label: string;
  value: string;
  note?: string;
  secondary?: string;
  exact?: string;
  help: string;
  accent?: boolean;
}) {
  return (
    <div className={accent ? "metric accent" : "metric"}>
      <div className="metric-label">
        <span>{label}</span>
        <InfoTip text={help} />
      </div>
      <strong
        title={exact ?? value}
        aria-label={exact ? `${label}: ${exact}` : undefined}
      >
        {value}
      </strong>
      {note && <small title={note}>{note}</small>}
      {secondary && (
        <small className="metric-secondary" title={secondary}>
          {secondary}
        </small>
      )}
    </div>
  );
}

function mergedDailyUsage(
  official: DailyUsage[],
  localToday: UsageSnapshot["local_today"],
  costEstimate: CostEstimateSnapshot | null,
): Map<string, { tokens: number; source: TokenDaySource }> {
  const merged = new Map(
    official.map((bucket) => [bucket.start_date, bucket.tokens]),
  );
  const result = new Map<
    string,
    { tokens: number; source: TokenDaySource }
  >();
  for (const [date, tokens] of merged) {
    result.set(date, { tokens, source: "official" });
  }
  for (const row of costEstimate?.daily ?? []) {
    if (!result.has(row.date)) {
      result.set(row.date, {
        tokens: row.total_tokens,
        source: "local",
      });
    }
  }
  if (localToday) {
    result.set(localToday.date, {
      tokens: localToday.total_tokens,
      source: "live",
    });
  }
  return result;
}

function mergedDailyCosts(
  estimate: CostEstimateSnapshot | null,
  localToday: UsageSnapshot["local_today"],
): Map<string, PeriodCost> {
  const result = new Map<string, PeriodCost>();
  for (const row of estimate?.daily ?? []) {
    result.set(row.date, {
      costUsd: row.cost_usd,
      pricedTokens: row.priced_tokens,
      unpricedTokens: row.unpriced_tokens,
    });
  }
  if (localToday) {
    result.set(localToday.date, {
      costUsd: localToday.estimated_cost_usd,
      pricedTokens: localToday.priced_tokens,
      unpricedTokens: localToday.unpriced_tokens,
    });
  }
  return result;
}

function buildDailyPoints(
  days: number,
  usage: Map<string, { tokens: number; source: TokenDaySource }>,
  costs: Map<string, PeriodCost>,
  locale: Locale,
): PeriodPoint[] {
  const today = localDateKey();
  const points: PeriodPoint[] = [];

  for (let offset = days - 1; offset >= 0; offset -= 1) {
    const date = new Date();
    date.setHours(12, 0, 0, 0);
    date.setDate(date.getDate() - offset);
    const key = localDateKey(date);
    const item = usage.get(key);
    const cost = costs.get(key);
    points.push({
      key,
      label: new Intl.DateTimeFormat(locale, {
        month: "long",
        day: "numeric",
      }).format(date),
      shortLabel: new Intl.DateTimeFormat(locale, {
        month: "numeric",
        day: "numeric",
      }).format(date),
      tokens: item?.tokens ?? 0,
      source: item?.source ?? "none",
      isCurrent: key === today,
      incomplete: false,
      activeDays: item && item.tokens > 0 ? 1 : 0,
      officialDays: item?.source === "official" ? 1 : 0,
      localDays:
        item?.source === "local" || item?.source === "live" ? 1 : 0,
      localLiveDays: item?.source === "live" ? 1 : 0,
      costUsd: cost?.costUsd ?? null,
      pricedTokens: cost?.pricedTokens ?? 0,
      unpricedTokens: cost?.unpricedTokens ?? 0,
    });
  }
  return points;
}

function buildMonthlyPoints(
  usage: Map<string, { tokens: number; source: TokenDaySource }>,
  costs: Map<string, PeriodCost>,
  locale: Locale,
  t: Translator,
): PeriodPoint[] {
  const dates = [...new Set([...usage.keys(), ...costs.keys()])].sort();
  if (dates.length === 0) return [];
  const firstDate = dates[0];
  const currentMonth = localDateKey().slice(0, 7);
  const firstMonth = firstDate.slice(0, 7);
  const groups = new Map<
    string,
    {
      tokens: number;
      activeDays: number;
      officialDays: number;
      localDays: number;
      localLiveDays: number;
      costUsd: number;
      pricedTokens: number;
      unpricedTokens: number;
      hasCost: boolean;
    }
  >();

  for (const [date, item] of usage) {
    const month = date.slice(0, 7);
    const group = groups.get(month) ?? {
      tokens: 0,
      activeDays: 0,
      officialDays: 0,
      localDays: 0,
      localLiveDays: 0,
      costUsd: 0,
      pricedTokens: 0,
      unpricedTokens: 0,
      hasCost: false,
    };
    group.tokens += item.tokens;
    if (item.tokens > 0) group.activeDays += 1;
    if (item.source === "official") group.officialDays += 1;
    if (item.source === "local" || item.source === "live") {
      group.localDays += 1;
    }
    if (item.source === "live") group.localLiveDays += 1;
    groups.set(month, group);
  }
  for (const [date, cost] of costs) {
    const month = date.slice(0, 7);
    const group = groups.get(month) ?? {
      tokens: 0,
      activeDays: 0,
      officialDays: 0,
      localDays: 0,
      localLiveDays: 0,
      costUsd: 0,
      pricedTokens: 0,
      unpricedTokens: 0,
      hasCost: false,
    };
    group.costUsd += cost.costUsd;
    group.pricedTokens += cost.pricedTokens;
    group.unpricedTokens += cost.unpricedTokens;
    group.hasCost = true;
    groups.set(month, group);
  }

  const [startYear, startMonth] = firstMonth.split("-").map(Number);
  const [endYear, endMonth] = currentMonth.split("-").map(Number);
  const cursor = new Date(startYear, startMonth - 1, 1, 12);
  const end = new Date(endYear, endMonth - 1, 1, 12);
  const points: PeriodPoint[] = [];
  while (cursor <= end) {
    const key = `${cursor.getFullYear()}-${String(cursor.getMonth() + 1).padStart(2, "0")}`;
    const group = groups.get(key);
    const isCurrent = key === currentMonth;
    const firstMonthPartial = key === firstMonth && !firstDate.endsWith("-01");
    points.push({
      key,
      label: t("history.monthLabel", {
        year: cursor.getFullYear(),
        month: cursor.getMonth() + 1,
      }),
      shortLabel: t("history.shortMonth", {
        month: cursor.getMonth() + 1,
      }),
      tokens: group?.tokens ?? 0,
      source:
        (group?.officialDays ?? 0) > 0 && (group?.localDays ?? 0) > 0
          ? "mixed"
          : (group?.localDays ?? 0) > 0
            ? "local"
            : (group?.officialDays ?? 0) > 0
              ? "official"
              : "none",
      isCurrent,
      incomplete: firstMonthPartial || isCurrent,
      activeDays: group?.activeDays ?? 0,
      officialDays: group?.officialDays ?? 0,
      localDays: group?.localDays ?? 0,
      localLiveDays: group?.localLiveDays ?? 0,
      costUsd: group?.hasCost ? group.costUsd : null,
      pricedTokens: group?.pricedTokens ?? 0,
      unpricedTokens: group?.unpricedTokens ?? 0,
    });
    cursor.setMonth(cursor.getMonth() + 1);
  }
  return points;
}

function buildPeriodPoints(
  range: HistoryRange,
  official: DailyUsage[],
  localToday: UsageSnapshot["local_today"],
  costEstimate: CostEstimateSnapshot | null,
  locale: Locale,
  t: Translator,
): PeriodPoint[] {
  const usage = mergedDailyUsage(official, localToday, costEstimate);
  const costs = mergedDailyCosts(costEstimate, localToday);
  if (range === "monthly") return buildMonthlyPoints(usage, costs, locale, t);
  return buildDailyPoints(range === "14d" ? 14 : 30, usage, costs, locale);
}

type ActivityDay = {
  key: string;
  date: Date;
  tokens: number;
  costUsd: number | null;
  source: TokenDaySource | "none";
  future: boolean;
  level: number;
};

function TokenActivityHeatmap({
  official,
  localToday,
  costEstimate,
  locale,
}: {
  official: DailyUsage[];
  localToday: UsageSnapshot["local_today"];
  costEstimate: CostEstimateSnapshot | null;
  locale: Locale;
}) {
  const usage = useMemo(
    () => mergedDailyUsage(official, localToday, costEstimate),
    [costEstimate, localToday, official],
  );
  const costs = useMemo(
    () => mergedDailyCosts(costEstimate, localToday),
    [costEstimate, localToday],
  );
  const days = useMemo(() => {
    const today = new Date();
    today.setHours(12, 0, 0, 0);
    const start = new Date(today);
    start.setDate(today.getDate() - today.getDay() - 52 * 7);
    const values: Omit<ActivityDay, "level">[] = [];
    for (let offset = 0; offset < 53 * 7; offset += 1) {
      const date = new Date(start);
      date.setDate(start.getDate() + offset);
      const key = localDateKey(date);
      const item = usage.get(key);
      values.push({
        key,
        date,
        tokens: item?.tokens ?? 0,
        costUsd: costs.get(key)?.costUsd ?? null,
        source: item?.source ?? "none",
        future: date > today,
      });
    }
    const positive = values
      .map((day) => day.tokens)
      .filter((tokens) => tokens > 0)
      .sort((left, right) => left - right);
    const thresholdAt = (percentile: number) =>
      positive[Math.max(0, Math.ceil(positive.length * percentile) - 1)] ?? 0;
    const thresholds = [
      thresholdAt(0.25),
      thresholdAt(0.55),
      thresholdAt(0.82),
    ];
    return values.map<ActivityDay>((day) => {
      if (day.future || day.tokens <= 0 || positive.length === 0) {
        return { ...day, level: 0 };
      }
      return {
        ...day,
        level:
          day.tokens <= thresholds[0]
            ? 1
            : day.tokens <= thresholds[1]
              ? 2
              : day.tokens <= thresholds[2]
                ? 3
                : 4,
      };
    });
  }, [costs, usage]);
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const focused =
    days.find((day) => day.key === focusedKey) ??
    [...days].reverse().find((day) => !day.future && day.tokens > 0) ??
    null;
  const focusedIndex = focused
    ? days.findIndex((day) => day.key === focused.key)
    : -1;
  const monthLabels = useMemo(() => {
    const formatter = new Intl.DateTimeFormat(locale, { month: "short" });
    const labels: string[] = [];
    let previous = "";
    for (const day of days) {
      const monthKey = `${day.date.getFullYear()}-${day.date.getMonth()}`;
      if (monthKey !== previous) {
        labels.push(formatter.format(day.date));
        previous = monthKey;
      }
    }
    return labels;
  }, [days, locale]);
  const sourceLabel = (source: ActivityDay["source"]) => {
    if (source === "official") {
      return locale === "zh-CN" ? "官方账户日统计" : "Official account daily";
    }
    if (source === "live") {
      return locale === "zh-CN" ? "本地今日实时" : "Live local today";
    }
    if (source === "local") {
      return locale === "zh-CN" ? "本地 Session" : "Local session";
    }
    return locale === "zh-CN" ? "无活动" : "No activity";
  };
  const dateFormatter = new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });

  return (
    <div className="activity-heatmap">
      <div className="activity-heatmap-heading">
        <div>
          <strong>
            {locale === "zh-CN" ? "近一年 Token 活动" : "Token activity · last year"}
          </strong>
          <span>
            {locale === "zh-CN" ? "按自然日统计" : "Grouped by calendar day"}
          </span>
        </div>
        <div className="activity-readout" aria-live="polite">
          {focused ? (
            <>
              <strong>
                {focused.tokens > 0
                  ? `${formatReadableTokens(focused.tokens)} Token${
                      focused.costUsd == null
                        ? ""
                        : ` · ≈ ${formatReadableUsd(focused.costUsd)}`
                    }`
                  : locale === "zh-CN"
                    ? "无活动"
                    : "No activity"}
              </strong>
              <small>
                {focused.tokens > 0
                  ? `${formatExactTokens(focused.tokens)} Token · `
                  : ""}
                {dateFormatter.format(focused.date)} ·{" "}
                {sourceLabel(focused.source)}
              </small>
            </>
          ) : (
            <small>
              {locale === "zh-CN"
                ? "悬浮日期查看精确数据"
                : "Hover a day for exact data"}
            </small>
          )}
        </div>
      </div>
      <div
        className="activity-grid"
        role="group"
        tabIndex={0}
        aria-label={
          locale === "zh-CN"
            ? "近一年每日 Token 活动热力图；使用方向键逐日查看"
            : "Daily token activity heatmap for the last year; use arrow keys to inspect days"
        }
        onKeyDown={(event) => {
          const step =
            event.key === "ArrowUp"
              ? -1
              : event.key === "ArrowDown"
                ? 1
                : event.key === "ArrowLeft"
                  ? -7
                  : event.key === "ArrowRight"
                    ? 7
                    : 0;
          if (step === 0) return;
          event.preventDefault();
          const nextIndex = Math.max(
            0,
            Math.min(days.length - 1, Math.max(0, focusedIndex) + step),
          );
          if (!days[nextIndex].future) setFocusedKey(days[nextIndex].key);
        }}
      >
        {days.map((day) => {
          const label = `${dateFormatter.format(day.date)} · ${
            day.tokens > 0
              ? `${formatExactTokens(day.tokens)} Token`
              : sourceLabel("none")
          }${
            day.costUsd == null
              ? ""
              : ` · ≈ ${formatExactUsd(day.costUsd)}`
          } · ${sourceLabel(day.source)}`;
          return (
            <span
              key={day.key}
              className={`activity-cell level-${day.level}${
                day.future ? " future" : ""
              }${focused?.key === day.key ? " focused" : ""}`}
              title={label}
              aria-hidden="true"
              onMouseEnter={() => setFocusedKey(day.key)}
            />
          );
        })}
      </div>
      <div className="activity-months" aria-hidden="true">
        {monthLabels.map((month, index) => (
          <span key={`${month}-${index}`}>{month}</span>
        ))}
      </div>
      <div className="activity-legend">
        <div aria-hidden="true">
          <small>{locale === "zh-CN" ? "少" : "Less"}</small>
          {[0, 1, 2, 3, 4].map((level) => (
            <i key={level} className={`level-${level}`} />
          ))}
          <small>{locale === "zh-CN" ? "多" : "More"}</small>
        </div>
      </div>
    </div>
  );
}

function mergeModelBreakdown(
  rows: Iterable<ModelCostEstimate>,
): ModelCostEstimate[] {
  const models = new Map<string, ModelCostEstimate>();
  for (const row of rows) {
    const current = models.get(row.model);
    if (!current) {
      models.set(row.model, { ...row });
      continue;
    }
    current.input_tokens += row.input_tokens;
    current.cached_input_tokens += row.cached_input_tokens;
    current.cache_write_input_tokens += row.cache_write_input_tokens;
    current.output_tokens += row.output_tokens;
    current.total_tokens += row.total_tokens;
    current.cost_usd += row.cost_usd;
    current.cache_savings_usd += row.cache_savings_usd;
    current.priced = current.priced && row.priced;
  }
  return [...models.values()].sort(
    (left, right) =>
      right.cost_usd - left.cost_usd ||
      right.total_tokens - left.total_tokens ||
      left.model.localeCompare(right.model),
  );
}

function buildUsageLedgerRows(
  estimate: CostEstimateSnapshot | null,
  accountPoints: PeriodPoint[],
  range: HistoryRange,
  locale: Locale,
  t: Translator,
): UsageLedgerRow[] {
  if (!estimate) return [];
  const accountByKey = new Map(
    accountPoints.map((point) => [point.key, point]),
  );
  if (range !== "monthly") {
    const localByDate = new Map(
      estimate.daily.map((row) => [row.date, row]),
    );
    return accountPoints.flatMap((point) => {
      const local = localByDate.get(point.key);
      if (!local) return [];
      return [
        {
          key: point.key,
          label: point.label,
          shortLabel: point.shortLabel,
          isCurrent: point.isCurrent,
          incomplete: false,
          inputTokens: local.input_tokens,
          cachedInputTokens: local.cached_input_tokens,
          cacheWriteInputTokens: local.cache_write_input_tokens,
          outputTokens: local.output_tokens,
          totalTokens: local.total_tokens,
          costUsd: local.cost_usd,
          models: local.models,
          accountPoint: point,
        },
      ];
    });
  }

  const monthly = new Map<
    string,
    {
      inputTokens: number;
      cachedInputTokens: number;
      cacheWriteInputTokens: number;
      outputTokens: number;
      totalTokens: number;
      costUsd: number;
      models: ModelCostEstimate[];
    }
  >();
  for (const day of estimate.daily) {
    const key = day.date.slice(0, 7);
    const row = monthly.get(key) ?? {
      inputTokens: 0,
      cachedInputTokens: 0,
      cacheWriteInputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
      costUsd: 0,
      models: [],
    };
    row.inputTokens += day.input_tokens;
    row.cachedInputTokens += day.cached_input_tokens;
    row.cacheWriteInputTokens += day.cache_write_input_tokens;
    row.outputTokens += day.output_tokens;
    row.totalTokens += day.total_tokens;
    row.costUsd += day.cost_usd;
    row.models.push(...day.models);
    monthly.set(key, row);
  }

  const currentMonth = localDateKey().slice(0, 7);
  return [...monthly.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, row]) => {
      const [year, month] = key.split("-").map(Number);
      const accountPoint = accountByKey.get(key) ?? null;
      return {
        key,
        label:
          accountPoint?.label ??
          t("history.monthLabel", {
            year,
            month,
          }),
        shortLabel:
          accountPoint?.shortLabel ??
          new Intl.DateTimeFormat(locale, {
            year: "2-digit",
            month: "numeric",
          }).format(new Date(year, month - 1, 1, 12)),
        isCurrent: key === currentMonth,
        incomplete: accountPoint?.incomplete ?? key === currentMonth,
        inputTokens: row.inputTokens,
        cachedInputTokens: row.cachedInputTokens,
        cacheWriteInputTokens: row.cacheWriteInputTokens,
        outputTokens: row.outputTokens,
        totalTokens: row.totalTokens,
        costUsd: row.costUsd,
        models: mergeModelBreakdown(row.models),
        accountPoint,
      };
    });
}

function summarizeLedger(rows: UsageLedgerRow[]) {
  const summary = rows.reduce(
    (total, row) => {
      total.inputTokens += row.inputTokens;
      total.cachedInputTokens += row.cachedInputTokens;
      total.cacheWriteInputTokens += row.cacheWriteInputTokens;
      total.outputTokens += row.outputTokens;
      total.totalTokens += row.totalTokens;
      total.costUsd += row.costUsd;
      total.accountTokens += row.accountPoint?.tokens ?? 0;
      return total;
    },
    {
      inputTokens: 0,
      cachedInputTokens: 0,
      cacheWriteInputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
      costUsd: 0,
      accountTokens: 0,
    },
  );
  return {
    ...summary,
    freshInputTokens: Math.max(
      summary.inputTokens - summary.cachedInputTokens,
      0,
    ),
    cacheHitPercent:
      summary.inputTokens === 0
        ? 0
        : (summary.cachedInputTokens / summary.inputTokens) * 100,
    accountDelta: summary.totalTokens - summary.accountTokens,
  };
}

function ledgerChartPoints(rows: UsageLedgerRow[]): PeriodPoint[] {
  return rows.map((row) => ({
    key: row.key,
    label: row.label,
    shortLabel: row.shortLabel,
    tokens: row.totalTokens,
    source: row.isCurrent ? "live" : "local",
    isCurrent: row.isCurrent,
    incomplete: row.incomplete,
    activeDays: row.totalTokens > 0 ? 1 : 0,
    officialDays: 0,
    localDays: 1,
    localLiveDays: row.isCurrent ? 1 : 0,
    costUsd: row.costUsd,
    pricedTokens: row.totalTokens,
    unpricedTokens: 0,
  }));
}

type ProvenancePresentation = {
  label: string;
  detail: string;
  badge: "local" | "official" | "neutral";
};

function tokenSourcePresentation(
  point: PeriodPoint,
  range: HistoryRange,
  locale: Locale,
): ProvenancePresentation {
  const zh = locale === "zh-CN";
  if (point.source === "none") {
    return {
      label: zh ? "无 Token 数据" : "No token data",
      detail: zh ? "该周期没有返回可用值" : "No usable value was returned",
      badge: "neutral",
    };
  }
  if (range !== "monthly") {
    if (point.source === "official") {
      return {
        label: zh ? "官方账户日统计" : "Official daily usage",
        detail: "account/usage/read",
        badge: "official",
      };
    }
    if (point.source === "live") {
      return {
        label: zh ? "本地实时值" : "Local live value",
        detail: zh
          ? "覆盖官方今日桶，不与它相加"
          : "Replaces today's official bucket; never added to it",
        badge: "local",
      };
    }
    return {
      label: zh ? "本地 Session 补缺" : "Local session fallback",
      detail: zh
        ? "仅在官方缺少该日时采用"
        : "Used only when the official day is missing",
      badge: "local",
    };
  }

  const gapDays = Math.max(point.localDays - point.localLiveDays, 0);
  const details = [
    point.officialDays > 0
      ? zh
        ? `官方 ${point.officialDays} 天`
        : `${point.officialDays} official days`
      : "",
    gapDays > 0
      ? zh
        ? `本地补缺 ${gapDays} 天`
        : `${gapDays} local fallback days`
      : "",
    point.localLiveDays > 0
      ? zh
        ? `本地实时 ${point.localLiveDays} 天`
        : `${point.localLiveDays} local live days`
      : "",
  ].filter(Boolean);
  if (point.source === "official") {
    return {
      label: zh ? "官方账户日统计" : "Official daily usage",
      detail: details.join(" · "),
      badge: "official",
    };
  }
  if (point.source === "local" || point.source === "live") {
    return {
      label: zh ? "本地 Session" : "Local sessions",
      detail: details.join(" · "),
      badge: "local",
    };
  }
  return {
    label:
      gapDays > 0 && point.localLiveDays > 0
        ? zh
          ? "官方 + 本地逐日合并"
          : "Official + local daily merge"
        : gapDays > 0
        ? zh
          ? "官方日统计 + 本地补缺"
          : "Official + local fallback"
        : zh
          ? "官方日统计 + 本地实时"
          : "Official + local live",
    detail: details.join(" · "),
    badge: "local",
  };
}

function UsageChart({
  points,
  range,
  onInspect,
  locale,
  t,
  sourceMode = "account",
}: {
  points: PeriodPoint[];
  range: HistoryRange;
  onInspect?: (key: string) => void;
  locale: Locale;
  t: Translator;
  sourceMode?: "account" | "local";
}) {
  const max = Math.max(...points.map((item) => item.tokens), 1);

  return (
    <div
      className={`daily-chart ${range === "monthly" ? "monthly" : ""}`}
      aria-label={t("history.tokenTrend")}
    >
      {points.map((item, index) => {
        const tokenSource =
          sourceMode === "local"
            ? {
                label:
                  locale === "zh-CN"
                    ? "本地 Session 明细"
                    : "Local session detail",
                detail:
                  locale === "zh-CN"
                    ? "未缓存输入 + 缓存读取 + 输出"
                    : "Fresh input + cache read + output",
                badge: "local" as const,
              }
            : tokenSourcePresentation(item, range, locale);
        const height =
          item.tokens === 0 ? 2 : Math.max(5, (item.tokens / max) * 100);
        const showLabel =
          range !== "30d" ||
          index === 0 ||
          index === points.length - 1 ||
          index % 5 === 0;
        return (
          <button
            className={`chart-column${item.isCurrent ? " today" : ""}${item.incomplete ? " incomplete" : ""}`}
            key={item.key}
            aria-label={`${item.label}, ${formatExactTokens(item.tokens)} ${t("common.tokens")}${item.costUsd == null ? "" : `, ${t("history.cost", { value: formatExactUsd(item.costUsd) })}`}, ${tokenSource.label}`}
            onMouseEnter={() => onInspect?.(item.key)}
            onFocus={() => onInspect?.(item.key)}
            onClick={() => onInspect?.(item.key)}
          >
            <span className="chart-tooltip" role="tooltip">
              <small>
                {item.label}
                {item.isCurrent
                  ? range === "monthly"
                    ? ` · ${t("common.inProgress")}`
                    : ` · ${t("common.today")}`
                  : item.incomplete
                    ? ` · ${t("common.incomplete")}`
                    : ""}
              </small>
              <strong>{formatReadableTokens(item.tokens)}</strong>
              <em>{formatExactTokens(item.tokens)} Token</em>
              {item.costUsd != null && (
                <b>{t("history.cost", { value: formatExactUsd(item.costUsd) })}</b>
              )}
              <i>
                {tokenSource.label}
                {tokenSource.detail && ` · ${tokenSource.detail}`}
                {range === "monthly" &&
                  ` · ${t("history.activeDays", { count: item.activeDays })}`}
              </i>
            </span>
            <span className="bar-space">
              <span className="bar" style={{ height: `${height}%` }} />
            </span>
            <span className="chart-date">
              {showLabel ? item.shortLabel : ""}
              {item.isCurrent && (
                <i>
                  {range === "monthly"
                    ? t("common.currentMonth")
                    : t("common.today")}
                </i>
              )}
            </span>
          </button>
        );
      })}
    </div>
  );
}

function UsageLedgerTable({
  rows,
  locale,
  range,
  limit,
  order,
  onToggleOrder,
  expandedKeys,
  onToggleExpanded,
}: {
  rows: UsageLedgerRow[];
  locale: Locale;
  range: HistoryRange;
  limit?: number;
  order: HistoryOrder;
  onToggleOrder?: () => void;
  expandedKeys: string[];
  onToggleExpanded?: (key: string) => void;
}) {
  const orderedRows =
    order === "desc" ? [...rows].reverse() : [...rows];
  const visibleRows = orderedRows.slice(0, limit);
  const totals = summarizeLedger(visibleRows);
  const showCacheWrite = visibleRows.some(
    (row) => row.cacheWriteInputTokens > 0,
  );
  const columnCount = showCacheWrite ? 8 : 7;
  const zh = locale === "zh-CN";

  return (
    <div className="usage-ledger-wrap">
      <table className="usage-ledger-table">
        <caption>
          {zh
            ? "按周期统计的本地 Session Token、模型与 API 等价成本"
            : "Local session tokens, models, and API-equivalent cost by period"}
        </caption>
        <thead>
          <tr>
            <th
              aria-sort={
                order === "desc" ? "descending" : "ascending"
              }
            >
              {onToggleOrder ? (
                <button
                  className="ledger-sort"
                  onClick={onToggleOrder}
                  aria-label={
                    zh
                      ? `周期按${order === "desc" ? "从新到旧" : "从旧到新"}排列，点击切换`
                      : `Periods sorted ${order === "desc" ? "newest first" : "oldest first"}; activate to reverse`
                  }
                >
                  {zh ? "周期" : "Period"}
                  {order === "desc" ? (
                    <ArrowDown aria-hidden="true" />
                  ) : (
                    <ArrowUp aria-hidden="true" />
                  )}
                </button>
              ) : zh ? (
                "周期"
              ) : (
                "Period"
              )}
            </th>
            <th className="ledger-model-column">
              {zh ? "模型" : "Models"}
            </th>
            <th>{zh ? "未缓存输入" : "Fresh input"}</th>
            <th>{zh ? "缓存读取" : "Cache read"}</th>
            {showCacheWrite && (
              <th>{zh ? "缓存写入" : "Cache write"}</th>
            )}
            <th>{zh ? "输出" : "Output"}</th>
            <th>{zh ? "本地总 Token" : "Local total"}</th>
            <th>{zh ? "API 等价成本" : "API-equivalent"}</th>
          </tr>
        </thead>
        <tbody>
          {visibleRows.length === 0 && (
            <tr className="ledger-empty-row">
              <td colSpan={columnCount}>
                <strong>
                  {zh ? "当前范围没有本地 Token 明细" : "No local token detail in this range"}
                </strong>
                <small>
                  {zh
                    ? "可以扩大日期范围，或等待本地索引完成"
                    : "Expand the date range or wait for the local index to finish"}
                </small>
              </td>
            </tr>
          )}
          {visibleRows.map((row) => {
            const expanded = expandedKeys.includes(row.key);
            const accountSource = row.accountPoint
              ? tokenSourcePresentation(row.accountPoint, range, locale)
              : null;
            const freshInput = Math.max(
              row.inputTokens - row.cachedInputTokens,
              0,
            );
            return (
              <Fragment key={row.key}>
                <tr
                  className={expanded ? "expanded" : ""}
                >
                  <td>
                    {onToggleExpanded && row.models.length > 0 ? (
                    <button
                      className="ledger-period-toggle"
                      onClick={() => onToggleExpanded(row.key)}
                      aria-expanded={expanded}
                      aria-label={
                        zh
                          ? `${expanded ? "收起" : "展开"} ${row.label} 的模型明细`
                          : `${expanded ? "Collapse" : "Expand"} model details for ${row.label}`
                      }
                    >
                      <ChevronRight
                        className="ledger-chevron"
                        aria-hidden="true"
                      />
                      <span>
                        <strong>{row.label}</strong>
                        {row.isCurrent && (
                          <small>{zh ? "进行中" : "In progress"}</small>
                        )}
                        <small className="ledger-period-models">
                          {row.models
                            .slice(0, 2)
                            .map((model) => model.model)
                            .join(" · ")}
                          {row.models.length > 2
                            ? ` +${row.models.length - 2}`
                            : ""}
                        </small>
                      </span>
                    </button>
                  ) : (
                    <div className="ledger-static-period">
                      <strong>{row.label}</strong>
                      {row.isCurrent && (
                        <small>
                          {zh ? "进行中" : "In progress"}
                        </small>
                      )}
                    </div>
                  )}
                </td>
                <td className="ledger-model-column">
                  <div className="ledger-model-list">
                    {row.models.slice(0, 2).map((model) => (
                      <code key={model.model}>{model.model}</code>
                    ))}
                    {row.models.length > 2 && (
                      <small>+{row.models.length - 2}</small>
                    )}
                  </div>
                </td>
                <td>{formatExactTokens(freshInput)}</td>
                <td>{formatExactTokens(row.cachedInputTokens)}</td>
                {showCacheWrite && (
                  <td>{formatExactTokens(row.cacheWriteInputTokens)}</td>
                )}
                <td>{formatExactTokens(row.outputTokens)}</td>
                <td>
                  <strong>{formatExactTokens(row.totalTokens)}</strong>
                  {row.accountPoint && accountSource && (
                    <small
                      className="ledger-account-compare"
                      title={`${accountSource.label} · ${accountSource.detail}`}
                    >
                      {zh ? "账户视图" : "Account view"}{" "}
                      {formatReadableTokens(row.accountPoint.tokens)}
                    </small>
                  )}
                </td>
                <td>
                  <strong>{formatExactUsd(row.costUsd)}</strong>
                  <small>{zh ? "估算" : "Estimate"}</small>
                </td>
              </tr>
              {expanded && (
                <tr className="ledger-breakdown-row">
                  <td colSpan={columnCount}>
                    <div className="ledger-breakdown">
                      <div className="ledger-breakdown-heading">
                        <strong>
                          {zh ? "模型明细" : "Model breakdown"}
                        </strong>
                        <span>
                          {zh
                            ? "输入列不含缓存读取；推理 Token 已包含在输出中"
                            : "Fresh input excludes cache reads; reasoning is included in output"}
                        </span>
                      </div>
                      <table>
                        <thead>
                          <tr>
                            <th>{zh ? "模型" : "Model"}</th>
                            <th>{zh ? "未缓存输入" : "Fresh input"}</th>
                            <th>{zh ? "缓存读取" : "Cache read"}</th>
                            {showCacheWrite && (
                              <th>{zh ? "缓存写入" : "Cache write"}</th>
                            )}
                            <th>{zh ? "输出" : "Output"}</th>
                            <th>{zh ? "总 Token" : "Total tokens"}</th>
                            <th>{zh ? "成本" : "Cost"}</th>
                          </tr>
                        </thead>
                        <tbody>
                          {row.models.map((model) => (
                            <tr key={model.model}>
                              <td>
                                <code>{model.model}</code>
                                {!model.priced && (
                                  <small>
                                    {zh ? "未定价" : "Unpriced"}
                                  </small>
                                )}
                              </td>
                              <td>
                                {formatExactTokens(
                                  Math.max(
                                    model.input_tokens -
                                      model.cached_input_tokens,
                                    0,
                                  ),
                                )}
                              </td>
                              <td>
                                {formatExactTokens(
                                  model.cached_input_tokens,
                                )}
                              </td>
                              {showCacheWrite && (
                                <td>
                                  {formatExactTokens(
                                    model.cache_write_input_tokens,
                                  )}
                                </td>
                              )}
                              <td>
                                {formatExactTokens(model.output_tokens)}
                              </td>
                              <td>
                                <strong>
                                  {formatExactTokens(model.total_tokens)}
                                </strong>
                              </td>
                              <td>
                                {model.priced
                                  ? formatExactUsd(model.cost_usd)
                                  : "—"}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  </td>
                </tr>
              )}
            </Fragment>
            );
          })}
        </tbody>
        {visibleRows.length > 0 && (
        <tfoot>
          <tr>
            <td>
              <strong>{zh ? "总计" : "Total"}</strong>
              <small>
                {visibleRows.length}{" "}
                {range === "monthly"
                  ? zh
                    ? "个月"
                    : "months"
                  : zh
                    ? "天"
                    : "days"}
              </small>
            </td>
            <td className="ledger-model-column">
              {mergeModelBreakdown(
                visibleRows.flatMap((row) => row.models),
              ).length}{" "}
              {zh ? "个模型" : "models"}
            </td>
            <td>{formatExactTokens(totals.freshInputTokens)}</td>
            <td>
              {formatExactTokens(totals.cachedInputTokens)}
              <small>{totals.cacheHitPercent.toFixed(1)}%</small>
            </td>
            {showCacheWrite && (
              <td>{formatExactTokens(totals.cacheWriteInputTokens)}</td>
            )}
            <td>{formatExactTokens(totals.outputTokens)}</td>
            <td>
              <strong>{formatExactTokens(totals.totalTokens)}</strong>
              {totals.accountTokens > 0 && (
                <small>
                  {zh ? "账户视图" : "Account view"}{" "}
                  {formatReadableTokens(totals.accountTokens)}
                </small>
              )}
            </td>
            <td>
              <strong>{formatExactUsd(totals.costUsd)}</strong>
              <small>{zh ? "本地估算" : "Local estimate"}</small>
            </td>
          </tr>
        </tfoot>
        )}
      </table>
    </div>
  );
}

function LoadingState({ t }: { t: Translator }) {
  return (
    <main className="state-screen loading-state">
      <LoaderCircle className="standard-loader app-loader" aria-hidden="true" />
      <p>{t("loading.title")}</p>
      <span>{t("loading.note")}</span>
    </main>
  );
}

function ErrorState({
  message,
  onRetry,
  t,
}: {
  message: string;
  onRetry: () => void;
  t: Translator;
}) {
  return (
    <main className="state-screen error-state">
      <CircleAlert className="error-mark" aria-hidden="true" />
      <h1>{t("error.usageTitle")}</h1>
      <p>{message}</p>
      <button onClick={onRetry}>{t("error.retry")}</button>
    </main>
  );
}

function App() {
  const [locale, setLocale] = useState<Locale>(readLocale);
  const [theme, setTheme] = useState<Theme>(readTheme);
  const [activeView, setActiveView] = useState<ActiveView>("usage");
  const [consoleMounted, setConsoleMounted] = useState(false);
  const [usageReport, setUsageReport] = useState<UsageReport>("overview");
  const usageWorkspaceRef = useRef<HTMLElement | null>(null);
  const t = useMemo(() => createTranslator(locale), [locale]);
  setFormatLocale(locale);
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(
    readUsageBootCache,
  );
  const [costEstimate, setCostEstimate] =
    useState<CostEstimateSnapshot | null>(readCostBootCache);
  const [loading, setLoading] = useState(snapshot === null);
  const [refreshing, setRefreshing] = useState(false);
  const [usingCache, setUsingCache] = useState(snapshot !== null);
  const [error, setError] = useState<string | null>(null);
  const [costLoading, setCostLoading] = useState(costEstimate === null);
  const [costUsingCache, setCostUsingCache] = useState(costEstimate !== null);
  const [costError, setCostError] = useState<string | null>(null);
  const [projectUsage, setProjectUsage] =
    useState<ProjectUsageSnapshot | null>(null);
  const [projectUsageLoading, setProjectUsageLoading] = useState(false);
  const [projectUsageError, setProjectUsageError] = useState<string | null>(
    null,
  );
  const [projectUsageLoaded, setProjectUsageLoaded] = useState(false);
  const [now, setNow] = useState(Date.now());
  const [historyRange, setHistoryRange] = useState<HistoryRange>("14d");
  const [historyDisplay, setHistoryDisplay] =
    useState<HistoryDisplay>("table");
  const [historyOrder, setHistoryOrder] = useState<HistoryOrder>("desc");
  const [expandedUsageRows, setExpandedUsageRows] = useState<string[]>([]);
  const [traceSnapshot, setTraceSnapshot] = useState<TraceSnapshot | null>(
    readTraceBootCache,
  );
  const [traceLoading, setTraceLoading] = useState(false);
  const [traceUsingCache, setTraceUsingCache] = useState(
    traceSnapshot !== null,
  );
  const [traceError, setTraceError] = useState<string | null>(null);
  const [traceLoaded, setTraceLoaded] = useState(false);

  useEffect(() => {
    writeLocale(locale);
    setFormatLocale(locale);
    document.documentElement.lang = locale;
    document.title = "Codex X-Ray";
  }, [locale]);

  useEffect(() => {
    writeTheme(theme);
  }, [theme]);

  const applySnapshot = useCallback(
    (next: UsageSnapshot, fromCache: boolean) => {
      writeBootCache(USAGE_BOOT_CACHE_KEY, next);
      setSnapshot(next);
      setUsingCache(fromCache);
    },
    [],
  );

  const applyCostEstimate = useCallback(
    (next: CostEstimateSnapshot, fromCache: boolean) => {
      writeBootCache(COST_BOOT_CACHE_KEY, next);
      setCostEstimate(next);
      setCostUsingCache(fromCache);
    },
    [],
  );

  const loadUsage = useCallback(
    async (mode: UsageRefreshMode = "initial") => {
      if (mode === "initial") setLoading(true);
      if (mode === "manual") setRefreshing(true);
      setError(null);

      try {
        const next = await invoke<UsageSnapshot>("get_usage");
        applySnapshot(next, false);
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setLoading(false);
        if (mode === "manual") setRefreshing(false);
      }
    },
    [applySnapshot],
  );

  const loadCostEstimate = useCallback(async () => {
    setCostLoading(true);
    setCostError(null);
    try {
      const next = await invoke<CostEstimateSnapshot>("get_cost_estimate");
      applyCostEstimate(next, false);
    } catch (reason) {
      setCostError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setCostLoading(false);
    }
  }, [applyCostEstimate]);

  const loadProjectUsage = useCallback(async () => {
    setProjectUsageLoading(true);
    setProjectUsageError(null);
    try {
      const next = await invoke<ProjectUsageSnapshot>("get_project_usage");
      setProjectUsage(next);
    } catch (reason) {
      setProjectUsageError(
        reason instanceof Error ? reason.message : String(reason),
      );
    } finally {
      setProjectUsageLoading(false);
      setProjectUsageLoaded(true);
    }
  }, []);

  const loadProjectTurns = useCallback(async (sessionId: string) => {
    const detail = await invoke<ProjectTurnUsageDetail>(
      "get_project_turn_usage",
      { sessionId },
    );
    setProjectUsage((current) => {
      if (!current) return current;
      let snapshotTurnDelta = 0;
      let indexedSessionDelta = 0;
      let found = false;
      const projects = current.projects.map((project) => {
        let projectTurnDelta = 0;
        let projectIndexedDelta = 0;
        const conversations = project.conversations.map((conversation) => {
          if (conversation.id !== sessionId) return conversation;
          found = true;
          projectTurnDelta = detail.turns.length - conversation.turns;
          projectIndexedDelta = conversation.turns_indexed ? 0 : 1;
          snapshotTurnDelta += projectTurnDelta;
          indexedSessionDelta += projectIndexedDelta;
          return {
            ...conversation,
            turns: detail.turns.length,
            turns_indexed: true,
            turn_rows: detail.turns,
          };
        });
        if (projectTurnDelta === 0 && projectIndexedDelta === 0) return project;
        return {
          ...project,
          turns: project.turns + projectTurnDelta,
          turn_sessions_indexed:
            project.turn_sessions_indexed + projectIndexedDelta,
          conversations,
        };
      });
      if (!found) return current;
      return {
        ...current,
        turns: current.turns + snapshotTurnDelta,
        turn_sessions_indexed:
          current.turn_sessions_indexed + indexedSessionDelta,
        projects,
      };
    });
  }, []);

  const openUsageTrace = useCallback(
    (sessionId: string, turnId?: string) => {
      window.localStorage.setItem(TRACE_SELECTED_SESSION_KEY, sessionId);
      window.localStorage.setItem(TRACE_TARGET_SESSION_KEY, sessionId);
      if (turnId) {
        window.localStorage.setItem(TRACE_TARGET_TURN_KEY, turnId);
      } else {
        window.localStorage.removeItem(TRACE_TARGET_TURN_KEY);
      }
      window.localStorage.removeItem(TRACE_TARGET_CALL_KEY);
      setActiveView("trace");
    },
    [],
  );

  const applyTraceSnapshot = useCallback(
    (next: TraceSnapshot, fromCache: boolean) => {
      writeBootCache(TRACE_BOOT_CACHE_KEY, next);
      setTraceSnapshot(next);
      setTraceUsingCache(fromCache);
    },
    [],
  );

  const loadTrace = useCallback(async () => {
    setTraceLoading(true);
    setTraceError(null);
    try {
      const next = await invoke<TraceSnapshot>("get_trace_catalog");
      applyTraceSnapshot(next, false);
    } catch (reason) {
      setTraceError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setTraceLoading(false);
      setTraceLoaded(true);
    }
  }, [applyTraceSnapshot]);

  useEffect(() => {
    let cancelled = false;
    const bootstrap = async () => {
      let restoredCache = snapshot !== null;
      try {
        const cached = await invoke<UsageSnapshot | null>("get_cached_usage");
        if (cached && !cancelled) {
          applySnapshot(cached, true);
          setLoading(false);
          restoredCache = true;
        }
      } catch {
        // A missing or incompatible cache should never block a live refresh.
      }
      if (!cancelled) {
        await loadUsage(restoredCache ? "background" : "initial");
      }
    };
    void bootstrap();
    const refreshTimer = window.setInterval(
      () => void loadUsage("background"),
      60_000,
    );
    const clockTimer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => {
      cancelled = true;
      window.clearInterval(refreshTimer);
      window.clearInterval(clockTimer);
    };
  }, [applySnapshot, loadUsage]);

  useEffect(() => {
    let cancelled = false;
    let refreshTimer: number | undefined;
    const bootstrapCost = async () => {
      let restoredCache = costEstimate !== null;
      try {
        const cached = await invoke<CostEstimateSnapshot | null>(
          "get_cached_cost_estimate",
        );
        if (cached && !cancelled) {
          applyCostEstimate(cached, true);
          restoredCache = true;
        }
      } catch {
        // A missing cost cache only means the first local index is still needed.
      }
      if (!cancelled) {
        if (restoredCache) {
          refreshTimer = window.setTimeout(
            () => void loadCostEstimate(),
            1_200,
          );
        } else {
          void loadCostEstimate();
        }
      }
    };
    void bootstrapCost();
    return () => {
      cancelled = true;
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
    };
  }, [applyCostEstimate, loadCostEstimate]);

  useEffect(() => {
    if (activeView !== "trace" || traceLoaded) return;
    let cancelled = false;
    const bootstrapTrace = async () => {
      let restoredCache = traceSnapshot !== null;
      try {
        const cached = await invoke<TraceSnapshot | null>("get_cached_trace");
        if (cached && !cancelled) {
          applyTraceSnapshot(cached, true);
          restoredCache = true;
        }
      } catch {
        // The first trace view can build from local sessions without a cache.
      }
      if (!cancelled) {
        if (!restoredCache) setTraceLoading(true);
        await loadTrace();
      }
    };
    void bootstrapTrace();
    return () => {
      cancelled = true;
    };
  }, [
    activeView,
    applyTraceSnapshot,
    loadTrace,
    traceLoaded,
  ]);

  useEffect(() => {
    if (
      activeView !== "usage" ||
      usageReport !== "projects" ||
      projectUsageLoaded
    ) {
      return;
    }
    let cancelled = false;
    const bootstrapProjectUsage = async () => {
      setProjectUsageLoading(true);
      try {
        const cached = await invoke<ProjectUsageSnapshot | null>(
          "get_cached_project_usage",
        );
        if (cached && !cancelled) {
          setProjectUsage(cached);
          setProjectUsageLoading(false);
        }
      } catch {
        // The first project report can be built directly from the cost index.
      }
      if (!cancelled) await loadProjectUsage();
    };
    void bootstrapProjectUsage();
    return () => {
      cancelled = true;
    };
  }, [
    activeView,
    loadProjectUsage,
    projectUsageLoaded,
    usageReport,
  ]);

  const periodPoints = useMemo(
    () =>
      snapshot
        ? buildPeriodPoints(
            historyRange,
            snapshot.daily_usage,
            snapshot.local_today,
            costEstimate,
            locale,
            t,
          )
        : [],
    [costEstimate, historyRange, locale, snapshot, t],
  );
  const ledgerRows = useMemo(
    () =>
      buildUsageLedgerRows(
        costEstimate,
        periodPoints,
        historyRange,
        locale,
        t,
      ),
    [costEstimate, historyRange, locale, periodPoints, t],
  );
  const ledgerSummary = useMemo(
    () => summarizeLedger(ledgerRows),
    [ledgerRows],
  );
  const localChartPoints = useMemo(
    () => ledgerChartPoints(ledgerRows),
    [ledgerRows],
  );

  if (loading && !snapshot) return <LoadingState t={t} />;
  if (error && !snapshot) {
    return (
      <ErrorState
        message={error}
        onRetry={() => void loadUsage("initial")}
        t={t}
      />
    );
  }
  if (!snapshot) return null;

  const localToday = snapshot.local_today;
  const officialToday = snapshot.daily_usage.find(
    (item) => item.start_date === localDateKey(),
  );
  const todayTotal = localToday?.total_tokens ?? officialToday?.tokens ?? 0;
  const dailyCosts = mergedDailyCosts(costEstimate, localToday);
  const indexedTodayCost = costEstimate?.daily.find(
    (item) => item.date === localDateKey(),
  );
  const todayCost =
    localToday?.estimated_cost_usd ?? indexedTodayCost?.cost_usd ?? null;
  const adjustedLifetimeCost =
    costEstimate == null
      ? todayCost
      : Math.max(
          0,
          costEstimate.total_cost_usd -
            (indexedTodayCost?.cost_usd ?? 0) +
            (todayCost ?? indexedTodayCost?.cost_usd ?? 0),
        );
  const currentMonth = localDateKey().slice(0, 7);
  const currentMonthCost = [...dailyCosts.entries()]
    .filter(([date]) => date.startsWith(currentMonth))
    .reduce((sum, [, item]) => sum + item.costUsd, 0);
  const topCostModel = costEstimate?.models.find((model) => model.priced);
  const hasCustomPricing =
    costEstimate?.pricing_basis.includes("用户自定义") ?? false;
  const costCoverageTokens =
    (costEstimate?.priced_tokens ?? 0) + (costEstimate?.unpriced_tokens ?? 0);
  const costCoveragePercent =
    costCoverageTokens === 0
      ? 0
      : ((costEstimate?.priced_tokens ?? 0) / costCoverageTokens) * 100;
  const hasCostData = Boolean(costEstimate || localToday);
  const costUncachedInput =
    costEstimate?.uncached_input_cost_usd ??
    localToday?.uncached_input_cost_usd ??
    0;
  const costCachedInput =
    costEstimate?.cached_input_cost_usd ??
    localToday?.cached_input_cost_usd ??
    0;
  const costOutput =
    costEstimate?.output_cost_usd ?? localToday?.output_cost_usd ?? 0;
  const costPartsTotal = costUncachedInput + costCachedInput + costOutput;
  const cacheSavings =
    costEstimate?.cache_savings_usd ?? localToday?.cache_savings_usd ?? null;
  const primaryLimit =
    snapshot.rate_limits.find(
      (item) => item.limit_id === snapshot.current_limit_id,
    ) ??
    snapshot.rate_limits.find((item) => item.limit_id === "codex") ??
    snapshot.rate_limits[0] ??
    null;
  const additionalLimits = snapshot.rate_limits.filter(
    (item) => item.limit_id !== primaryLimit?.limit_id,
  );
  const visibleAdditionalLimits = additionalLimits.filter(
    (limit) =>
      (limit.primary?.used_percent ?? 0) > 0 ||
      (limit.secondary?.used_percent ?? 0) > 0 ||
      (limit.credits != null &&
        !limit.credits.unlimited &&
        limit.credits.balance != null),
  );
  const hasWindows = Boolean(primaryLimit?.primary || primaryLimit?.secondary);
  const officialUnlimited =
    primaryLimit?.credits?.unlimited === true && !hasWindows;
  const openExternal = (url: string) => {
    void invoke("open_external", { url });
  };
  const startWindowDrag = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    void getCurrentWindow().startDragging().catch(() => {
      // The native title bar remains the fallback outside the Tauri runtime.
    });
  };

  return (
    <div className="app-shell">
      <div
        className="window-titlebar"
        data-tauri-drag-region
        onMouseDown={startWindowDrag}
        aria-hidden="true"
      />
      <aside className="sidebar">
        <div className="brand">
          <BrandMark />
          <span className="brand-copy">
            <strong>Codex X-Ray</strong>
            <small title={t("brand.tagline")}>{t("brand.tagline")}</small>
          </span>
        </div>

        <nav aria-label={t("nav.label")}>
          <button
            className={`nav-item${activeView === "usage" ? " active" : ""}`}
            aria-current={activeView === "usage" ? "page" : undefined}
            onClick={() => setActiveView("usage")}
          >
            <ChartNoAxesCombined className="nav-icon" aria-hidden="true" />
            {t("nav.usage")}
          </button>
          <button
            className={`nav-item${activeView === "trace" ? " active" : ""}`}
            aria-current={activeView === "trace" ? "page" : undefined}
            onClick={() => setActiveView("trace")}
          >
            <ScanSearch className="nav-icon" aria-hidden="true" />
            {t("nav.trace")}
          </button>
          <button
            className={`nav-item${activeView === "remote" ? " active" : ""}`}
            aria-current={activeView === "remote" ? "page" : undefined}
            onClick={() => setActiveView("remote")}
          >
            <RadioTower className="nav-icon" aria-hidden="true" />
            {t("nav.remote")}
          </button>
          <button
            className={`nav-item${activeView === "access" ? " active" : ""}`}
            aria-current={activeView === "access" ? "page" : undefined}
            onClick={() => {
              setConsoleMounted(true);
              setActiveView("access");
            }}
          >
            <Cable className="nav-icon" aria-hidden="true" />
            {t("nav.access")}
          </button>
          <button
            className={`nav-item${activeView === "console" ? " active" : ""}`}
            aria-current={activeView === "console" ? "page" : undefined}
            onClick={() => {
              setConsoleMounted(true);
              setActiveView("console");
            }}
          >
            <SlidersHorizontal className="nav-icon" aria-hidden="true" />
            {t("nav.console")}
          </button>
        </nav>

        <div className="sidebar-controls">
          <span>{t("language.label")}</span>
          <div className="language-switcher" role="group" aria-label={t("language.label")}>
            <button
              className={locale === "zh-CN" ? "selected" : ""}
              aria-pressed={locale === "zh-CN"}
              onClick={() => setLocale("zh-CN")}
            >
              {t("language.zh")}
            </button>
            <button
              className={locale === "en-US" ? "selected" : ""}
              aria-pressed={locale === "en-US"}
              onClick={() => setLocale("en-US")}
            >
              {t("language.en")}
            </button>
          </div>
          <span className="theme-control-label">
            {locale === "zh-CN" ? "主题" : "Theme"}
          </span>
          <div
            className="theme-switcher"
            role="group"
            aria-label={locale === "zh-CN" ? "界面主题" : "Interface theme"}
          >
            <button
              className={theme === "light" ? "selected" : ""}
              aria-pressed={theme === "light"}
              onClick={() => setTheme("light")}
            >
              <Sun aria-hidden="true" />
              {locale === "zh-CN" ? "亮色" : "Light"}
            </button>
            <button
              className={theme === "dark" ? "selected" : ""}
              aria-pressed={theme === "dark"}
              onClick={() => setTheme("dark")}
            >
              <Moon aria-hidden="true" />
              {locale === "zh-CN" ? "暗色" : "Dark"}
            </button>
          </div>
          <UpdateControl locale={locale} onOpenUrl={openExternal} />
        </div>

      </aside>

      {consoleMounted && (
        <ProviderView
          locale={locale}
          onOpenUrl={openExternal}
          surface={activeView === "console" ? "console" : "access"}
          hidden={activeView !== "access" && activeView !== "console"}
        />
      )}
      {activeView === "trace" && (
        <TraceView
          locale={locale}
          t={t}
          snapshot={traceSnapshot}
          loading={traceLoading}
          usingCache={traceUsingCache}
          error={traceError}
          onRefresh={() => void loadTrace()}
        />
      )}
      {activeView === "remote" && <RemoteControlView locale={locale} />}
      {activeView === "usage" && (
      <main
        ref={usageWorkspaceRef}
        className={`workspace usage-workspace${
          usageReport === "overview" ? " overview" : ""
        }`}
      >
        <header className="topbar">
          <div>
            <h1>{t("usage.title")}</h1>
            <span className="header-note">{t("usage.subtitle")}</span>
          </div>
          <div className="topbar-actions">
            <span>
              {formatPlan(snapshot.account?.plan_type)}
              <i />
              {usingCache
                ? `${t("common.cacheAt", {
                    time: formatSyncTime(snapshot.fetched_at),
                  })} · ${
                    error
                      ? t("common.updateFailed")
                      : t("common.backgroundUpdating")
                  }`
                : `${formatSyncTime(snapshot.fetched_at)} ${t("common.updated")}`}
            </span>
            <button
              className={
                refreshing ? "refresh-button spinning" : "refresh-button"
              }
              onClick={() => {
                void loadUsage("manual");
                if (!costLoading) {
                  void loadCostEstimate().then(() => {
                    if (usageReport === "projects") void loadProjectUsage();
                  });
                }
              }}
              disabled={refreshing}
              aria-label={t("usage.refreshLabel")}
            >
              <RefreshCw aria-hidden="true" />
            </button>
          </div>
        </header>

        <nav
          className="usage-report-nav"
          aria-label={locale === "zh-CN" ? "用量报表" : "Usage reports"}
        >
          <div>
            {(
              [
                ["overview", locale === "zh-CN" ? "概览" : "Overview"],
                ["daily", locale === "zh-CN" ? "按日" : "Daily"],
                ["monthly", locale === "zh-CN" ? "按月" : "Monthly"],
                ["projects", locale === "zh-CN" ? "按项目" : "Projects"],
                ["models", locale === "zh-CN" ? "模型成本" : "Model cost"],
              ] as [UsageReport, string][]
            ).map(([report, label]) => (
              <button
                key={report}
                className={usageReport === report ? "active" : ""}
                aria-current={usageReport === report ? "page" : undefined}
                onClick={() => {
                  usageWorkspaceRef.current?.scrollTo({
                    top: 0,
                    left: 0,
                    behavior: "auto",
                  });
                  setUsageReport(report);
                  if (report === "daily") setHistoryRange("30d");
                  if (report === "monthly") setHistoryRange("monthly");
                  if (report === "daily" || report === "monthly") {
                    setHistoryDisplay("table");
                  }
                  setExpandedUsageRows([]);
                }}
              >
                {label}
              </button>
            ))}
          </div>
        </nav>

        {usageReport === "projects" && (
          <ProjectUsageView
            locale={locale}
            snapshot={projectUsage}
            loading={projectUsageLoading}
            error={projectUsageError}
            onRefresh={() => {
              void loadCostEstimate().then(loadProjectUsage);
            }}
            onOpenTrace={openUsageTrace}
            onLoadTurns={loadProjectTurns}
          />
        )}

        {usageReport === "overview" && (
        <section className="quota-section">
          <div className="section-heading">
            <div className="heading-with-help">
              <h2>{t("quota.heading")}</h2>
              <InfoTip text={t("quota.help")} />
            </div>
          </div>

          <div className="quota-summary-grid">
            <div className="quota-account-card">
              <span>
                {t("quota.plan")}
                <InfoTip text={t("quota.planHelp")} />
              </span>
              <strong>
                {formatPlan(primaryLimit?.plan_type ?? snapshot.account?.plan_type)}
              </strong>
            </div>

            <div className="quota-primary-card">
              <div className="quota-card-heading">
                <div>
                  <span>{t("quota.generalLabel")}</span>
                  <strong>
                    {primaryLimit
                      ? friendlyLimitName(primaryLimit, t)
                    : t("quota.codexGeneric")}
                  </strong>
                </div>
              </div>
              {hasWindows ? (
                <div className="quota-windows">
                  {primaryLimit?.primary && (
                    <QuotaWindowRow
                      window={primaryLimit.primary}
                      fallbackMinutes={300}
                      now={now}
                      locale={locale}
                      t={t}
                    />
                  )}
                  {primaryLimit?.secondary && (
                    <QuotaWindowRow
                      window={primaryLimit.secondary}
                      fallbackMinutes={10_080}
                      now={now}
                      locale={locale}
                      t={t}
                    />
                  )}
                </div>
              ) : (
                <div className="quota-unavailable">
                  <div className="quota-unavailable-heading">
                    <strong>
                      {officialUnlimited
                        ? t("quota.unmetered")
                        : t("quota.noWindow")}
                    </strong>
                    <InfoTip
                      text={
                        officialUnlimited
                          ? t("quota.unmeteredNote")
                          : t("quota.noWindowNote")
                      }
                    />
                  </div>
                </div>
              )}
            </div>

            <div className="quota-credits-card">
              <span>
                {t("quota.credits")}
                <InfoTip text={t("quota.creditsHelp")} />
              </span>
              <strong>
                {primaryLimit?.credits?.unlimited
                  ? t("quota.unlimited")
                  : primaryLimit?.credits?.balance ??
                    (primaryLimit?.credits
                      ? "0"
                      : t("quota.notApplicable"))}
              </strong>
            </div>
          </div>

          {visibleAdditionalLimits.length > 0 && (
            <div className="additional-limits">
              <div>
                <strong>{t("quota.additional")}</strong>
                <InfoTip text={t("quota.additionalNote")} />
              </div>
              <ul>
                {visibleAdditionalLimits.map((limit) => (
                  <li key={limit.limit_id}>
                    <div>
                      <strong>{friendlyLimitName(limit, t)}</strong>
                      <small>{limit.limit_name ?? t("quota.modelSpecific")}</small>
                    </div>
                    <span>
                      {limit.primary
                        ? t("quota.usedValue", {
                            value: limit.primary.used_percent.toFixed(
                              limit.primary.used_percent % 1 === 0 ? 0 : 1,
                            ),
                          })
                        : limit.credits?.unlimited
                          ? t("quota.unlimited")
                          : t("quota.noWindowShort")}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </section>
        )}

        {usageReport === "overview" && (
        <section className="today-section">
          <div className="today-total">
            <div className="heading-with-help">
              <p>{t("usage.today")}</p>
              <InfoTip text={t("usage.todayHelp")} align="start" />
            </div>
            <strong
              title={`${formatExactTokens(todayTotal)} Token`}
              aria-label={`${t("usage.today")}: ${formatExactTokens(todayTotal)} Token`}
            >
              {formatReadableTokens(todayTotal)}
            </strong>
            <div
              className="today-cost-line"
              title={formatExactUsd(todayCost)}
            >
              <span>{t("usage.todayCost")}</span>
              <strong>{formatReadableUsd(todayCost)}</strong>
              <small>{t("usage.estimated")}</small>
            </div>
            <div className="today-source-line">
              <SourceBadge type={localToday ? "local" : "official"}>
                {localToday
                  ? t("usage.localLive")
                  : t("usage.officialDaily")}
              </SourceBadge>
              <small>
                {localToday
                  ? t("usage.latestEvent", {
                      time: formatEventTime(
                        localToday.latest_event_at,
                        locale,
                      ),
                    })
                  : t("usage.localUnavailable")}
              </small>
            </div>
          </div>

          <div className="today-breakdown">
            <Metric
              label={t("usage.input")}
              value={formatReadableTokens(localToday?.input_tokens)}
              exact={
                localToday
                  ? `${formatExactTokens(localToday.input_tokens)} Token`
                  : undefined
              }
              note={
                localToday ? undefined : t("usage.noBreakdown")
              }
              help={t("usage.inputHelp")}
            />
            <Metric
              label={
                localToday
                  ? t("usage.cachedInputWithRate", {
                      rate: localToday.cache_hit_percent.toFixed(2),
                    })
                  : t("usage.cachedInput")
              }
              value={formatReadableTokens(localToday?.cached_input_tokens)}
              exact={
                localToday
                  ? `${formatExactTokens(localToday.cached_input_tokens)} Token`
                  : undefined
              }
              note={
                localToday ? undefined : t("usage.noBreakdown")
              }
              help={t("usage.cachedInputHelp")}
              accent
            />
            <Metric
              label={t("usage.uncachedInput")}
              value={formatReadableTokens(localToday?.uncached_input_tokens)}
              exact={
                localToday
                  ? `${formatExactTokens(localToday.uncached_input_tokens)} Token`
                  : undefined
              }
              note={
                localToday ? undefined : t("usage.noBreakdown")
              }
              help={t("usage.uncachedHelp")}
            />
            <Metric
              label={t("usage.output")}
              value={formatReadableTokens(localToday?.output_tokens)}
              exact={
                localToday
                  ? `${formatExactTokens(localToday.output_tokens)} Token`
                  : undefined
              }
              note={
                localToday ? undefined : t("usage.noBreakdown")
              }
              secondary={
                localToday
                  ? t("usage.reasoning", {
                      value: formatReadableTokens(
                        localToday.reasoning_output_tokens,
                      ),
                    })
                  : undefined
              }
              help={t("usage.outputHelp")}
            />
          </div>

        </section>
        )}

        {usageReport === "overview" && (
          <section className="activity-section">
            <div className="activity-summary-strip">
              <Metric
                label={t("account.lifetime")}
                value={formatReadableTokens(snapshot.summary?.lifetime_tokens)}
                exact={
                  snapshot.summary?.lifetime_tokens == null
                    ? undefined
                    : `${formatExactTokens(snapshot.summary.lifetime_tokens)} Token`
                }
                help={t("account.lifetimeHelp")}
              />
              <Metric
                label={t("account.peak")}
                value={formatReadableTokens(snapshot.summary?.peak_daily_tokens)}
                exact={
                  snapshot.summary?.peak_daily_tokens == null
                    ? undefined
                    : `${formatExactTokens(snapshot.summary.peak_daily_tokens)} Token`
                }
                help={t("account.peakHelp")}
              />
              <Metric
                label={t("account.longestTurn")}
                value={formatDuration(snapshot.summary?.longest_running_turn_sec)}
                help={t("account.longestTurnHelp")}
              />
              <Metric
                label={
                  locale === "zh-CN" ? "当前连续天数" : "Current streak"
                }
                value={
                  snapshot.summary?.current_streak_days == null
                    ? "—"
                    : t("account.streakValue", {
                        count: snapshot.summary.current_streak_days,
                      })
                }
                help={t("account.streakHelp")}
              />
              <Metric
                label={
                  locale === "zh-CN" ? "最长连续天数" : "Longest streak"
                }
                value={
                  snapshot.summary?.longest_streak_days == null
                    ? "—"
                    : t("account.streakValue", {
                        count: snapshot.summary.longest_streak_days,
                      })
                }
                help={t("account.streakHelp")}
              />
            </div>
            <TokenActivityHeatmap
              official={snapshot.daily_usage}
              localToday={snapshot.local_today}
              costEstimate={costEstimate}
              locale={locale}
            />
          </section>
        )}

        {usageReport === "models" && (
        <section className="cost-section">
          <div className="cost-heading">
            <div className="cost-heading-copy">
              <div className="heading-with-help">
                <h2>{t("cost.heading")}</h2>
                <InfoTip text={t("cost.help")} />
              </div>
              <span>
                {locale === "zh-CN"
                  ? "按本地 Session 的模型和 Token 类型估算"
                  : "Estimated from local session models and token types"}
              </span>
            </div>
            <div className="cost-status">
              <PricingSettings
                locale={locale}
                observedModels={costEstimate?.models ?? []}
                onApplied={async () => {
                  const refreshes = [
                    loadCostEstimate(),
                    loadUsage("background"),
                  ];
                  if (traceLoaded) refreshes.push(loadTrace());
                  await Promise.all(refreshes);
                  if (projectUsageLoaded) await loadProjectUsage();
                }}
              />
              <span className="cost-disclaimer">{t("cost.disclaimer")}</span>
              <span>
                {costLoading
                  ? costEstimate
                    ? t("cost.updating")
                    : t("cost.firstIndex")
                  : costError
                    ? t("cost.updateFailed")
                    : costUsingCache
                      ? t("cost.cached")
                      : t("cost.updated")}
              </span>
            </div>
          </div>

          {hasCostData ? (
            <>
              <div className="cost-overview">
                <div className="cost-total">
                  <span>{t("cost.localLifetime")}</span>
                  <strong>{formatReadableUsd(adjustedLifetimeCost)}</strong>
                  <small>
                    {formatExactUsd(adjustedLifetimeCost)} ·{" "}
                    {t("cost.coverage", {
                      start:
                        costEstimate?.coverage_start ??
                        localToday?.date ??
                        t("common.unknown"),
                      end:
                        costEstimate?.coverage_end ??
                        localToday?.date ??
                        t("common.today"),
                    })}
                  </small>
                  <p>
                    {t("cost.coverageRate", {
                      rate: costEstimate
                        ? costCoveragePercent.toFixed(2)
                        : localToday && localToday.total_tokens > 0
                          ? (
                              (localToday.priced_tokens /
                                localToday.total_tokens) *
                              100
                            ).toFixed(2)
                          : "0",
                    })}
                  </p>
                </div>

                <div className="cost-metrics">
                  <Metric
                    label={t("cost.todayEstimate")}
                    value={formatReadableUsd(todayCost)}
                    note={
                      localToday
                        ? t("cost.localRealtime", {
                            value: formatExactUsd(todayCost),
                          })
                        : t("cost.waitToday")
                    }
                    help={t("cost.todayHelp")}
                    accent
                  />
                  <Metric
                    label={t("cost.monthEstimate")}
                    value={formatReadableUsd(
                      dailyCosts.size > 0 ? currentMonthCost : null,
                    )}
                    note={t("cost.localRange", { month: currentMonth })}
                    help={t("cost.monthHelp")}
                  />
                  <Metric
                    label={t("cost.cacheSavings")}
                    value={formatReadableUsd(cacheSavings)}
                    note={t("cost.cacheSavingsNote")}
                    help={t("cost.cacheSavingsHelp")}
                  />
                  <Metric
                    label={t("cost.topModel")}
                    value={topCostModel?.model ?? t("cost.waitIndex")}
                    note={
                      topCostModel
                        ? `${formatExactUsd(topCostModel.cost_usd)} · ${formatReadableTokens(topCostModel.total_tokens)} Token`
                        : t("cost.firstIndexNote")
                    }
                    help={t("cost.topModelHelp")}
                  />
                </div>
              </div>

              <div className="cost-composition">
                <div
                  className="cost-bar"
                  aria-label={t("cost.composition")}
                >
                  <i
                    className="cost-bar-input"
                    style={{
                      width: `${costPartsTotal > 0 ? (costUncachedInput / costPartsTotal) * 100 : 0}%`,
                    }}
                  />
                  <i
                    className="cost-bar-cache"
                    style={{
                      width: `${costPartsTotal > 0 ? (costCachedInput / costPartsTotal) * 100 : 0}%`,
                    }}
                  />
                  <i
                    className="cost-bar-output"
                    style={{
                      width: `${costPartsTotal > 0 ? (costOutput / costPartsTotal) * 100 : 0}%`,
                    }}
                  />
                </div>
                <div className="cost-legend">
                  <span>
                    <i className="cost-dot-input" />
                    {t("cost.uncachedInput", {
                      value: formatExactUsd(costUncachedInput),
                    })}
                  </span>
                  <span>
                    <i className="cost-dot-cache" />
                    {t("cost.cachedInput", {
                      value: formatExactUsd(costCachedInput),
                    })}
                  </span>
                  <span>
                    <i className="cost-dot-output" />
                    {t("cost.output", { value: formatExactUsd(costOutput) })}
                  </span>
                  <small>
                    {hasCustomPricing
                      ? locale === "zh-CN"
                        ? "公开单价 + 按事件日期匹配的自定义版本"
                        : "Published prices + date-matched custom versions"
                      : t("cost.pricingBasis")}
                    {costEstimate
                      ? t("cost.pricingSnapshot", {
                          date: costEstimate.pricing_updated_at.slice(0, 10),
                        })
                      : ""}
                  </small>
                </div>
              </div>

              {costEstimate && costEstimate.models.length > 0 && (
                <div className="model-cost-table-wrap">
                  <div className="usage-compact-heading">
                    <div>
                      <h2>{locale === "zh-CN" ? "按模型" : "By model"}</h2>
                      <span>
                        {locale === "zh-CN"
                          ? "Token 与 API 等价成本"
                          : "Tokens and API-equivalent cost"}
                      </span>
                    </div>
                  </div>
                  <table className="model-cost-table">
                    <thead>
                      <tr>
                        <th>{locale === "zh-CN" ? "模型" : "Model"}</th>
                        <th>
                          {locale === "zh-CN"
                            ? "未缓存输入"
                            : "Fresh input"}
                        </th>
                        <th>{locale === "zh-CN" ? "缓存读取" : "Cache read"}</th>
                        {costEstimate.models.some(
                          (model) => model.cache_write_input_tokens > 0,
                        ) && (
                          <th>{locale === "zh-CN" ? "缓存写入" : "Cache write"}</th>
                        )}
                        <th>{locale === "zh-CN" ? "输出" : "Output"}</th>
                        <th>{locale === "zh-CN" ? "总 Token" : "Total tokens"}</th>
                        <th>{locale === "zh-CN" ? "估算成本" : "Est. cost"}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {[...costEstimate.models]
                        .sort((left, right) => right.cost_usd - left.cost_usd)
                        .map((model) => (
                          <tr key={model.model}>
                            <td>
                              <code>{model.model}</code>
                              {!model.priced && (
                                <small>
                                  {locale === "zh-CN" ? "未定价" : "Unpriced"}
                                </small>
                              )}
                            </td>
                            <td>
                              {formatExactTokens(
                                Math.max(
                                  model.input_tokens -
                                    model.cached_input_tokens,
                                  0,
                                ),
                              )}
                            </td>
                            <td>{formatExactTokens(model.cached_input_tokens)}</td>
                            {costEstimate.models.some(
                              (item) => item.cache_write_input_tokens > 0,
                            ) && (
                              <td>
                                {formatExactTokens(
                                  model.cache_write_input_tokens,
                                )}
                              </td>
                            )}
                            <td>{formatExactTokens(model.output_tokens)}</td>
                            <td>
                              <strong>{formatExactTokens(model.total_tokens)}</strong>
                            </td>
                            <td>{model.priced ? formatExactUsd(model.cost_usd) : "—"}</td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                </div>
              )}

              <div className="cost-index-note">
                <span>
                  {costEstimate
                    ? t("cost.indexStats", {
                        files: costEstimate.files_indexed,
                        scanned: costEstimate.files_scanned,
                        reused: costEstimate.files_reused,
                        elapsed: costEstimate.elapsed_ms,
                      })
                    : t("cost.indexBackground")}
                </span>
                <span>{t("cost.privacy")}</span>
              </div>
            </>
          ) : (
            <div className="cost-indexing">
              <span className="cost-index-pulse" />
              <div>
                <strong>{t("cost.building")}</strong>
                <p>{t("cost.buildingNote")}</p>
              </div>
            </div>
          )}

          {costError && <p className="cost-error">{costError}</p>}
        </section>
        )}

        {(usageReport === "daily" || usageReport === "monthly") && (
        <section className="history-section ledger-section">
          <div className="history-toolbar">
            <div>
              <div className="heading-with-help">
                <h2>
                  {usageReport === "monthly"
                    ? locale === "zh-CN"
                      ? "按月用量"
                      : "Monthly usage"
                    : locale === "zh-CN"
                      ? "按日用量"
                      : "Daily usage"}
                </h2>
                <InfoTip
                  text={
                    locale === "zh-CN"
                      ? "按本地 Session 聚合。总 Token = 输入 + 输出；缓存读取已包含在输入中。金额只对能识别模型和单价的 Token 进行估算。"
                      : "Aggregated from local sessions. Total tokens = input + output; cache reads are included in input. Cost is estimated only when model pricing is known."
                  }
                />
              </div>
              <span>
                {locale === "zh-CN"
                  ? "展开周期查看逐模型明细"
                  : "Expand a period for per-model detail"}
              </span>
            </div>
            <div className="history-controls">
              {usageReport === "daily" && (
                <div
                  className="range-switcher"
                  role="tablist"
                  aria-label={t("history.tabs")}
                >
                  {(
                    [
                      ["14d", t("history.tab14")],
                      ["30d", t("history.tab30")],
                    ] as const
                  ).map(([range, label]) => (
                    <button
                      key={range}
                      role="tab"
                      aria-selected={historyRange === range}
                      className={historyRange === range ? "selected" : ""}
                      onClick={() => {
                        setHistoryRange(range);
                        setExpandedUsageRows([]);
                      }}
                    >
                      {label}
                    </button>
                  ))}
                </div>
              )}
              <div
                className="report-display-switcher"
                role="tablist"
                aria-label={
                  locale === "zh-CN" ? "报表展示方式" : "Report display"
                }
              >
                {(
                  [
                    ["table", locale === "zh-CN" ? "明细表" : "Details"],
                    ["trend", locale === "zh-CN" ? "趋势" : "Trend"],
                  ] as [HistoryDisplay, string][]
                ).map(([display, label]) => (
                  <button
                    key={display}
                    role="tab"
                    aria-selected={historyDisplay === display}
                    className={historyDisplay === display ? "selected" : ""}
                    onClick={() => setHistoryDisplay(display)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
          </div>

          <div className="ledger-summary-strip">
            <div className="primary">
              <span>{locale === "zh-CN" ? "本地 Token 总量" : "Local token total"}</span>
              <strong>{formatReadableTokens(ledgerSummary.totalTokens)}</strong>
              <small>{formatExactTokens(ledgerSummary.totalTokens)} Token</small>
            </div>
            <div>
              <span>{locale === "zh-CN" ? "未缓存输入" : "Fresh input"}</span>
              <strong>
                {formatReadableTokens(ledgerSummary.freshInputTokens)}
              </strong>
              <small>{formatExactTokens(ledgerSummary.freshInputTokens)}</small>
            </div>
            <div className="accent">
              <span>
                {locale === "zh-CN"
                  ? `缓存读取 · ${ledgerSummary.cacheHitPercent.toFixed(1)}%`
                  : `Cache read · ${ledgerSummary.cacheHitPercent.toFixed(1)}%`}
              </span>
              <strong>
                {formatReadableTokens(ledgerSummary.cachedInputTokens)}
              </strong>
              <small>{formatExactTokens(ledgerSummary.cachedInputTokens)}</small>
            </div>
            <div>
              <span>{locale === "zh-CN" ? "输出" : "Output"}</span>
              <strong>{formatReadableTokens(ledgerSummary.outputTokens)}</strong>
              <small>{formatExactTokens(ledgerSummary.outputTokens)}</small>
            </div>
            <div>
              <span>
                {locale === "zh-CN" ? "API 等价成本" : "API-equivalent cost"}
              </span>
              <strong>{formatReadableUsd(ledgerSummary.costUsd)}</strong>
              <small>{formatExactUsd(ledgerSummary.costUsd)}</small>
            </div>
          </div>

          <div className="ledger-scope-bar">
            <span>
              <b>{locale === "zh-CN" ? "数据源" : "Source"}</b>
              {locale === "zh-CN"
                ? "本地 Session"
                : "Local sessions"}
            </span>
            <span>
              <b>{locale === "zh-CN" ? "官方账户" : "Official account"}</b>
              {formatReadableTokens(ledgerSummary.accountTokens)}
              {ledgerSummary.accountTokens > 0 && (
                <>
                  {" · "}
                  {locale === "zh-CN" ? "本地差额 " : "local delta "}
                  {ledgerSummary.accountDelta >= 0 ? "+" : "−"}
                  {formatReadableTokens(
                    Math.abs(ledgerSummary.accountDelta),
                  )}
                </>
              )}
            </span>
            <small>
              {locale === "zh-CN"
                ? "两者统计口径不同"
                : "Different reporting scopes"}
            </small>
          </div>

          {costLoading && ledgerRows.length === 0 ? (
            <div className="ledger-loading" role="status">
              <span className="cost-index-pulse" />
              <div>
                <strong>
                  {locale === "zh-CN"
                    ? "正在建立本地明细索引"
                    : "Building local detail index"}
                </strong>
                <small>
                  {locale === "zh-CN"
                    ? "完成后会显示输入、缓存、输出与模型"
                    : "Input, cache, output, and model detail will appear when ready"}
                </small>
              </div>
            </div>
          ) : historyDisplay === "table" ? (
            <UsageLedgerTable
              rows={ledgerRows}
              locale={locale}
              range={historyRange}
              order={historyOrder}
              onToggleOrder={() =>
                setHistoryOrder((current) =>
                  current === "desc" ? "asc" : "desc",
                )
              }
              expandedKeys={expandedUsageRows}
              onToggleExpanded={(key) =>
                setExpandedUsageRows((current) =>
                  current.includes(key)
                    ? current.filter((item) => item !== key)
                    : [...current, key],
                )
              }
            />
          ) : (
            <>
              <UsageChart
                points={localChartPoints}
                range={historyRange}
                locale={locale}
                t={t}
                sourceMode="local"
              />
              <div className="chart-meta">
                <div className="chart-legend">
                  <span>
                    <i className="legend-local" />
                    {locale === "zh-CN"
                      ? "本地 Session 总 Token"
                      : "Local session total tokens"}
                  </span>
                  <span>
                    <i className="legend-partial" />
                    {t("history.legendPartial")}
                  </span>
                  <span>{t("history.legendCost")}</span>
                </div>
                <span className="coverage-note">
                  {locale === "zh-CN" ? "本地索引范围：" : "Local index range: "}
                  {costEstimate?.coverage_start ?? t("common.unknown")}{" "}
                  {locale === "zh-CN" ? "至" : "to"}{" "}
                  {costEstimate?.coverage_end ?? t("common.unknown")}
                </span>
              </div>
            </>
          )}

          <div className="account-rollup">
            <Metric
              label={t("account.lifetime")}
              value={formatReadableTokens(snapshot.summary?.lifetime_tokens)}
              help={t("account.lifetimeHelp")}
            />
            <Metric
              label={t("account.peak")}
              value={formatReadableTokens(snapshot.summary?.peak_daily_tokens)}
              help={t("account.peakHelp")}
            />
            <Metric
              label={t("account.streak")}
              value={
                snapshot.summary?.current_streak_days == null
                  ? "—"
                  : t("account.streakValue", {
                      count: snapshot.summary.current_streak_days,
                    })
              }
              note={t("account.longestStreak", {
                count: snapshot.summary?.longest_streak_days ?? "—",
              })}
              help={t("account.streakHelp")}
            />
            <Metric
              label={t("account.longestTurn")}
              value={formatDuration(snapshot.summary?.longest_running_turn_sec)}
              help={t("account.longestTurnHelp")}
            />
          </div>
        </section>
        )}

        {(snapshot.warnings.length > 0 ||
          (costEstimate?.warnings.length ?? 0) > 0 ||
          error) && (
          <section className="warnings" aria-live="polite">
            <strong>{t("warnings.partial")}</strong>
            {[
              ...(error ? [error] : []),
              ...snapshot.warnings,
              ...(costEstimate?.warnings ?? []),
            ].map((warning, index) => (
              <span key={`${warning}-${index}`}>{warning}</span>
            ))}
          </section>
        )}

      </main>
      )}
    </div>
  );
}

export default App;
