import { invoke } from "@tauri-apps/api/core";
import { ArrowRight, RefreshCw, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ComponentProps } from "react";
import type { Locale } from "./i18n";
import type {
  CodexSettings,
  SettingsApplyRequest,
  SettingsSnapshot,
} from "./types";

export type ConfigCategory =
  | "model"
  | "permissions"
  | "context"
  | "capabilities";

type ConfigPanelProps = {
  category: ConfigCategory;
  locale: Locale;
};

type SelectOption = {
  value: string;
  zh: string;
  en: string;
  recommended?: boolean;
};

type ChangeEntry = {
  key: keyof CodexSettings;
  label: string;
  before: string;
  after: string;
};

const CATEGORY_COPY: Record<
  ConfigCategory,
  { eyebrow: string; zh: string; en: string; descZh: string; descEn: string }
> = {
  model: {
    eyebrow: "MODEL BEHAVIOR",
    zh: "模型怎么思考和回答",
    en: "How the model thinks and responds",
    descZh: "这些设置影响速度、Token 消耗、回答长度和沟通风格。",
    descEn: "These settings affect speed, token use, answer length, and tone.",
  },
  permissions: {
    eyebrow: "SAFETY & PERMISSIONS",
    zh: "权限与确认",
    en: "Permissions & confirmation",
    descZh: "控制 Codex 能改什么、什么时候必须停下来问你。",
    descEn: "Control what Codex may change and when it must ask first.",
  },
  context: {
    eyebrow: "CONTEXT & MEMORY",
    zh: "上下文、历史与记忆",
    en: "Context, history & memory",
    descZh: "控制会话如何保存、何时压缩，以及旧对话能否成为未来记忆。",
    descEn: "Control transcript storage, compaction, and cross-thread memory.",
  },
  capabilities: {
    eyebrow: "TOOLS & FEATURES",
    zh: "工具与能力",
    en: "Tools & capabilities",
    descZh: "打开或关闭联网、子 Agent、Hooks、Apps 和持久目标等能力。",
    descEn: "Enable or disable search, subagents, hooks, apps, and persistent goals.",
  },
};

const REASONING_OPTIONS: SelectOption[] = [
  { value: "", zh: "跟随模型默认", en: "Model default", recommended: true },
  { value: "minimal", zh: "极少 · 最快", en: "Minimal · fastest" },
  { value: "low", zh: "低", en: "Low" },
  { value: "medium", zh: "中等", en: "Medium" },
  { value: "high", zh: "高", en: "High" },
  { value: "xhigh", zh: "极高 · 最慢", en: "Extra high · slowest" },
];

const PLAN_REASONING_OPTIONS: SelectOption[] = [
  { value: "", zh: "使用计划模式默认值", en: "Plan mode default", recommended: true },
  { value: "none", zh: "不额外推理", en: "None" },
  ...REASONING_OPTIONS.slice(1),
];

const SUMMARY_OPTIONS: SelectOption[] = [
  { value: "", zh: "跟随模型默认", en: "Model default", recommended: true },
  { value: "auto", zh: "自动", en: "Automatic" },
  { value: "concise", zh: "简短摘要", en: "Concise summary" },
  { value: "detailed", zh: "详细摘要", en: "Detailed summary" },
  { value: "none", zh: "不显示摘要", en: "No summary" },
];

const VERBOSITY_OPTIONS: SelectOption[] = [
  { value: "", zh: "跟随模型默认", en: "Model default", recommended: true },
  { value: "low", zh: "简洁", en: "Concise" },
  { value: "medium", zh: "适中", en: "Balanced" },
  { value: "high", zh: "详细", en: "Detailed" },
];

const PERSONALITY_OPTIONS: SelectOption[] = [
  { value: "", zh: "跟随 Codex 默认", en: "Codex default", recommended: true },
  { value: "none", zh: "中性", en: "Neutral" },
  { value: "friendly", zh: "友好", en: "Friendly" },
  { value: "pragmatic", zh: "务实直接", en: "Pragmatic" },
];

const APPROVAL_OPTIONS: SelectOption[] = [
  {
    value: "on-request",
    zh: "需要时问我",
    en: "Ask when needed",
    recommended: true,
  },
  { value: "untrusted", zh: "只信任安全命令", en: "Trusted commands only" },
  { value: "never", zh: "从不询问", en: "Never ask" },
  { value: "granular", zh: "精细规则（高级）", en: "Granular rules (advanced)" },
];

const REVIEWER_OPTIONS: SelectOption[] = [
  { value: "user", zh: "由我确认", en: "I review", recommended: true },
  { value: "auto_review", zh: "交给自动审查", en: "Automatic reviewer" },
];

const SANDBOX_OPTIONS: SelectOption[] = [
  { value: "read-only", zh: "只读", en: "Read only" },
  {
    value: "workspace-write",
    zh: "只允许工作区写入",
    en: "Workspace write",
    recommended: true,
  },
  {
    value: "danger-full-access",
    zh: "完全访问",
    en: "Full access",
  },
];

const WEB_OPTIONS: SelectOption[] = [
  { value: "disabled", zh: "关闭", en: "Off" },
  {
    value: "cached",
    zh: "缓存索引 · 更安全",
    en: "Cached index · safer",
    recommended: true,
  },
  { value: "indexed", zh: "索引联网", en: "Indexed web access" },
  { value: "live", zh: "实时联网", en: "Live web" },
];

const HISTORY_OPTIONS: SelectOption[] = [
  {
    value: "save-all",
    zh: "保存会话历史",
    en: "Save transcripts",
    recommended: true,
  },
  { value: "none", zh: "不保存", en: "Do not save" },
];

const COMPACT_SCOPE_OPTIONS: SelectOption[] = [
  {
    value: "total",
    zh: "按完整上下文计算",
    en: "Count full context",
    recommended: true,
  },
  {
    value: "body_after_prefix",
    zh: "只计算压缩前缀后的增长",
    en: "Count growth after compacted prefix",
  },
];

const LABELS: Record<keyof CodexSettings, { zh: string; en: string }> = {
  model_reasoning_effort: { zh: "默认推理强度", en: "Reasoning effort" },
  plan_mode_reasoning_effort: { zh: "计划模式推理", en: "Plan reasoning" },
  model_reasoning_summary: { zh: "推理摘要", en: "Reasoning summary" },
  model_verbosity: { zh: "回答详细度", en: "Response detail" },
  personality: { zh: "沟通风格", en: "Personality" },
  approval_policy: { zh: "审批方式", en: "Approval policy" },
  approvals_reviewer: { zh: "谁来审批", en: "Approval reviewer" },
  sandbox_mode: { zh: "文件访问范围", en: "Filesystem access" },
  workspace_network_access: { zh: "工作区命令联网", en: "Workspace network" },
  web_search: { zh: "联网搜索", en: "Web search" },
  history_persistence: { zh: "会话历史", en: "Transcript history" },
  auto_compact_token_limit: { zh: "自动压缩阈值", en: "Auto compact threshold" },
  auto_compact_scope: { zh: "压缩计数范围", en: "Compaction scope" },
  memories_enabled: { zh: "启用 Memories", en: "Enable Memories" },
  memories_use: { zh: "在新会话使用记忆", en: "Use memories" },
  memories_generate: { zh: "从旧会话生成记忆", en: "Generate memories" },
  memories_disable_on_external_context: {
    zh: "外部上下文会话不生成记忆",
    en: "Skip external-context threads",
  },
  multi_agent_enabled: { zh: "子 Agent 协作", en: "Subagent collaboration" },
  goals_enabled: { zh: "持久目标", en: "Persistent goals" },
  hooks_enabled: { zh: "生命周期 Hooks", en: "Lifecycle hooks" },
  unified_exec_enabled: { zh: "统一终端执行器", en: "Unified terminal runner" },
  fast_mode_enabled: { zh: "Fast 模式入口", en: "Fast mode controls" },
  apps_enabled: { zh: "Apps 与连接器", en: "Apps & connectors" },
  hide_agent_reasoning: { zh: "隐藏推理摘要", en: "Hide reasoning summaries" },
  show_raw_agent_reasoning: { zh: "显示原始推理", en: "Show raw reasoning" },
};

function valueText(
  key: keyof CodexSettings,
  value: CodexSettings[keyof CodexSettings],
  zh: boolean,
) {
  if (value == null || value === "") return zh ? "跟随默认" : "Default";
  if (typeof value === "boolean") return value ? (zh ? "开启" : "On") : zh ? "关闭" : "Off";
  if (typeof value === "number") return value.toLocaleString();
  const optionsByKey: Partial<Record<keyof CodexSettings, SelectOption[]>> = {
    model_reasoning_effort: REASONING_OPTIONS,
    plan_mode_reasoning_effort: PLAN_REASONING_OPTIONS,
    model_reasoning_summary: SUMMARY_OPTIONS,
    model_verbosity: VERBOSITY_OPTIONS,
    personality: PERSONALITY_OPTIONS,
    approval_policy: APPROVAL_OPTIONS,
    approvals_reviewer: REVIEWER_OPTIONS,
    sandbox_mode: SANDBOX_OPTIONS,
    web_search: WEB_OPTIONS,
    history_persistence: HISTORY_OPTIONS,
    auto_compact_scope: COMPACT_SCOPE_OPTIONS,
  };
  const lookup = optionsByKey[key]?.find((option) => option.value === value);
  return lookup ? (zh ? lookup.zh : lookup.en) : String(value);
}

function SelectSetting({
  label,
  description,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  description: string;
  value: string | null;
  options: SelectOption[];
  disabled?: boolean;
  onChange: (value: string | null) => void;
}) {
  return (
    <label className={`config-row${disabled ? " disabled" : ""}`}>
      <span className="config-row-copy">
        <strong>{label}</strong>
        <small>{description}</small>
      </span>
      <span className="config-select-wrap">
        <select
          value={value ?? ""}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value || null)}
        >
          {options.map((option) => (
            <option key={option.value || "default"} value={option.value}>
              {option.recommended ? "✓ " : ""}
              {option.zh}
            </option>
          ))}
        </select>
      </span>
    </label>
  );
}

function LocalizedSelectSetting({
  locale,
  ...props
}: Omit<ComponentProps<typeof SelectSetting>, "options"> & {
  locale: Locale;
  options: SelectOption[];
}) {
  const options = props.options.map((option) => ({
    ...option,
    zh: locale === "zh-CN" ? option.zh : option.en,
    en: locale === "zh-CN" ? option.zh : option.en,
  }));
  return <SelectSetting {...props} options={options} />;
}

function ToggleSetting({
  label,
  description,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className={`config-row${disabled ? " disabled" : ""}`}>
      <span className="config-row-copy">
        <strong>{label}</strong>
        <small>{description}</small>
      </span>
      <input
        className="config-switch"
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
    </label>
  );
}

export default function ConfigPanel({ category, locale }: ConfigPanelProps) {
  const zh = locale === "zh-CN";
  const [snapshot, setSnapshot] = useState<SettingsSnapshot | null>(null);
  const [draft, setDraft] = useState<CodexSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [preview, setPreview] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await invoke<SettingsSnapshot>("get_codex_settings");
      setSnapshot(next);
      setDraft(next.settings);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  useEffect(() => {
    setPreview(false);
  }, [category]);

  useEffect(() => {
    if (!preview) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !saving) setPreview(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [preview, saving]);

  const changes = useMemo<ChangeEntry[]>(() => {
    if (!snapshot || !draft) return [];
    return (Object.keys(draft) as (keyof CodexSettings)[])
      .filter(
        (key) =>
          JSON.stringify(snapshot.settings[key]) !== JSON.stringify(draft[key]),
      )
      .map((key) => ({
        key,
        label: zh ? LABELS[key].zh : LABELS[key].en,
        before: valueText(key, snapshot.settings[key], zh),
        after: valueText(key, draft[key], zh),
      }));
  }, [draft, snapshot, zh]);

  const update = <K extends keyof CodexSettings>(
    key: K,
    value: CodexSettings[K],
  ) => {
    setDraft((current) => (current ? { ...current, [key]: value } : current));
    setNotice(null);
  };

  const apply = async () => {
    if (!draft || !snapshot) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const request: SettingsApplyRequest = {
        settings: draft,
        expected_version: snapshot.version,
      };
      const next = await invoke<SettingsSnapshot>("apply_codex_settings", {
        request,
      });
      setSnapshot(next);
      setDraft(next.settings);
      setPreview(false);
      setNotice(
        zh
          ? "设置已通过 Codex 官方配置接口保存。运行中的任务可能保持旧值，新任务会使用新设置。"
          : "Saved through the official Codex config API. Active tasks may retain old values; new tasks use the update.",
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  const restore = async () => {
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const next = await invoke<SettingsSnapshot>("restore_codex_settings");
      setSnapshot(next);
      setDraft(next.settings);
      setPreview(false);
      setNotice(
        zh
          ? "已恢复上一次设置；刚刚替换的设置也已保留，可再次恢复回来。"
          : "Previous settings restored. The replaced values are retained for redo.",
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  if (loading && !draft) {
    return (
      <section className="config-loading" aria-live="polite">
        <span className="config-loading-line" />
        <span className="config-loading-line short" />
        <span className="config-loading-block" />
      </section>
    );
  }

  if (!draft) {
    return (
      <section className="config-empty">
        <strong>{zh ? "无法读取 Codex 设置" : "Unable to read Codex settings"}</strong>
        <p>{error}</p>
        <button className="primary-action" onClick={() => void load()}>
          {zh ? "重新读取" : "Try again"}
        </button>
      </section>
    );
  }

  const copy = CATEGORY_COPY[category];
  const riskNotesByCategory: Record<ConfigCategory, string[]> = {
    permissions: [
      draft.sandbox_mode === "danger-full-access" &&
        (zh
          ? "完全访问允许命令修改工作区之外的文件。"
          : "Full access lets commands modify files outside the workspace."),
      draft.approval_policy === "never" &&
        (zh
          ? "“从不询问”会跳过交互确认，但不会绕过系统安全策略。"
          : "Never ask skips interactive confirmation but not system safety policy."),
    ].filter(Boolean) as string[],
    context: [
      draft.history_persistence === "none" &&
        (zh
          ? "关闭历史后，用量与成本、执行追踪将缺少后续 Session 数据。"
          : "Disabling history removes future session evidence from usage and execution views."),
    ].filter(Boolean) as string[],
    capabilities: [
      draft.web_search === "live" &&
        (zh
          ? "实时联网会访问外部网站，结果和隐私边界不同于缓存索引。"
          : "Live search accesses external sites and has a different privacy boundary."),
    ].filter(Boolean) as string[],
    model: [
      draft.show_raw_agent_reasoning &&
        (zh
          ? "只有模型实际返回原始推理时才会显示；多数模型不会提供。"
          : "Raw reasoning appears only when the model emits it; most models do not."),
    ].filter(Boolean) as string[],
  };
  const riskNotes = Object.values(riskNotesByCategory).flat();
  const visibleRiskNotes = riskNotesByCategory[category];

  return (
    <section
      className={`config-panel${changes.length > 0 ? " has-unsaved-changes" : ""}`}
    >
      <header className="config-panel-header">
        <div>
          <h2>{zh ? copy.zh : copy.en}</h2>
          <span>{zh ? copy.descZh : copy.descEn}</span>
        </div>
        <div className="config-panel-actions">
          <button
            className="text-button"
            disabled={!snapshot?.restore_available || saving}
            onClick={() => void restore()}
          >
            {zh ? "撤销上次保存" : "Undo last save"}
          </button>
          <button
            className={loading ? "refresh-button spinning" : "refresh-button"}
            onClick={() => void load()}
            disabled={loading || saving}
            aria-label={zh ? "重新读取配置" : "Reload config"}
          >
            <RefreshCw aria-hidden="true" />
          </button>
        </div>
      </header>

      {category === "model" && (
        <>
          <div className="config-group">
            <div className="config-group-heading">
              <strong>{zh ? "思考方式" : "Reasoning"}</strong>
              <span>{zh ? "越高通常越慢、Token 越多" : "Higher is usually slower and uses more tokens"}</span>
            </div>
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "默认推理强度" : "Default reasoning effort"}
              description={
                zh
                  ? "新任务使用；并非所有模型都支持全部档位。"
                  : "Used for new tasks; support varies by model."
              }
              value={draft.model_reasoning_effort}
              options={REASONING_OPTIONS}
              onChange={(value) => update("model_reasoning_effort", value)}
            />
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "计划模式推理" : "Plan mode reasoning"}
              description={
                zh
                  ? "只影响计划模式，可单独使用更高强度。"
                  : "Applies only in Plan mode and may be tuned separately."
              }
              value={draft.plan_mode_reasoning_effort}
              options={PLAN_REASONING_OPTIONS}
              onChange={(value) => update("plan_mode_reasoning_effort", value)}
            />
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "推理摘要" : "Reasoning summary"}
              description={
                zh
                  ? "这是可读摘要，不是模型隐藏的完整思维链。"
                  : "A readable summary, not the model's hidden chain of thought."
              }
              value={draft.model_reasoning_summary}
              options={SUMMARY_OPTIONS}
              onChange={(value) => update("model_reasoning_summary", value)}
            />
          </div>

          <div className="config-group">
            <div className="config-group-heading">
              <strong>{zh ? "表达方式" : "Response style"}</strong>
              <span>{zh ? "只改变默认表现，不替代任务中的明确要求" : "Defaults do not override explicit task instructions"}</span>
            </div>
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "回答详细度" : "Response detail"}
              description={
                zh
                  ? "控制默认输出篇幅；高详细度通常产生更多输出 Token。"
                  : "Controls default answer length; higher detail usually uses more output tokens."
              }
              value={draft.model_verbosity}
              options={VERBOSITY_OPTIONS}
              onChange={(value) => update("model_verbosity", value)}
            />
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "沟通风格" : "Personality"}
              description={
                zh
                  ? "仅支持声明了 Personality 能力的模型。"
                  : "Only applies to models that advertise personality support."
              }
              value={draft.personality}
              options={PERSONALITY_OPTIONS}
              onChange={(value) => update("personality", value)}
            />
            <ToggleSetting
              label={zh ? "隐藏推理摘要" : "Hide reasoning summaries"}
              description={
                zh
                  ? "界面不再展示模型返回的推理摘要。"
                  : "Suppress model-provided reasoning summaries in the UI."
              }
              checked={draft.hide_agent_reasoning}
              onChange={(checked) => {
                update("hide_agent_reasoning", checked);
                if (checked) update("show_raw_agent_reasoning", false);
              }}
            />
            <ToggleSetting
              label={zh ? "显示原始推理（高级）" : "Show raw reasoning (advanced)"}
              description={
                zh
                  ? "仅在模型明确返回时显示，不保证可用。"
                  : "Shown only when explicitly emitted by the model."
              }
              checked={draft.show_raw_agent_reasoning}
              onChange={(checked) => {
                update("show_raw_agent_reasoning", checked);
                if (checked) update("hide_agent_reasoning", false);
              }}
            />
          </div>
        </>
      )}

      {category === "permissions" && (
        <>
          <div className="config-preset-strip">
            <div>
              <strong>{zh ? "推荐：日常开发" : "Recommended: everyday development"}</strong>
              <span>
                {zh
                  ? "只写工作区，需要时由你确认。"
                  : "Write only in the workspace and ask when needed."}
              </span>
            </div>
            <button
              className="secondary-action"
              onClick={() =>
                setDraft({
                  ...draft,
                  sandbox_mode: "workspace-write",
                  approval_policy: "on-request",
                  approvals_reviewer: "user",
                  workspace_network_access: false,
                })
              }
            >
              {zh ? "使用推荐设置" : "Use recommended"}
            </button>
          </div>
          <div className="config-group">
            <div className="config-group-heading">
              <strong>{zh ? "执行边界" : "Execution boundary"}</strong>
              <span>{zh ? "沙箱决定能碰哪里，审批决定何时问你" : "Sandbox controls reach; approvals control interruptions"}</span>
            </div>
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "文件访问范围" : "Filesystem access"}
              description={
                zh
                  ? "工作区写入适合大多数编码任务；完全访问风险最高。"
                  : "Workspace write fits most coding tasks; full access carries the most risk."
              }
              value={draft.sandbox_mode}
              options={SANDBOX_OPTIONS}
              onChange={(value) => {
                if (!value) return;
                update("sandbox_mode", value);
                if (value !== "workspace-write") {
                  update("workspace_network_access", false);
                }
              }}
            />
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "审批方式" : "Approval policy"}
              description={
                zh
                  ? "“需要时问我”在自动执行和可控性之间更平衡。"
                  : "Ask when needed balances autonomy and control."
              }
              value={draft.approval_policy}
              options={APPROVAL_OPTIONS}
              disabled={draft.approval_policy === "granular"}
              onChange={(value) => value && update("approval_policy", value)}
            />
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "谁来处理审批" : "Who reviews approvals"}
              description={
                zh
                  ? "自动审查会由 reviewer Agent 判断，仍受沙箱约束。"
                  : "Automatic review uses a reviewer agent and remains sandboxed."
              }
              value={draft.approvals_reviewer}
              options={REVIEWER_OPTIONS}
              disabled={draft.approval_policy === "never"}
              onChange={(value) => value && update("approvals_reviewer", value)}
            />
            <ToggleSetting
              label={zh ? "允许工作区命令联网" : "Allow workspace commands to use network"}
              description={
                zh
                  ? "只在“工作区写入”沙箱下生效；联网搜索由另一项控制。"
                  : "Applies only to workspace-write; web search is controlled separately."
              }
              checked={draft.workspace_network_access}
              disabled={draft.sandbox_mode !== "workspace-write"}
              onChange={(checked) => update("workspace_network_access", checked)}
            />
          </div>
        </>
      )}

      {category === "context" && (
        <>
          <div className="config-group">
            <div className="config-group-heading">
              <strong>{zh ? "会话记录" : "Conversation records"}</strong>
              <span>{zh ? "完整统计与执行追踪依赖本地 Session" : "Complete usage and execution analysis requires local sessions"}</span>
            </div>
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "保存会话历史" : "Transcript history"}
              description={
                zh
                  ? "关闭后新会话不会进入完整历史，用量和 Timeline 会缺失。"
                  : "Turning this off removes future transcripts from usage and timeline analysis."
              }
              value={draft.history_persistence}
              options={HISTORY_OPTIONS}
              onChange={(value) => value && update("history_persistence", value)}
            />
          </div>

          <div className="config-group">
            <div className="config-group-heading">
              <strong>{zh ? "上下文压缩" : "Context compaction"}</strong>
              <span>{zh ? "留空阈值时由模型与 Codex 决定" : "Leave threshold empty to use model and Codex defaults"}</span>
            </div>
            <label className="config-row">
              <span className="config-row-copy">
                <strong>{zh ? "自动压缩阈值" : "Auto compact threshold"}</strong>
                <small>
                  {zh
                    ? "接近该 Token 数时总结旧上下文；不是额度限制。"
                    : "Summarizes older context near this token count; this is not a quota."}
                </small>
              </span>
              <span className="config-number-wrap">
                <input
                  type="number"
                  min={16000}
                  max={10000000}
                  step={1000}
                  value={draft.auto_compact_token_limit ?? ""}
                  placeholder={zh ? "模型默认" : "Model default"}
                  onChange={(event) =>
                    update(
                      "auto_compact_token_limit",
                      event.target.value ? Number(event.target.value) : null,
                    )
                  }
                />
                <em>Token</em>
              </span>
            </label>
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "阈值计算范围" : "Threshold scope"}
              description={
                zh
                  ? "通常按完整上下文计算；多次压缩后可只看新增部分。"
                  : "Usually count the full context; compacted threads may count only new growth."
              }
              value={draft.auto_compact_scope}
              options={COMPACT_SCOPE_OPTIONS}
              onChange={(value) => value && update("auto_compact_scope", value)}
            />
          </div>

          <div className="config-group">
            <div className="config-group-heading">
              <strong>Memories</strong>
              <span>{zh ? "跨会话复用的提炼知识，不等于当前会话历史" : "Distilled cross-thread knowledge, separate from transcript history"}</span>
            </div>
            <ToggleSetting
              label={zh ? "启用 Memories（实验性）" : "Enable Memories (experimental)"}
              description={
                zh
                  ? "允许 Codex 在后台运行本地记忆管线。"
                  : "Allows Codex to run the local memory pipeline in the background."
              }
              checked={draft.memories_enabled}
              onChange={(checked) => update("memories_enabled", checked)}
            />
            <ToggleSetting
              label={zh ? "在新会话使用已有记忆" : "Use existing memories in new tasks"}
              description={
                zh
                  ? "把合适的记忆作为开发者上下文注入未来会话。"
                  : "Injects relevant memories as developer context in future tasks."
              }
              checked={draft.memories_use}
              disabled={!draft.memories_enabled}
              onChange={(checked) => update("memories_use", checked)}
            />
            <ToggleSetting
              label={zh ? "从旧会话生成新记忆" : "Generate memories from older tasks"}
              description={
                zh
                  ? "后台提取会消耗一定额度，并只处理符合条件的空闲会话。"
                  : "Background extraction uses some quota and only processes eligible idle tasks."
              }
              checked={draft.memories_generate}
              disabled={!draft.memories_enabled}
              onChange={(checked) => update("memories_generate", checked)}
            />
            <ToggleSetting
              label={zh ? "含外部上下文的会话不生成记忆" : "Skip tasks with external context"}
              description={
                zh
                  ? "使用过 MCP、Web Search 等外部内容时跳过记忆生成。"
                  : "Skip memory generation for tasks that used MCP, web search, or similar context."
              }
              checked={draft.memories_disable_on_external_context}
              disabled={!draft.memories_enabled || !draft.memories_generate}
              onChange={(checked) =>
                update("memories_disable_on_external_context", checked)
              }
            />
          </div>
        </>
      )}

      {category === "capabilities" && (
        <>
          <div className="config-group">
            <div className="config-group-heading">
              <strong>{zh ? "联网能力" : "Networked tools"}</strong>
              <span>{zh ? "联网搜索和终端命令联网是两套独立权限" : "Web search and shell networking are separate controls"}</span>
            </div>
            <LocalizedSelectSetting
              locale={locale}
              label={zh ? "联网搜索" : "Web search"}
              description={
                zh
                  ? "缓存索引适合普通查询；实时联网会直接访问外部网站。"
                  : "Cached index fits ordinary lookup; live mode accesses external sites."
              }
              value={draft.web_search}
              options={WEB_OPTIONS}
              onChange={(value) => value && update("web_search", value)}
            />
            <ToggleSetting
              label={zh ? "Apps 与连接器" : "Apps & connectors"}
              description={
                zh
                  ? "允许安装的连接器向 Codex 提供私有数据和操作。"
                  : "Allows installed connectors to provide private data and actions."
              }
              checked={draft.apps_enabled}
              onChange={(checked) => update("apps_enabled", checked)}
            />
          </div>

          <div className="config-group">
            <div className="config-group-heading">
              <strong>{zh ? "执行能力" : "Execution features"}</strong>
              <span>{zh ? "关闭后新任务将看不到对应能力" : "New tasks will not expose disabled capabilities"}</span>
            </div>
            <ToggleSetting
              label={zh ? "子 Agent 协作" : "Subagent collaboration"}
              description={
                zh
                  ? "允许主任务拆分并行子任务；会产生独立 Token 消耗。"
                  : "Lets a task delegate parallel work; subagents have separate token use."
              }
              checked={draft.multi_agent_enabled}
              onChange={(checked) => update("multi_agent_enabled", checked)}
            />
            <ToggleSetting
              label={zh ? "持久目标" : "Persistent goals"}
              description={
                zh
                  ? "保存长任务目标并支持自动继续。"
                  : "Persists long-running objectives and supports automatic continuation."
              }
              checked={draft.goals_enabled}
              onChange={(checked) => update("goals_enabled", checked)}
            />
            <ToggleSetting
              label={zh ? "生命周期 Hooks" : "Lifecycle hooks"}
              description={
                zh
                  ? "允许在工具、命令、文件修改等生命周期执行已配置规则。"
                  : "Runs configured rules around tool, command, and file-edit lifecycle events."
              }
              checked={draft.hooks_enabled}
              onChange={(checked) => update("hooks_enabled", checked)}
            />
            <ToggleSetting
              label={zh ? "统一终端执行器" : "Unified terminal runner"}
              description={
                zh
                  ? "使用支持 PTY 和持续会话的统一 exec 工具。"
                  : "Uses the PTY-backed unified exec tool with persistent sessions."
              }
              checked={draft.unified_exec_enabled}
              onChange={(checked) => update("unified_exec_enabled", checked)}
            />
            <ToggleSetting
              label={zh ? "Fast 模式入口" : "Fast mode controls"}
              description={
                zh
                  ? "当模型支持时，在 Codex 中显示更快服务层级的选择。"
                  : "Shows faster service-tier controls when the model supports them."
              }
              checked={draft.fast_mode_enabled}
              onChange={(checked) => update("fast_mode_enabled", checked)}
            />
          </div>
        </>
      )}

      {visibleRiskNotes.length > 0 && (
        <div className="config-risk-notes config-current-warnings" role="note">
          <strong>{zh ? "当前设置需要留意" : "Current settings to review"}</strong>
          {visibleRiskNotes.map((note) => (
            <p key={note}>{note}</p>
          ))}
        </div>
      )}

      {snapshot?.warnings.length ? (
        <div className="config-inline-warning">
          {snapshot.warnings.map((warning) => (
            <p key={warning}>{warning}</p>
          ))}
        </div>
      ) : null}
      {error && <div className="provider-message error">{error}</div>}
      {notice && <div className="provider-message success">{notice}</div>}

      {changes.length > 0 && (
        <footer className="config-savebar dirty">
          <div>
            <strong>
              {zh
                ? `${changes.length} 项尚未保存`
                : `${changes.length} unsaved change${changes.length === 1 ? "" : "s"}`}
            </strong>
            <span title={snapshot?.config_path}>
              {zh
                ? "写入用户级配置，不修改项目文件"
                : "Writes user-level config; project files stay untouched"}
            </span>
          </div>
          <button
            className="text-button"
            disabled={saving}
            onClick={() => setDraft(snapshot?.settings ?? draft)}
          >
            {zh ? "放弃修改" : "Discard"}
          </button>
          <button
            className="primary-action"
            disabled={saving}
            onClick={() => setPreview(true)}
          >
            {zh ? "查看并保存" : "Review & save"}
          </button>
        </footer>
      )}

      {preview && (
        <div
          className="config-review-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.currentTarget === event.target && !saving) setPreview(false);
          }}
        >
          <section
            className="config-review"
            role="dialog"
            aria-modal="true"
            aria-labelledby="config-review-title"
          >
            <header>
              <div>
                <h2 id="config-review-title">
                  {zh ? "确认这些变化" : "Confirm these changes"}
                </h2>
              </div>
              <button
                className="dialog-close"
                aria-label={zh ? "关闭" : "Close"}
                autoFocus
                disabled={saving}
                onClick={() => setPreview(false)}
              >
                <X aria-hidden="true" />
              </button>
            </header>
            <div className="config-change-list">
              {changes.map((change) => (
                <div key={change.key}>
                  <strong>{change.label}</strong>
                  <span>{change.before}</span>
                  <ArrowRight aria-hidden="true" />
                  <span>{change.after}</span>
                </div>
              ))}
            </div>
            {riskNotes.length > 0 && (
              <div className="config-risk-notes">
                <strong>{zh ? "需要知道" : "Before you save"}</strong>
                {riskNotes.map((note) => (
                  <p key={note}>{note}</p>
                ))}
              </div>
            )}
            <footer>
              <span>
                {zh
                  ? "通过 config/batchWrite 原子写入，并保存一次撤销点。"
                  : "Written atomically through config/batchWrite with one undo point."}
              </span>
              <button
                className="text-button"
                disabled={saving}
                onClick={() => setPreview(false)}
              >
                {zh ? "返回修改" : "Back"}
              </button>
              <button
                className="primary-action"
                disabled={saving}
                onClick={() => void apply()}
              >
                {saving ? (zh ? "正在保存…" : "Saving…") : zh ? "确认保存" : "Save settings"}
              </button>
            </footer>
          </section>
        </div>
      )}
    </section>
  );
}
