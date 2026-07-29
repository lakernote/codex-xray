import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  Copy,
  FolderOpen,
  LoaderCircle,
  RefreshCw,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import ExtensionUsageView from "./ExtensionUsageView";
import type { Locale } from "./i18n";
import type { EnvironmentSnapshot } from "./types";

type ConsoleSection = "provider" | "permissions" | "context" | "capabilities";

type Props = {
  locale: Locale;
  onOpenTrace: () => void;
  onOpenConsole: (section: ConsoleSection) => void;
};

type EnvironmentTab = "diagnosis" | "extensions";
type CheckTone = "good" | "warning" | "neutral";

function copy(locale: Locale, zh: string, en: string): string {
  return locale === "zh-CN" ? zh : en;
}

function stateLabel(locale: Locale, value: boolean): string {
  return value
    ? copy(locale, "已开启", "Enabled")
    : copy(locale, "未开启", "Disabled");
}

function shortVersion(value: string): string {
  return value.replace(/^codex-cli\s*/i, "").trim() || value;
}

function formatBytes(bytes: number, locale: Locale): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = units[0];
  for (let index = 1; index < units.length && value >= 1024; index += 1) {
    value /= 1024;
    unit = units[index];
  }
  return `${new Intl.NumberFormat(locale, {
    maximumFractionDigits: value >= 10 ? 1 : 2,
  }).format(value)} ${unit}`;
}

function DiagnosticCheck({
  title,
  status,
  detail,
  tone,
  action,
}: {
  title: string;
  status: string;
  detail: string;
  tone: CheckTone;
  action?: { label: string; onClick: () => void };
}) {
  return (
    <div className={`diagnostic-check ${tone}`}>
      <span className="diagnostic-check-dot" aria-hidden="true" />
      <div>
        <strong>{title}</strong>
        <small>{detail}</small>
      </div>
      <b>{status}</b>
      {action ? (
        <button className="row-action" onClick={action.onClick}>
          {action.label}
        </button>
      ) : (
        <span className="diagnostic-action-placeholder" />
      )}
    </div>
  );
}

function EnvironmentRow({
  label,
  value,
  detail,
  mono = false,
  tone,
  actions,
}: {
  label: string;
  value: string;
  detail?: string;
  mono?: boolean;
  tone?: "good" | "warning";
  actions?: React.ReactNode;
}) {
  return (
    <div className="environment-row">
      <span>{label}</span>
      <div>
        <strong className={`${mono ? "mono" : ""}${tone ? ` ${tone}` : ""}`}>
          {value}
        </strong>
        {detail && <small>{detail}</small>}
      </div>
      {actions && <div className="environment-row-actions">{actions}</div>}
    </div>
  );
}

export default function EnvironmentView({
  locale,
  onOpenTrace,
  onOpenConsole,
}: Props) {
  const [snapshot, setSnapshot] = useState<EnvironmentSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<EnvironmentTab>("diagnosis");
  const [extensionRefreshSignal, setExtensionRefreshSignal] = useState(0);
  const [copiedPath, setCopiedPath] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setSnapshot(await invoke<EnvironmentSnapshot>("get_environment_snapshot"));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const activeMcpCount = useMemo(
    () => snapshot?.mcp_servers.filter((server) => server.enabled).length ?? 0,
    [snapshot],
  );

  const copyPath = async (path: string) => {
    try {
      await navigator.clipboard.writeText(path);
      setCopiedPath(path);
      window.setTimeout(
        () => setCopiedPath((current) => (current === path ? null : current)),
        1400,
      );
    } catch (cause) {
      setError(
        copy(
          locale,
          `无法复制路径：${String(cause)}`,
          `Unable to copy path: ${String(cause)}`,
        ),
      );
    }
  };

  const revealPath = async (path: string) => {
    try {
      setError(null);
      await invoke("reveal_local_path", { path });
    } catch (cause) {
      setError(String(cause));
    }
  };

  const pathActions = (path: string) => (
    <>
      <button
        title={copy(locale, "在 Finder 中显示", "Reveal in file manager")}
        aria-label={copy(locale, `显示 ${path}`, `Reveal ${path}`)}
        onClick={() => void revealPath(path)}
      >
        <FolderOpen aria-hidden="true" />
      </button>
      <button
        title={copy(locale, "复制路径", "Copy path")}
        aria-label={copy(locale, `复制 ${path}`, `Copy ${path}`)}
        onClick={() => void copyPath(path)}
      >
        {copiedPath === path ? (
          <Check aria-hidden="true" />
        ) : (
          <Copy aria-hidden="true" />
        )}
      </button>
    </>
  );

  if (!snapshot && loading) {
    return (
      <main className="workspace environment-workspace">
        <div className="environment-loading" aria-live="polite">
          <LoaderCircle className="standard-loader" aria-hidden="true" />
          <strong>
            {copy(locale, "正在检查 Codex 环境", "Checking Codex environment")}
          </strong>
          <small>
            {copy(
              locale,
              "读取官方合并配置和本地分析数据库。",
              "Reading merged Codex config and the local analysis database.",
            )}
          </small>
        </div>
      </main>
    );
  }

  if (!snapshot) {
    return (
      <main className="workspace environment-workspace">
        <div className="environment-error" role="alert">
          <h1>
            {copy(locale, "无法读取 Codex 环境", "Codex environment unavailable")}
          </h1>
          <p>{error}</p>
          <button className="primary-action" onClick={() => void load()}>
            {copy(locale, "重新检查", "Try again")}
          </button>
        </div>
      </main>
    );
  }

  const settings = snapshot.settings;
  const providerConfigured =
    snapshot.provider.compatibility === "responses" &&
    (!snapshot.provider.credential_variable ||
      snapshot.provider.credential_available);
  const protectedMode = !(
    settings.sandbox_mode === "danger-full-access" &&
    settings.approval_policy === "never"
  );
  const sessionsReady = !snapshot.warnings.some(
    (warning) =>
      warning.includes("CODEX_HOME") ||
      warning.includes("sessions directory") ||
      warning.includes("history persistence"),
  );
  const storageReady =
    snapshot.storage.integrity_ok &&
    snapshot.storage.foreign_key_violations === 0;
  const issueCount = [
    !sessionsReady,
    !storageReady,
    !providerConfigured,
    !protectedMode,
    snapshot.storage.malformed_session_lines > 0,
  ].filter(Boolean).length;

  return (
    <main className="workspace environment-workspace">
      <header className="topbar environment-topbar">
        <div>
          <h1>
            {activeTab === "diagnosis"
              ? copy(locale, "环境诊断", "Environment")
              : copy(locale, "扩展调用", "Extension calls")}
          </h1>
          <span className="header-note">
            {activeTab === "diagnosis"
              ? copy(
                  locale,
                  "检查本地数据、Provider、权限和扩展配置",
                  "Check local data, provider, permissions, and extension config",
                )
              : copy(
                  locale,
                  "按真实执行记录统计 MCP、Skill、CLI 与工具调用",
                  "Observed MCP, Skill, CLI, and tool calls",
                )}
          </span>
        </div>
        <div className="topbar-actions">
          <span>
            {activeTab === "diagnosis"
              ? new Date(snapshot.fetched_at).toLocaleTimeString(locale, {
                  hour: "2-digit",
                  minute: "2-digit",
                  second: "2-digit",
                })
              : copy(locale, "已分析记录", "Analyzed records")}
          </span>
          <button
            className={loading ? "refresh-button spinning" : "refresh-button"}
            onClick={() => {
              if (activeTab === "diagnosis") {
                void load();
              } else {
                setExtensionRefreshSignal((value) => value + 1);
              }
            }}
            disabled={activeTab === "diagnosis" && loading}
            aria-label={
              activeTab === "diagnosis"
                ? copy(locale, "重新检查", "Refresh diagnosis")
                : copy(locale, "刷新扩展调用", "Refresh extension usage")
            }
          >
            <RefreshCw aria-hidden="true" />
          </button>
        </div>
      </header>

      <div
        className="environment-tabs"
        role="tablist"
        aria-label={copy(locale, "环境视图", "Environment views")}
      >
        <button
          role="tab"
          aria-selected={activeTab === "diagnosis"}
          className={activeTab === "diagnosis" ? "selected" : ""}
          onClick={() => setActiveTab("diagnosis")}
        >
          {copy(locale, "检查结果", "Checks")}
        </button>
        <button
          role="tab"
          aria-selected={activeTab === "extensions"}
          className={activeTab === "extensions" ? "selected" : ""}
          onClick={() => setActiveTab("extensions")}
        >
          {copy(locale, "扩展调用", "Extension calls")}
        </button>
      </div>

      {activeTab === "diagnosis" ? (
        <>
          <section className="diagnostic-summary">
            <div>
              <strong>
                {issueCount === 0
                  ? copy(locale, "检查通过", "All checks passed")
                  : copy(
                      locale,
                      `${issueCount} 项建议确认`,
                      `${issueCount} item${issueCount === 1 ? "" : "s"} to review`,
                    )}
              </strong>
              <span>
                Codex {shortVersion(snapshot.codex_version)} ·{" "}
                {copy(locale, "配置与数据库可读取", "Config and database readable")}
              </span>
            </div>
            <small>
              {copy(locale, "检测不会修改 Codex 配置", "Checks do not modify Codex config")}
            </small>
          </section>

          <section
            className="diagnostic-checks"
            aria-label={copy(locale, "诊断结果", "Diagnostic results")}
          >
            <DiagnosticCheck
              title={copy(locale, "本地会话与分析数据", "Local sessions and analysis")}
              status={
                sessionsReady && storageReady
                  ? copy(locale, "正常", "Ready")
                  : copy(locale, "不完整", "Incomplete")
              }
              detail={
                sessionsReady && storageReady
                  ? copy(
                      locale,
                      `${snapshot.storage.usage_sessions.toLocaleString()} 个会话 · ${snapshot.storage.usage_turns.toLocaleString()} 个回合 · SQLite quick_check 通过`,
                      `${snapshot.storage.usage_sessions.toLocaleString()} sessions · ${snapshot.storage.usage_turns.toLocaleString()} turns · SQLite quick_check passed`,
                    )
                  : copy(
                      locale,
                      `会话目录、历史保存或数据库完整性需要处理。${snapshot.storage.integrity_ok ? "" : ` ${snapshot.storage.integrity_message}`}`,
                      `Session storage, transcript history, or database integrity needs attention.${snapshot.storage.integrity_ok ? "" : ` ${snapshot.storage.integrity_message}`}`,
                    )
              }
              tone={sessionsReady && storageReady ? "good" : "warning"}
              action={{
                label: copy(locale, "查看执行", "Open execution"),
                onClick: onOpenTrace,
              }}
            />
            <DiagnosticCheck
              title={copy(locale, "模型供应商", "Model provider")}
              status={
                providerConfigured
                  ? copy(locale, "配置完整 · 未实测", "Configured · untested")
                  : copy(locale, "需处理", "Action needed")
              }
              detail={`${snapshot.provider.name} · ${
                snapshot.provider.model ?? copy(locale, "默认模型", "Default model")
              } · ${snapshot.provider.wire_api}`}
              tone={providerConfigured ? "neutral" : "warning"}
              action={{
                label: copy(locale, "打开供应商", "Open provider"),
                onClick: () => onOpenConsole("provider"),
              }}
            />
            <DiagnosticCheck
              title={copy(locale, "文件与审批边界", "Filesystem and approvals")}
              status={
                protectedMode
                  ? copy(locale, "有保护", "Protected")
                  : copy(locale, "完全放开", "Unrestricted")
              }
              detail={`${settings.sandbox_mode} · ${settings.approval_policy}`}
              tone={protectedMode ? "good" : "warning"}
              action={{
                label: copy(locale, "调整权限", "Review permissions"),
                onClick: () => onOpenConsole("permissions"),
              }}
            />
            <DiagnosticCheck
              title={copy(locale, "扩展配置", "Extension config")}
              status={copy(
                locale,
                `${activeMcpCount} 个 MCP 已启用`,
                `${activeMcpCount} MCP enabled`,
              )}
              detail={copy(
                locale,
                `共配置 ${snapshot.mcp_servers.length} 个 MCP；这里表示配置状态，不代表服务器已连接。`,
                `${snapshot.mcp_servers.length} MCP configured; this does not confirm live connectivity.`,
              )}
              tone="neutral"
              action={{
                label: copy(locale, "查看调用", "Open calls"),
                onClick: () => setActiveTab("extensions"),
              }}
            />
          </section>

          <div className="environment-columns">
            <section className="environment-section">
              <header>
                <div>
                  <h2>{copy(locale, "本地数据", "Local data")}</h2>
                </div>
                <span>{copy(locale, "可打开或复制路径", "Reveal or copy paths")}</span>
              </header>
              <div className="environment-rows">
                <EnvironmentRow
                  label="CODEX_HOME"
                  value={snapshot.codex_home}
                  mono
                  actions={pathActions(snapshot.codex_home)}
                />
                <EnvironmentRow
                  label={copy(locale, "用户配置", "User config")}
                  value={snapshot.config_path}
                  mono
                  actions={pathActions(snapshot.config_path)}
                />
                <EnvironmentRow
                  label={copy(locale, "会话目录", "Sessions")}
                  value={snapshot.sessions_path}
                  mono
                  actions={pathActions(snapshot.sessions_path)}
                />
                <EnvironmentRow
                  label={copy(locale, "分析数据库", "Analysis database")}
                  value={snapshot.xray_sqlite_path}
                  detail={`${snapshot.storage.journal_mode.toUpperCase()} · ${formatBytes(
                    snapshot.storage.database_bytes,
                    locale,
                  )}${
                    snapshot.storage.wal_bytes
                      ? ` + WAL ${formatBytes(snapshot.storage.wal_bytes, locale)}`
                      : ""
                  } · schema ${snapshot.storage.schema_version} · ${
                    snapshot.storage.integrity_ok
                      ? copy(locale, "完整性正常", "integrity OK")
                      : snapshot.storage.integrity_message
                  }`}
                  mono
                  actions={pathActions(snapshot.xray_sqlite_path)}
                />
                <EnvironmentRow
                  label={copy(locale, "索引完整性", "Index integrity")}
                  value={
                    snapshot.storage.integrity_ok &&
                    snapshot.storage.foreign_key_violations === 0
                      ? copy(locale, "检查通过", "Checks passed")
                      : copy(locale, "需要处理", "Action needed")
                  }
                  detail={copy(
                    locale,
                    `${snapshot.storage.foreign_key_violations.toLocaleString()} 个外键问题 · ${snapshot.storage.malformed_session_lines.toLocaleString()} 行 Session 无法解析`,
                    `${snapshot.storage.foreign_key_violations.toLocaleString()} foreign-key issues · ${snapshot.storage.malformed_session_lines.toLocaleString()} unparseable session lines`,
                  )}
                  tone={
                    snapshot.storage.integrity_ok &&
                    snapshot.storage.foreign_key_violations === 0 &&
                    snapshot.storage.malformed_session_lines === 0
                      ? "good"
                      : "warning"
                  }
                />
              </div>
            </section>

            <section className="environment-section">
              <header>
                <div>
                  <h2>{copy(locale, "当前配置", "Current config")}</h2>
                </div>
                <button
                  className="row-action"
                  onClick={() => onOpenConsole("context")}
                >
                  {copy(locale, "在控制台修改", "Edit in console")}
                </button>
              </header>
              <div className="environment-rows">
                <EnvironmentRow
                  label={copy(locale, "模型", "Model")}
                  value={
                    snapshot.provider.model ??
                    copy(locale, "使用默认模型", "Default model")
                  }
                  mono
                />
                <EnvironmentRow
                  label={copy(locale, "历史保存", "Transcript history")}
                  value={settings.history_persistence}
                  mono
                  tone={
                    settings.history_persistence === "none"
                      ? "warning"
                      : "good"
                  }
                />
                <EnvironmentRow
                  label={copy(locale, "网页搜索", "Web search")}
                  value={settings.web_search}
                  mono
                />
                <EnvironmentRow
                  label={copy(locale, "记忆 / 多 Agent / Apps", "Memory / multi-agent / apps")}
                  value={[
                    stateLabel(locale, settings.memories_enabled),
                    stateLabel(locale, settings.multi_agent_enabled),
                    stateLabel(locale, settings.apps_enabled),
                  ].join(" · ")}
                />
              </div>
            </section>
          </div>

          <details className="environment-technical">
            <summary>{copy(locale, "技术详情与扩展配置", "Technical details and extension config")}</summary>
            <div className="environment-technical-grid">
              <div>
                <EnvironmentRow label="Codex CLI" value={snapshot.codex_binary} mono />
                <EnvironmentRow
                  label={copy(locale, "X-Ray 数据目录", "X-Ray data")}
                  value={snapshot.xray_data_path}
                  mono
                  actions={pathActions(snapshot.xray_data_path)}
                />
                <EnvironmentRow
                  label="Config version"
                  value={snapshot.config_version ?? "—"}
                  mono
                />
                <EnvironmentRow
                  label="Provider endpoint"
                  value={snapshot.provider.endpoint ?? copy(locale, "内置", "Built in")}
                  mono
                />
              </div>
              <div>
                <h3>MCP</h3>
                {snapshot.mcp_servers.length ? (
                  <div className="extension-list">
                    {snapshot.mcp_servers.map((server) => (
                      <div key={server.name}>
                        <span
                          className={
                            server.enabled ? "status-dot" : "status-dot muted"
                          }
                        />
                        <strong>{server.name}</strong>
                        <small>{server.transport}</small>
                        <code>{server.target ?? "—"}</code>
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="empty-inline">
                    {copy(locale, "没有配置 MCP Server", "No MCP servers configured")}
                  </p>
                )}
                <h3>{copy(locale, "扩展目录", "Extension roots")}</h3>
                <div className="extension-list path-list">
                  {snapshot.extension_paths.map((path) => (
                    <div key={path.path}>
                      <span
                        className={path.exists ? "status-dot" : "status-dot muted"}
                      />
                      <strong>{path.label}</strong>
                      <small>
                        {path.item_count === null
                          ? copy(locale, "不可用", "Unavailable")
                          : copy(
                              locale,
                              `${path.item_count} 个目录`,
                              `${path.item_count} folders`,
                            )}
                      </small>
                      <code>{path.path}</code>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </details>

          {snapshot.warnings.length > 0 && (
            <details className="environment-raw-warnings">
              <summary>
                {copy(
                  locale,
                  `${snapshot.warnings.length} 条原始诊断信息`,
                  `${snapshot.warnings.length} raw diagnostic messages`,
                )}
              </summary>
              {snapshot.warnings.map((warning) => (
                <code key={warning}>{warning}</code>
              ))}
            </details>
          )}

          {error && (
            <div className="inline-error" role="alert">
              {error}
            </div>
          )}
        </>
      ) : (
        <ExtensionUsageView
          locale={locale}
          refreshSignal={extensionRefreshSignal}
          totalSessions={snapshot.storage.usage_sessions}
          onOpenTrace={onOpenTrace}
        />
      )}
    </main>
  );
}
