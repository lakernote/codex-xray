import type { Locale } from "./i18n";

let activeLocale: Locale = "zh-CN";

export function setFormatLocale(locale: Locale): void {
  activeLocale = locale;
}

function exactNumber(): Intl.NumberFormat {
  return new Intl.NumberFormat(activeLocale);
}

const usdNumber = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

export function formatTokens(value: number | null | undefined): string {
  if (value == null) return "—";
  return new Intl.NumberFormat(activeLocale, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}

export function formatReadableTokens(
  value: number | null | undefined,
): string {
  if (value == null) return "—";
  if (activeLocale === "en-US") {
    return new Intl.NumberFormat("en-US", {
      notation: "compact",
      maximumFractionDigits: 2,
    }).format(value);
  }
  if (value >= 100_000_000) {
    const scaled = value / 100_000_000;
    return `${new Intl.NumberFormat("zh-CN", {
      minimumFractionDigits: scaled >= 100 ? 1 : 2,
      maximumFractionDigits: scaled >= 100 ? 1 : 2,
    }).format(scaled)} 亿`;
  }
  if (value >= 10_000) {
    const scaled = value / 10_000;
    return `${new Intl.NumberFormat("zh-CN", {
      minimumFractionDigits: scaled >= 1_000 ? 1 : 2,
      maximumFractionDigits: scaled >= 1_000 ? 1 : 2,
    }).format(scaled)} 万`;
  }
  return exactNumber().format(value);
}

export function formatExactTokens(value: number | null | undefined): string {
  if (value == null)
    return activeLocale === "zh-CN" ? "数据不可用" : "Unavailable";
  return exactNumber().format(value);
}

export function formatReadableUsd(
  value: number | null | undefined,
): string {
  if (value == null || !Number.isFinite(value)) return "—";
  if (value >= 1_000_000) {
    return `$${new Intl.NumberFormat("en-US", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    }).format(value / 1_000_000)}M`;
  }
  if (value >= 10_000) {
    return `$${new Intl.NumberFormat("en-US", {
      minimumFractionDigits: 1,
      maximumFractionDigits: 1,
    }).format(value / 1_000)}K`;
  }
  if (value > 0 && value < 0.01) {
    return `$${value.toFixed(4)}`;
  }
  return usdNumber.format(value);
}

export function formatExactUsd(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value))
    return activeLocale === "zh-CN" ? "金额不可用" : "Amount unavailable";
  if (value > 0 && value < 0.01) return `$${value.toFixed(6)}`;
  return usdNumber.format(value);
}

export function formatPlan(plan: string | null | undefined): string {
  if (!plan) return activeLocale === "zh-CN" ? "未知套餐" : "Unknown plan";
  const labels: Record<string, string> = {
    free: "Free",
    go: "Go",
    plus: "Plus",
    pro: "Pro",
    prolite: "Pro Lite",
    team: "Team",
    business: "Business",
    self_serve_business_usage_based: "Business",
    enterprise: "Enterprise",
    enterprise_cbp_usage_based: "Enterprise",
    edu: "Edu",
  };
  return labels[plan] ?? plan;
}

export function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null) return "—";
  const totalMinutes = Math.round(seconds / 60);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (activeLocale === "en-US") {
    if (hours > 0) return `${hours}h ${minutes}m`;
    return `${minutes} min`;
  }
  if (hours > 0) return `${hours}小时${minutes}分`;
  return `${minutes} 分钟`;
}

export function formatWindowLabel(minutes: number | null): string {
  if (activeLocale === "en-US") {
    if (minutes == null) return "Quota window";
    if (minutes === 300) return "5 hours";
    if (minutes === 10_080) return "Weekly";
    if (minutes < 60) return `${minutes} min`;
    if (minutes % 1_440 === 0) return `${minutes / 1_440} days`;
    return `${Math.round(minutes / 60)} hours`;
  }
  if (minutes == null) return "额度窗口";
  if (minutes === 300) return "5 小时";
  if (minutes === 10_080) return "每周";
  if (minutes < 60) return `${minutes} 分钟`;
  if (minutes % 1_440 === 0) return `${minutes / 1_440} 天`;
  return `${Math.round(minutes / 60)} 小时`;
}

export function localDateKey(date = new Date()): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function formatSyncTime(value: string): string {
  return new Intl.DateTimeFormat(activeLocale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
}

export function formatChartDate(value: string): string {
  const [, month, day] = value.split("-");
  return `${Number(month)}/${Number(day)}`;
}

export function countdownLabel(
  resetsAt: number | null,
  nowMs: number,
): string {
  if (resetsAt == null)
    return activeLocale === "zh-CN"
      ? "未提供重置时间"
      : "Reset time unavailable";
  const remaining = Math.max(0, resetsAt * 1_000 - nowMs);
  if (remaining === 0)
    return activeLocale === "zh-CN" ? "即将重置" : "Resetting soon";
  const totalMinutes = Math.floor(remaining / 60_000);
  const days = Math.floor(totalMinutes / 1_440);
  const hours = Math.floor((totalMinutes % 1_440) / 60);
  const minutes = totalMinutes % 60;
  if (activeLocale === "en-US") {
    if (days > 0) return `Resets in ${days}d ${hours}h`;
    if (hours > 0) return `Resets in ${hours}h ${minutes}m`;
    return `Resets in ${minutes}m`;
  }
  if (days > 0) return `${days} 天 ${hours} 小时后重置`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分后重置`;
  return `${minutes} 分钟后重置`;
}
