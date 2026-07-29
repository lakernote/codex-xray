import { invoke } from "@tauri-apps/api/core";
import {
  ArrowRight,
  CheckCircle2,
  ExternalLink,
  FlaskConical,
  RefreshCw,
  XCircle,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import ConfigPanel, { type ConfigCategory } from "./ConfigPanel";
import type { Locale } from "./i18n";
import type {
  ProviderApplyRequest,
  ProviderDefinition,
  ProviderSnapshot,
  ProviderTestResult,
} from "./types";

export type ControlSection = "provider" | ConfigCategory;

type ProviderViewProps = {
  locale: Locale;
  onOpenUrl: (url: string) => void;
  initialSection?: ControlSection;
  hidden?: boolean;
};

type Preset = {
  id: string;
  title: string;
  vendorZh: string;
  vendorEn: string;
  providerId: string;
  name: string;
  model: string;
  baseUrl: string;
  envKey: string;
  support: "official" | "native" | "hosted" | "custom";
  noteZh: string;
  noteEn: string;
  docsUrl: string;
};

type Draft = {
  providerId: string;
  name: string;
  model: string;
  baseUrl: string;
  envKey: string;
};

const PRESETS: Preset[] = [
  {
    id: "openai",
    title: "OpenAI",
    vendorZh: "Codex 官方",
    vendorEn: "Codex official",
    providerId: "openai",
    name: "OpenAI",
    model: "gpt-5.6-sol",
    baseUrl: "",
    envKey: "",
    support: "official",
    noteZh: "Codex 内置 Provider，使用当前 Codex 登录。",
    noteEn: "Built into Codex and uses the current Codex sign-in.",
    docsUrl: "https://learn.chatgpt.com/docs/config-file/config-reference",
  },
  {
    id: "qwen",
    title: "Qwen",
    vendorZh: "阿里云百炼",
    vendorEn: "Alibaba Model Studio",
    providerId: "dashscope",
    name: "Alibaba Model Studio",
    model: "qwen3-coder-plus",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    envKey: "DASHSCOPE_API_KEY",
    support: "native",
    noteZh: "官方 Responses；建议随后换成业务空间专属域名。",
    noteEn: "Native Responses; a workspace-specific endpoint is recommended.",
    docsUrl:
      "https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses",
  },
  {
    id: "doubao",
    title: "Doubao",
    vendorZh: "火山方舟",
    vendorEn: "Volcengine Ark",
    providerId: "volcengine",
    name: "Volcengine Ark",
    model: "doubao-seed-2-0-lite-260215",
    baseUrl: "https://ark.cn-beijing.volces.com/api/v3",
    envKey: "ARK_API_KEY",
    support: "native",
    noteZh: "官方 Responses，支持流式输出和函数工具。",
    noteEn: "Native Responses with streaming and function tools.",
    docsUrl: "https://www.volcengine.com/docs/82379/1795150",
  },
  {
    id: "qianfan-glm",
    title: "GLM",
    vendorZh: "百度千帆托管",
    vendorEn: "Hosted on Baidu Qianfan",
    providerId: "qianfan-glm",
    name: "Baidu Qianfan · GLM",
    model: "glm-5",
    baseUrl: "https://qianfan.baidubce.com/v2",
    envKey: "QIANFAN_API_KEY",
    support: "hosted",
    noteZh: "GLM 原厂接口偏 Chat；千帆把 GLM 暴露为 Responses。",
    noteEn: "Qianfan exposes hosted GLM models through Responses.",
    docsUrl: "https://cloud.baidu.com/doc/qianfan-docs/s/4mi400l1m",
  },
  {
    id: "qianfan-deepseek",
    title: "DeepSeek",
    vendorZh: "百度千帆托管",
    vendorEn: "Hosted on Baidu Qianfan",
    providerId: "qianfan-deepseek",
    name: "Baidu Qianfan · DeepSeek",
    model: "deepseek-v4-pro",
    baseUrl: "https://qianfan.baidubce.com/v2",
    envKey: "QIANFAN_API_KEY",
    support: "hosted",
    noteZh: "DeepSeek 原厂接口是 Chat；千帆提供 Responses 版本。",
    noteEn: "Qianfan exposes hosted DeepSeek models through Responses.",
    docsUrl: "https://cloud.baidu.com/doc/qianfan-docs/s/4mi400l1m",
  },
  {
    id: "minimax",
    title: "MiniMax",
    vendorZh: "MiniMax",
    vendorEn: "MiniMax",
    providerId: "minimax",
    name: "MiniMax",
    model: "MiniMax-M3",
    baseUrl: "https://api.minimaxi.com/v1",
    envKey: "MINIMAX_API_KEY",
    support: "native",
    noteZh: "官方 Responses，并有官方 Codex 桌面端接入文档。",
    noteEn: "Native Responses with an official Codex desktop guide.",
    docsUrl: "https://platform.minimaxi.com/docs/token-plan/codex",
  },
  {
    id: "stepfun",
    title: "StepFun",
    vendorZh: "阶跃星辰",
    vendorEn: "StepFun",
    providerId: "stepfun",
    name: "StepFun",
    model: "step-3.7-flash",
    baseUrl: "https://api.stepfun.com/v1",
    envKey: "STEP_API_KEY",
    support: "native",
    noteZh: "官方 Responses；当前文档仅列 step-3.7-flash。",
    noteEn: "Native Responses; currently documented for step-3.7-flash.",
    docsUrl:
      "https://platform.stepfun.com/docs/zh/api-reference/responses/responses-create",
  },
  {
    id: "custom",
    title: "Custom",
    vendorZh: "Responses 网关",
    vendorEn: "Responses gateway",
    providerId: "custom-responses",
    name: "Custom Responses",
    model: "",
    baseUrl: "",
    envKey: "MODEL_API_KEY",
    support: "custom",
    noteZh: "只接受真正实现 /responses 的地址，不接受仅 Chat 兼容。",
    noteEn: "Requires a real /responses implementation, not Chat-only compatibility.",
    docsUrl: "https://learn.chatgpt.com/docs/config-file/config-reference",
  },
];

function draftFromPreset(preset: Preset, currentModel?: string | null): Draft {
  return {
    providerId: preset.providerId,
    name: preset.name,
    model:
      preset.id === "openai" && currentModel ? currentModel : preset.model,
    baseUrl: preset.baseUrl,
    envKey: preset.envKey,
  };
}

function supportLabel(
  support: Preset["support"],
  locale: Locale,
): string {
  const zh = {
    official: "Codex 内置",
    native: "原生 Responses",
    hosted: "托管 Responses",
    custom: "需要验证",
  };
  const en = {
    official: "Built into Codex",
    native: "Native Responses",
    hosted: "Hosted Responses",
    custom: "Verify endpoint",
  };
  return (locale === "zh-CN" ? zh : en)[support];
}

function detectedStatus(
  provider: ProviderDefinition,
  locale: Locale,
): string {
  if (provider.compatibility === "chat_only") {
    return locale === "zh-CN" ? "仅 Chat，不能直连" : "Chat only";
  }
  if (provider.compatibility === "unsupported_wire_api") {
    return locale === "zh-CN" ? "协议不受支持" : "Unsupported protocol";
  }
  if (provider.env_key && !provider.env_available) {
    return locale === "zh-CN"
      ? `未检测到 ${provider.env_key}`
      : `${provider.env_key} not detected`;
  }
  return locale === "zh-CN"
    ? "已配置 Responses"
    : "Configured for Responses";
}

export default function ProviderView({
  locale,
  onOpenUrl,
  initialSection = "provider",
  hidden = false,
}: ProviderViewProps) {
  const [activeSection, setActiveSection] =
    useState<ControlSection>(initialSection);
  const [lastConfigSection, setLastConfigSection] = useState<ConfigCategory>(
    initialSection === "provider" ? "model" : initialSection,
  );
  const [settingsMounted, setSettingsMounted] = useState(
    initialSection !== "provider",
  );
  const [snapshot, setSnapshot] = useState<ProviderSnapshot | null>(null);
  const initializedFromConfig = useRef(false);
  const [selectedId, setSelectedId] = useState("openai");
  const selectedPreset =
    PRESETS.find((preset) => preset.id === selectedId) ?? PRESETS[0];
  const [draft, setDraft] = useState<Draft>(() =>
    draftFromPreset(PRESETS[0]),
  );
  const [preview, setPreview] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<ProviderTestResult | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const zh = locale === "zh-CN";
  const activeDefinition = useMemo(
    () =>
      snapshot?.providers.find(
        (provider) => provider.id === snapshot.active_provider,
      ) ?? null,
    [snapshot],
  );
  const editingBuiltin = Boolean(
    snapshot?.providers.find(
      (provider) => provider.id === draft.providerId.trim() && provider.builtin,
    ),
  );
  const modelSuggestions = Array.from(
    new Set(
      [
        ...(selectedPreset.id === "openai"
          ? (snapshot?.models.map((model) => model.id) ?? [])
          : []),
        selectedPreset.model,
        snapshot?.active_model ?? "",
      ].filter(Boolean),
    ),
  );

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await invoke<ProviderSnapshot>("get_provider_config");
      setSnapshot(next);
      if (!initializedFromConfig.current) {
        const matchedPreset = PRESETS.find(
          (preset) => preset.providerId === next.active_provider,
        );
        if (matchedPreset) {
          setSelectedId(matchedPreset.id);
          setDraft(draftFromPreset(matchedPreset, next.active_model));
        } else {
          const active = next.providers.find(
            (provider) => provider.id === next.active_provider,
          );
          if (active) {
            setSelectedId("custom");
            setDraft({
              providerId: active.id,
              name: active.name,
              model: next.active_model ?? "",
              baseUrl: active.base_url ?? "",
              envKey: active.env_key ?? "",
            });
          }
        }
        initializedFromConfig.current = true;
      }
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
    setTestResult(null);
  }, [draft]);

  useEffect(() => {
    setActiveSection(initialSection);
    if (initialSection !== "provider") {
      setLastConfigSection(initialSection);
      setSettingsMounted(true);
    }
  }, [initialSection]);

  const selectPreset = (preset: Preset) => {
    setSelectedId(preset.id);
    setDraft(draftFromPreset(preset, snapshot?.active_model));
    setPreview(false);
    setNotice(null);
    setError(null);
  };

  const selectDetected = (provider: ProviderDefinition) => {
    setSelectedId("custom");
    setDraft({
      providerId: provider.id,
      name: provider.name,
      model:
        provider.id === snapshot?.active_provider
          ? snapshot.active_model ?? ""
          : "",
      baseUrl: provider.base_url ?? "",
      envKey: provider.env_key ?? "",
    });
    setPreview(false);
    setNotice(null);
  };

  const request: ProviderApplyRequest = {
    provider_id: draft.providerId.trim(),
    model: draft.model.trim(),
    name: editingBuiltin ? null : draft.name.trim() || null,
    base_url: editingBuiltin ? null : draft.baseUrl.trim() || null,
    env_key: editingBuiltin ? null : draft.envKey.trim() || null,
    expected_version: snapshot?.version ?? null,
  };

  const configurationComplete =
    request.provider_id.length > 0 &&
    request.model.length > 0 &&
    (editingBuiltin || Boolean(request.base_url));
  const targetDefinition = snapshot?.providers.find(
    (provider) => provider.id === request.provider_id,
  );
  const hasProviderChanges = Boolean(
    snapshot &&
      (request.provider_id !== snapshot.active_provider ||
        request.model !== (snapshot.active_model ?? "") ||
        (!editingBuiltin &&
          (request.name !== (targetDefinition?.name ?? null) ||
            request.base_url !== (targetDefinition?.base_url ?? null) ||
            request.env_key !== (targetDefinition?.env_key ?? null)))),
  );
  const canPreview = configurationComplete && hasProviderChanges;

  const discardDraft = () => {
    if (!snapshot) return;
    const active = snapshot.providers.find(
      (provider) => provider.id === snapshot.active_provider,
    );
    const preset = PRESETS.find(
      (candidate) => candidate.providerId === snapshot.active_provider,
    );
    if (preset) {
      setSelectedId(preset.id);
      setDraft(draftFromPreset(preset, snapshot.active_model));
    } else if (active) {
      setSelectedId("custom");
      setDraft({
        providerId: active.id,
        name: active.name,
        model: snapshot.active_model ?? "",
        baseUrl: active.base_url ?? "",
        envKey: active.env_key ?? "",
      });
    }
    setPreview(false);
    setError(null);
    setNotice(null);
  };

  const openSection = (section: ControlSection) => {
    if (section !== "provider") {
      setLastConfigSection(section);
      setSettingsMounted(true);
    }
    setActiveSection(section);
  };

  const apply = async () => {
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const next = await invoke<ProviderSnapshot>("apply_provider_config", {
        request,
      });
      setSnapshot(next);
      setPreview(false);
      setNotice(
        zh
          ? "已通过 Codex 官方 config/batchWrite 保存。新任务或重启 Codex 后使用新配置。"
          : "Saved through Codex config/batchWrite. New tasks or a Codex restart will use it.",
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  const testConnection = async () => {
    setTesting(true);
    setError(null);
    setNotice(null);
    setTestResult(null);
    try {
      const result = await invoke<ProviderTestResult>(
        "test_provider_connection",
        { request },
      );
      setTestResult(result);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setTesting(false);
    }
  };

  const restore = async () => {
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const next = await invoke<ProviderSnapshot>("restore_provider_config");
      setSnapshot(next);
      setPreview(false);
      setNotice(
        zh
          ? "已恢复上一个 Provider；恢复前的配置也已保留，可再次切换回来。"
          : "Previous provider restored. The configuration you replaced is kept for redo.",
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <main className="workspace provider-workspace" hidden={hidden}>
      <header className="topbar">
        <div>
          <h1>{zh ? "控制台" : "Console"}</h1>
          <span className="header-note">
            {zh
              ? "查看并修改 Codex 用户级配置"
              : "Review and edit user-level Codex configuration"}
          </span>
        </div>
        <div className="topbar-actions">
          <span>
            {zh ? "读取 config.toml" : "Reading config.toml"}
          </span>
          {activeSection === "provider" && (
            <button
              className={loading ? "refresh-button spinning" : "refresh-button"}
              onClick={() => void load()}
              disabled={loading || saving}
              aria-label={zh ? "刷新控制台" : "Refresh control center"}
            >
              <RefreshCw aria-hidden="true" />
            </button>
          )}
        </div>
      </header>

      <nav
        className="control-center-nav"
        aria-label={zh ? "Codex 设置分类" : "Codex setting categories"}
      >
        {(
          [
            ["provider", zh ? "供应商" : "Provider"],
            ["model", zh ? "模型" : "Model"],
            ["permissions", zh ? "权限" : "Permissions"],
            ["context", zh ? "上下文" : "Context"],
            ["capabilities", zh ? "工具" : "Tools"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            className={activeSection === id ? "active" : ""}
            aria-current={activeSection === id ? "page" : undefined}
            onClick={() => openSection(id)}
          >
            {label}
          </button>
        ))}
      </nav>

      {activeSection === "provider" ? (
        <>
      <section className="provider-active">
        <div>
          <p>{zh ? "当前供应商" : "Active provider"}</p>
          <h2>{activeDefinition?.name ?? snapshot?.active_provider ?? "—"}</h2>
        </div>
        <dl>
          <div>
            <dt>{zh ? "模型" : "Model"}</dt>
            <dd>{snapshot?.active_model ?? "—"}</dd>
          </div>
          <div>
            <dt>{zh ? "协议" : "Protocol"}</dt>
            <dd>{activeDefinition?.wire_api ?? "responses"}</dd>
          </div>
          <div>
            <dt>{zh ? "凭据" : "Credential"}</dt>
            <dd>
              {activeDefinition?.env_key
                ? activeDefinition.env_available
                  ? zh
                    ? `${activeDefinition.env_key} 已检测`
                    : `${activeDefinition.env_key} detected`
                  : zh
                    ? `${activeDefinition.env_key} 未检测`
                    : `${activeDefinition.env_key} missing`
                : activeDefinition?.builtin
                  ? zh
                    ? "Codex 登录 / 内置"
                    : "Codex sign-in / built-in"
                  : zh
                    ? "未声明环境变量"
                    : "No environment variable"}
            </dd>
          </div>
        </dl>
        <div className="provider-active-actions">
          <span title={snapshot?.config_path}>
            {zh ? "用户配置" : "User config"} ·{" "}
            {snapshot?.config_path ?? "~/.codex/config.toml"}
          </span>
          <button
            className="text-button"
            disabled={!snapshot?.restore_available || saving}
            onClick={() => void restore()}
          >
            {zh ? "恢复上一个" : "Restore previous"}
          </button>
        </div>
      </section>

      <section className="provider-console">
        <div className="provider-presets">
          <div className="provider-section-title">
            <p>{zh ? "供应商模板" : "Provider templates"}</p>
          </div>
          <div className="provider-preset-list">
            {PRESETS.map((preset) => (
              <button
                key={preset.id}
                className={selectedId === preset.id ? "selected" : ""}
                onClick={() => selectPreset(preset)}
              >
                <span>
                  <strong>{preset.title}</strong>
                  <small>{zh ? preset.vendorZh : preset.vendorEn}</small>
                </span>
                <em className={`support-${preset.support}`}>
                  {supportLabel(preset.support, locale)}
                </em>
              </button>
            ))}
          </div>
        </div>

        <div className="provider-editor">
          <div className="provider-section-title">
            <p>
              {zh ? "配置" : "Configuration"}
              {hasProviderChanges && (
                <span className="provider-draft-state">
                  {zh ? "未保存草稿" : "Unsaved draft"}
                  <button type="button" onClick={discardDraft}>
                    {zh ? "放弃" : "Discard"}
                  </button>
                </span>
              )}
            </p>
            <button
              className="docs-link"
              onClick={() => onOpenUrl(selectedPreset.docsUrl)}
            >
              <span>
                {zh ? "接口文档" : "API documentation"}
              </span>
              <ExternalLink aria-hidden="true" />
            </button>
          </div>

          <div className={`provider-support support-${selectedPreset.support}`}>
            <strong>{supportLabel(selectedPreset.support, locale)}</strong>
            <span>{zh ? selectedPreset.noteZh : selectedPreset.noteEn}</span>
          </div>

          <div className="provider-form">
            <label>
              <span>Provider ID</span>
              <input
                value={draft.providerId}
                onChange={(event) =>
                  setDraft({ ...draft, providerId: event.target.value })
                }
                disabled={selectedPreset.id === "openai"}
                spellCheck={false}
              />
            </label>
            <label>
              <span>{zh ? "显示名称" : "Display name"}</span>
              <input
                value={draft.name}
                onChange={(event) =>
                  setDraft({ ...draft, name: event.target.value })
                }
                disabled={selectedPreset.id === "openai"}
              />
            </label>
            <label className="provider-field-wide">
              <span>{zh ? "模型 ID" : "Model ID"}</span>
              <input
                value={draft.model}
                list="codex-provider-model-options"
                onChange={(event) =>
                  setDraft({ ...draft, model: event.target.value })
                }
                placeholder={
                  zh ? "必须与平台模型 ID 完全一致" : "Exact platform model ID"
                }
                spellCheck={false}
              />
              <datalist id="codex-provider-model-options">
                {modelSuggestions.map((model) => (
                  <option key={model} value={model} />
                ))}
              </datalist>
              <small>
                {selectedPreset.id === "openai" && snapshot?.models.length
                  ? zh
                    ? `${snapshot.models.length} 个选项来自 Codex 官方 model/list，也可以手动填写`
                    : `${snapshot.models.length} options from Codex model/list; manual IDs are also accepted`
                  : zh
                    ? "可从建议值选择，也可以填写平台提供的精确模型 ID"
                    : "Choose a suggestion or enter the exact model ID from the provider"}
              </small>
            </label>
            {!editingBuiltin && (
              <>
                <label className="provider-field-wide">
                  <span>Responses API Base URL</span>
                  <input
                    value={draft.baseUrl}
                    onChange={(event) =>
                      setDraft({ ...draft, baseUrl: event.target.value })
                    }
                    placeholder="https://provider.example.com/v1"
                    spellCheck={false}
                  />
                  <small>
                    {zh
                      ? "Codex 会在此地址后调用 /responses"
                      : "Codex calls /responses under this base URL"}
                  </small>
                </label>
                <label className="provider-field-wide">
                  <span>{zh ? "API Key 环境变量" : "API key environment variable"}</span>
                  <input
                    value={draft.envKey}
                    onChange={(event) =>
                      setDraft({ ...draft, envKey: event.target.value })
                    }
                    placeholder="MODEL_API_KEY"
                    spellCheck={false}
                  />
                  <small>
                    {zh
                      ? "这里只写变量名，绝不填写或保存真实 Key"
                      : "Enter the variable name only, never the actual key"}
                  </small>
                </label>
              </>
            )}
          </div>

          {!preview ? (
            <div className="provider-editor-actions">
              <span>
                {!configurationComplete
                  ? zh
                    ? "请补全 Provider、模型和 Responses 地址"
                    : "Complete the provider, model, and Responses URL"
                  : !hasProviderChanges
                    ? zh
                      ? "当前配置没有变化"
                      : "No changes to save"
                    : zh
                      ? "先核对差异，再写入用户配置"
                      : "Review the diff before writing user config"}
              </span>
              <div>
                <button
                  className="secondary-action"
                  disabled={!configurationComplete || loading || testing}
                  onClick={() => void testConnection()}
                >
                  <FlaskConical aria-hidden="true" />
                  {testing
                    ? zh
                      ? "测试中…"
                      : "Testing…"
                    : zh
                      ? "测试连接"
                      : "Test connection"}
                </button>
                <button
                  className="primary-action"
                  disabled={!canPreview || loading || testing}
                  onClick={() => {
                    setPreview(true);
                    setError(null);
                    setNotice(null);
                  }}
                >
                  {zh ? "预览变更" : "Preview changes"}
                </button>
              </div>
            </div>
          ) : (
            <div className="provider-preview">
              <p>{zh ? "将写入 Codex 用户配置" : "Will write to Codex user config"}</p>
              <div>
                <span>
                  <small>{zh ? "当前" : "Current"}</small>
                  <strong>
                    {snapshot?.active_provider ?? "—"} /{" "}
                    {snapshot?.active_model ?? "—"}
                  </strong>
                </span>
                <ArrowRight aria-hidden="true" />
                <span>
                  <small>{zh ? "目标" : "Target"}</small>
                  <strong>
                    {request.provider_id} / {request.model}
                  </strong>
                </span>
              </div>
              {request.base_url && <code>{request.base_url}/responses</code>}
              <footer>
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
                  {saving
                    ? zh
                      ? "正在保存…"
                      : "Saving…"
                    : zh
                      ? "确认切换"
                      : "Confirm switch"}
                </button>
              </footer>
            </div>
          )}

          {testResult && (
            <div
              className={`provider-test-result ${testResult.success ? "success" : "error"}`}
              role="status"
              aria-live="polite"
            >
              {testResult.success ? (
                <CheckCircle2 aria-hidden="true" />
              ) : (
                <XCircle aria-hidden="true" />
              )}
              <div>
                <strong>
                  {testResult.success
                    ? zh
                      ? "验证通过"
                      : "Verified"
                    : zh
                      ? "验证失败"
                      : "Verification failed"}
                  {" · "}
                  {testResult.latency_ms} ms
                  {testResult.http_status
                    ? ` · HTTP ${testResult.http_status}`
                    : ""}
                </strong>
                <span>{testResult.message}</span>
                {testResult.check_kind === "responses_request" && (
                  <small>
                    {zh
                      ? "已发送一次最小生成请求，可能产生极少量 API 费用。"
                      : "A minimal generation request was sent and may incur a very small API charge."}
                  </small>
                )}
              </div>
            </div>
          )}
          {error && (
            <div className="provider-message error" role="alert">
              {error}
            </div>
          )}
          {notice && (
            <div className="provider-message success" role="status">
              {notice}
            </div>
          )}
        </div>
      </section>

      <details className="provider-detected">
        <summary>
          <strong>{zh ? "已有 Provider" : "Configured providers"}</strong>
          <span>{snapshot?.providers.length ?? 0}</span>
        </summary>
        <div className="provider-detected-table">
          {snapshot?.providers.map((provider) => (
            <button
              key={provider.id}
              className={
                provider.id === snapshot.active_provider ? "active" : ""
              }
              onClick={() => selectDetected(provider)}
            >
              <span>
                <strong>{provider.name}</strong>
                <small>{provider.id}</small>
              </span>
              <code>{provider.base_url ?? "Codex built-in"}</code>
              <em>{detectedStatus(provider, locale)}</em>
            </button>
          ))}
        </div>
        {!loading && !snapshot && (
          <p className="provider-empty">
            {zh ? "没有读到 Provider 配置。" : "No provider configuration found."}
          </p>
        )}
      </details>
        </>
      ) : null}
      {settingsMounted && (
        <div hidden={activeSection === "provider"}>
          <ConfigPanel category={lastConfigSection} locale={locale} />
        </div>
      )}
    </main>
  );
}
