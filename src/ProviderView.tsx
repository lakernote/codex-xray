import { invoke } from "@tauri-apps/api/core";
import {
  ArrowRight,
  CheckCircle2,
  Eye,
  EyeOff,
  ExternalLink,
  FlaskConical,
  KeyRound,
  RefreshCw,
  XCircle,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import ConfigPanel, { type ConfigCategory } from "./ConfigPanel";
import EnvironmentView from "./EnvironmentView";
import type { Locale } from "./i18n";
import type {
  ChatBridgeStatus,
  ProviderApplyRequest,
  ProviderDefinition,
  ProviderSnapshot,
  ProviderTestResult,
} from "./types";

export type ControlSection = "provider" | "diagnostics" | ConfigCategory;

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
  protocol: "responses" | "chat_completions";
  contextWindow: number;
  support: "official" | "native" | "hosted" | "bridge" | "custom";
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
  protocol: "responses" | "chat_completions";
  contextWindow: number;
  credentialMode: "keychain" | "environment" | "none";
  apiKey: string;
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
    protocol: "responses",
    contextWindow: 128_000,
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
    protocol: "responses",
    contextWindow: 128_000,
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
    protocol: "responses",
    contextWindow: 128_000,
    support: "native",
    noteZh: "官方 Responses，支持流式输出和函数工具。",
    noteEn: "Native Responses with streaming and function tools.",
    docsUrl: "https://www.volcengine.com/docs/82379/1795150",
  },
  {
    id: "glm",
    title: "GLM",
    vendorZh: "智谱开放平台",
    vendorEn: "Zhipu AI",
    providerId: "glm-direct",
    name: "Zhipu GLM",
    model: "glm-5.2",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    envKey: "ZHIPUAI_API_KEY",
    protocol: "chat_completions",
    contextWindow: 128_000,
    support: "bridge",
    noteZh: "原厂 Chat 接口；X-Ray 在本机转换为 Codex 所需的 Responses 流。",
    noteEn: "Native Chat API translated locally into the Responses stream Codex expects.",
    docsUrl: "https://docs.bigmodel.cn/cn/guide/develop/http/introduction",
  },
  {
    id: "deepseek",
    title: "DeepSeek",
    vendorZh: "DeepSeek 原厂",
    vendorEn: "DeepSeek",
    providerId: "deepseek-direct",
    name: "DeepSeek",
    model: "deepseek-v4-pro",
    baseUrl: "https://api.deepseek.com",
    envKey: "DEEPSEEK_API_KEY",
    protocol: "chat_completions",
    contextWindow: 1_000_000,
    support: "bridge",
    noteZh: "原厂 Chat 接口；支持函数工具，经 X-Ray 本机兼容桥接入 Codex。",
    noteEn: "Native Chat API with function tools, connected through the local X-Ray bridge.",
    docsUrl: "https://api-docs.deepseek.com/api/create-chat-completion",
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
    protocol: "responses",
    contextWindow: 128_000,
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
    protocol: "responses",
    contextWindow: 128_000,
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
    protocol: "responses",
    contextWindow: 128_000,
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
    protocol: preset.protocol,
    contextWindow: preset.contextWindow,
    credentialMode: preset.id === "openai" ? "none" : "keychain",
    apiKey: "",
  };
}

function draftFromProvider(
  provider: ProviderDefinition,
  model: string,
): Draft {
  return {
    providerId: provider.id,
    name: provider.name,
    model,
    baseUrl: provider.base_url ?? "",
    envKey: provider.env_key ?? "",
    protocol: provider.protocol,
    contextWindow: provider.context_window ?? 128_000,
    credentialMode:
      provider.credential_source === "environment"
        ? "environment"
        : provider.credential_source === "none"
          ? "none"
          : "keychain",
    apiKey: "",
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
    bridge: "Chat 兼容桥",
    custom: "需要验证",
  };
  const en = {
    official: "Built into Codex",
    native: "Native Responses",
    hosted: "Hosted Responses",
    bridge: "Chat bridge",
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
  if (provider.compatibility === "chat_bridge") {
    return locale === "zh-CN"
      ? "Chat · 本机转换"
      : "Chat · local bridge";
  }
  if (provider.compatibility === "unsupported_wire_api") {
    return locale === "zh-CN" ? "协议不受支持" : "Unsupported protocol";
  }
  if (provider.env_key && !provider.env_available) {
    return locale === "zh-CN"
      ? `未检测到 ${provider.env_key}`
      : `${provider.env_key} not detected`;
  }
  if (provider.credential_source === "keychain") {
    return provider.credential_available
      ? locale === "zh-CN"
        ? "API Key 已保存"
        : "API key saved"
      : locale === "zh-CN"
        ? "缺少 API Key"
        : "Key missing";
  }
  if (provider.credential_source === "command") {
    return locale === "zh-CN" ? "外部认证命令" : "External auth command";
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
    initialSection === "provider" || initialSection === "diagnostics"
      ? "model"
      : initialSection,
  );
  const [settingsMounted, setSettingsMounted] = useState(
    initialSection !== "provider" && initialSection !== "diagnostics",
  );
  const [snapshot, setSnapshot] = useState<ProviderSnapshot | null>(null);
  const [bridgeStatus, setBridgeStatus] = useState<ChatBridgeStatus | null>(
    null,
  );
  const initializedFromConfig = useRef(false);
  const [selectedId, setSelectedId] = useState("openai");
  const selectedPreset =
    PRESETS.find((preset) => preset.id === selectedId) ?? PRESETS[0];
  const [draft, setDraft] = useState<Draft>(() =>
    draftFromPreset(PRESETS[0]),
  );
  const [preview, setPreview] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);
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
  const activeCredentialLabel = activeDefinition?.builtin
    ? zh
      ? "Codex 登录 / 内置"
      : "Codex sign-in / built-in"
    : activeDefinition?.credential_source === "keychain"
      ? activeDefinition.credential_available
        ? zh
          ? "API Key 已保存"
          : "API key saved"
        : zh
          ? "缺少 API Key"
          : "Key missing"
      : activeDefinition?.credential_source === "environment"
        ? `${activeDefinition.env_key} ${
            activeDefinition.env_available
              ? zh
                ? "已检测"
                : "detected"
              : zh
                ? "未检测"
                : "missing"
          }`
        : activeDefinition?.credential_source === "command"
          ? zh
            ? "外部认证命令"
            : "External auth command"
          : zh
            ? "无需认证"
            : "No authentication";
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
      const [next, bridge] = await Promise.all([
        invoke<ProviderSnapshot>("get_provider_config"),
        invoke<ChatBridgeStatus>("get_chat_bridge_status"),
      ]);
      setSnapshot(next);
      setBridgeStatus(bridge);
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
            setDraft(draftFromProvider(active, next.active_model ?? ""));
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
    if (initialSection !== "provider" && initialSection !== "diagnostics") {
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
    setDraft(
      draftFromProvider(
        provider,
        provider.id === snapshot?.active_provider
          ? snapshot.active_model ?? ""
          : "",
      ),
    );
    setPreview(false);
    setNotice(null);
  };

  const request: ProviderApplyRequest = {
    provider_id: draft.providerId.trim(),
    model: draft.model.trim(),
    name: editingBuiltin ? null : draft.name.trim() || null,
    base_url: editingBuiltin ? null : draft.baseUrl.trim() || null,
    env_key:
      editingBuiltin || draft.credentialMode !== "environment"
        ? null
        : draft.envKey.trim() || null,
    credential_mode: editingBuiltin ? null : draft.credentialMode,
    api_key:
      editingBuiltin || draft.credentialMode !== "keychain"
        ? null
        : draft.apiKey.trim() || null,
    protocol: editingBuiltin ? "responses" : draft.protocol,
    context_window: draft.contextWindow,
    expected_version: snapshot?.version ?? null,
  };

  const targetDefinition = snapshot?.providers.find(
    (provider) => provider.id === request.provider_id,
  );
  const hasStoredApiKey =
    targetDefinition?.credential_source === "keychain" &&
    targetDefinition.credential_available;
  const configurationComplete =
    request.provider_id.length > 0 &&
    request.model.length > 0 &&
    (request.protocol !== "chat_completions" ||
      (request.context_window >= 8_192 &&
        request.context_window <= 4_000_000)) &&
    (editingBuiltin ||
      (Boolean(request.base_url) &&
        (request.credential_mode !== "environment" ||
          Boolean(request.env_key)) &&
        (request.credential_mode !== "keychain" ||
          Boolean(request.api_key) ||
          hasStoredApiKey)));
  const hasProviderChanges = Boolean(
    snapshot &&
      (request.provider_id !== snapshot.active_provider ||
        request.model !== (snapshot.active_model ?? "") ||
        (!editingBuiltin &&
          (request.name !== (targetDefinition?.name ?? null) ||
            request.base_url !== (targetDefinition?.base_url ?? null) ||
            request.env_key !== (targetDefinition?.env_key ?? null) ||
            request.protocol !==
              (targetDefinition?.protocol ?? "responses") ||
            (request.protocol === "chat_completions" &&
              request.context_window !==
                (targetDefinition?.context_window ?? 128_000)) ||
            request.credential_mode !==
              (targetDefinition?.credential_source === "environment"
                ? "environment"
                : targetDefinition?.credential_source === "none"
                  ? "none"
                  : "keychain") ||
            Boolean(request.api_key)))),
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
      setDraft(draftFromProvider(active, snapshot.active_model ?? ""));
    }
    setPreview(false);
    setError(null);
    setNotice(null);
  };

  const openSection = (section: ControlSection) => {
    if (section !== "provider" && section !== "diagnostics") {
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
      setDraft((current) => ({ ...current, apiKey: "" }));
      setPreview(false);
      setNotice(
        request.protocol === "chat_completions"
          ? zh
            ? "已保存。新任务会通过 X-Ray 本机兼容桥连接 Chat API；使用时请保持 X-Ray 运行。"
            : "Saved. New tasks use the local X-Ray bridge for this Chat API; keep X-Ray running."
          : zh
            ? "已通过 Codex 官方 config/batchWrite 保存。新任务或重启 Codex 后使用新配置。"
            : "Saved through Codex config/batchWrite. New tasks or a Codex restart will use it.",
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  const clearCredential = async () => {
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      await invoke("clear_provider_credential", {
        providerId: request.provider_id,
      });
      setDraft((current) => ({ ...current, apiKey: "" }));
      await load();
      setNotice(
        zh
          ? "已从系统凭据存储删除这个 Provider 的 API Key。"
          : "The API key was removed from the system credential store.",
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
          {activeSection !== "diagnostics" && (
            <span>
              {loading
                ? zh
                  ? "正在读取配置"
                  : "Loading configuration"
                : zh
                  ? "用户配置已加载"
                  : "User configuration loaded"}
            </span>
          )}
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
            ["diagnostics", zh ? "诊断" : "Diagnostics"],
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
            <dd title={snapshot?.active_model ?? "—"}>
              {snapshot?.active_model ?? "—"}
            </dd>
          </div>
          <div>
            <dt>{zh ? "协议" : "Protocol"}</dt>
            <dd
              title={
                activeDefinition?.protocol === "chat_completions"
                  ? "Chat → Responses"
                  : "Responses"
              }
            >
              {activeDefinition?.protocol === "chat_completions"
                ? zh
                  ? "Chat → Responses"
                  : "Chat → Responses"
                : "Responses"}
            </dd>
          </div>
          <div>
            <dt>{zh ? "凭据" : "Credential"}</dt>
            <dd title={activeCredentialLabel}>{activeCredentialLabel}</dd>
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
                  <small title={zh ? preset.vendorZh : preset.vendorEn}>
                    {zh ? preset.vendorZh : preset.vendorEn}
                  </small>
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
            <strong>
              {selectedPreset.id === "custom" &&
              draft.protocol === "chat_completions"
                ? supportLabel("bridge", locale)
                : supportLabel(selectedPreset.support, locale)}
            </strong>
            <span>
              {selectedPreset.id === "custom" &&
              draft.protocol === "chat_completions"
                ? zh
                  ? "兼容 OpenAI Chat Completions 的上游将由 X-Ray 在本机转换。"
                  : "An OpenAI-compatible Chat Completions upstream will be translated locally by X-Ray."
                : zh
                  ? selectedPreset.noteZh
                  : selectedPreset.noteEn}
            </span>
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
                <div className="provider-form-group provider-field-wide">
                  <span>{zh ? "上游接口" : "Upstream API"}</span>
                  <div
                    className="provider-protocol"
                    role="group"
                    aria-label={zh ? "选择上游接口协议" : "Choose upstream API protocol"}
                  >
                    <button
                      type="button"
                      className={draft.protocol === "responses" ? "selected" : ""}
                      aria-pressed={draft.protocol === "responses"}
                      onClick={() =>
                        setDraft({ ...draft, protocol: "responses" })
                      }
                    >
                      Responses
                      <small>{zh ? "Codex 原生" : "Native"}</small>
                    </button>
                    <button
                      type="button"
                      className={
                        draft.protocol === "chat_completions" ? "selected" : ""
                      }
                      aria-pressed={draft.protocol === "chat_completions"}
                      onClick={() =>
                        setDraft({ ...draft, protocol: "chat_completions" })
                      }
                    >
                      Chat Completions
                      <small>{zh ? "本机转换" : "Local bridge"}</small>
                    </button>
                  </div>
                  {draft.protocol === "chat_completions" && (
                    <small
                      className={
                        bridgeStatus?.running
                          ? "provider-bridge-note running"
                          : "provider-bridge-note error"
                      }
                    >
                      {bridgeStatus?.running
                        ? zh
                          ? "兼容桥正在运行。内置 Web Search、服务端压缩和加密 Reasoning 不会传给 Chat 上游。"
                          : "Bridge is running. Native web search, server compaction, and encrypted reasoning are not forwarded upstream."
                        : zh
                          ? `兼容桥未运行${bridgeStatus?.last_error ? `：${bridgeStatus.last_error}` : ""}`
                          : `Bridge is not running${bridgeStatus?.last_error ? `: ${bridgeStatus.last_error}` : ""}`}
                    </small>
                  )}
                </div>
                <label className="provider-field-wide">
                  <span>
                    {draft.protocol === "chat_completions"
                      ? "Chat Completions Base URL"
                      : "Responses API Base URL"}
                  </span>
                  <input
                    value={draft.baseUrl}
                    onChange={(event) =>
                      setDraft({ ...draft, baseUrl: event.target.value })
                    }
                    placeholder="https://provider.example.com/v1"
                    spellCheck={false}
                  />
                  <small>
                    {draft.protocol === "chat_completions"
                      ? zh
                        ? "填写厂商 Base URL；X-Ray 会请求其 /chat/completions"
                        : "Enter the vendor base URL; X-Ray calls /chat/completions"
                      : zh
                        ? "Codex 会在此地址后调用 /responses"
                        : "Codex calls /responses under this base URL"}
                  </small>
                </label>
                {draft.protocol === "chat_completions" && (
                  <label>
                    <span>{zh ? "上下文窗口" : "Context window"}</span>
                    <input
                      type="number"
                      min={8_192}
                      max={4_000_000}
                      step={1_000}
                      value={draft.contextWindow}
                      onChange={(event) =>
                        setDraft({
                          ...draft,
                          contextWindow: Number(event.target.value),
                        })
                      }
                    />
                    <small>
                      {zh
                        ? "按厂商模型文档填写 Token 上限；用于 Codex 压缩时机和用量显示"
                        : "Use the provider's documented token limit; Codex uses it for compaction and usage display"}
                    </small>
                  </label>
                )}
                {draft.credentialMode === "keychain" && (
                  <label className="provider-field-wide">
                    <span>{zh ? "API Key（推荐）" : "API key (recommended)"}</span>
                    <div className="provider-secret-input">
                      <KeyRound aria-hidden="true" />
                      <input
                        type={showApiKey ? "text" : "password"}
                        value={draft.apiKey}
                        onChange={(event) =>
                          setDraft({ ...draft, apiKey: event.target.value })
                        }
                        placeholder={
                          targetDefinition?.credential_available
                            ? zh
                              ? "已保存；留空表示继续使用"
                              : "Saved; leave blank to keep it"
                            : zh
                              ? "粘贴平台提供的 API Key"
                              : "Paste the provider API key"
                        }
                        autoComplete="new-password"
                        spellCheck={false}
                      />
                      <button
                        type="button"
                        title={
                          showApiKey ? (zh ? "隐藏" : "Hide") : zh ? "显示" : "Show"
                        }
                        aria-label={
                          showApiKey
                            ? zh
                              ? "隐藏 API Key"
                              : "Hide API key"
                            : zh
                              ? "显示 API Key"
                              : "Show API key"
                        }
                        onClick={() => setShowApiKey((value) => !value)}
                      >
                        {showApiKey ? (
                          <EyeOff aria-hidden="true" />
                        ) : (
                          <Eye aria-hidden="true" />
                        )}
                      </button>
                    </div>
                    <small className="provider-secret-status">
                      <span>
                        {targetDefinition?.credential_source === "keychain" &&
                        targetDefinition.credential_available
                          ? zh
                            ? "已保存在系统钥匙串；留空不会覆盖"
                            : "Stored in the system credential store; leave blank to keep it"
                          : zh
                            ? "保存到系统钥匙串；config.toml 不记录明文"
                            : "Stored in the system credential store; config.toml never contains the key"}
                      </span>
                      {targetDefinition?.credential_source === "keychain" &&
                        targetDefinition.credential_available && (
                          <button
                            type="button"
                            disabled={saving}
                            onClick={() => void clearCredential()}
                          >
                            {zh ? "删除已保存 Key" : "Remove saved key"}
                          </button>
                        )}
                    </small>
                  </label>
                )}
                {draft.credentialMode === "environment" && (
                  <div className="provider-alternative-auth provider-field-wide">
                    <div>
                      <strong>{zh ? "使用环境变量" : "Use environment variable"}</strong>
                      <button
                        type="button"
                        onClick={() =>
                          setDraft({
                            ...draft,
                            credentialMode: "keychain",
                            apiKey: "",
                          })
                        }
                      >
                        {zh ? "改为直接填写 Key" : "Enter a key instead"}
                      </button>
                    </div>
                    <label>
                      <span>
                        {zh
                          ? "API Key 环境变量"
                          : "API key environment variable"}
                      </span>
                      <input
                        value={draft.envKey}
                        onChange={(event) =>
                          setDraft({ ...draft, envKey: event.target.value })
                        }
                        placeholder="MODEL_API_KEY"
                        spellCheck={false}
                      />
                    </label>
                  </div>
                )}
                {draft.credentialMode === "none" && (
                  <div className="provider-alternative-auth provider-field-wide">
                    <div>
                      <strong>{zh ? "不使用 API Key" : "No API key"}</strong>
                      <button
                        type="button"
                        onClick={() =>
                          setDraft({
                            ...draft,
                            credentialMode: "keychain",
                            apiKey: "",
                          })
                        }
                      >
                        {zh ? "改为直接填写 Key" : "Enter a key instead"}
                      </button>
                    </div>
                    <small>
                      {zh
                        ? "仅适用于本地服务或明确不需要认证的网关。"
                        : "Only for local services or gateways that explicitly require no authentication."}
                    </small>
                  </div>
                )}
                <details className="provider-auth-advanced provider-field-wide">
                  <summary>
                    {zh ? "其他认证方式" : "Other authentication methods"}
                  </summary>
                  <div
                    className="provider-credential-mode"
                    role="group"
                    aria-label={
                      zh ? "选择其他认证方式" : "Choose another authentication method"
                    }
                  >
                    {(
                      [
                        ["environment", zh ? "环境变量" : "Environment variable"],
                        ["none", zh ? "无需认证" : "No authentication"],
                      ] as const
                    ).map(([mode, label]) => (
                      <button
                        key={mode}
                        type="button"
                        className={
                          draft.credentialMode === mode ? "selected" : ""
                        }
                        aria-pressed={draft.credentialMode === mode}
                        onClick={() =>
                          setDraft({
                            ...draft,
                            credentialMode: mode,
                            apiKey: "",
                          })
                        }
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                </details>
              </>
            )}
          </div>

          {!preview ? (
            <div className="provider-editor-actions">
              <span>
                {!configurationComplete
                  ? zh
                    ? "请补全 Provider、模型、接口地址和认证信息"
                    : "Complete the provider, model, API URL, and authentication"
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
              {request.base_url && (
                <code>
                  {request.protocol === "chat_completions"
                    ? `Codex → X-Ray → ${request.base_url.replace(/\/$/, "")}/chat/completions`
                    : `${request.base_url.replace(/\/$/, "")}/responses`}
                </code>
              )}
              {!editingBuiltin && (
                <span className="provider-preview-auth">
                  {zh ? "认证" : "Authentication"} ·{" "}
                  {request.credential_mode === "keychain"
                    ? zh
                      ? "API Key"
                      : "API key"
                    : request.credential_mode === "environment"
                      ? request.env_key
                      : zh
                        ? "无需认证"
                        : "None"}
                </span>
              )}
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
                {(testResult.check_kind === "responses_request" ||
                  testResult.check_kind === "chat_completions_request") && (
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
        <div
          hidden={
            activeSection === "provider" || activeSection === "diagnostics"
          }
        >
          <ConfigPanel category={lastConfigSection} locale={locale} />
        </div>
      )}
      {activeSection === "diagnostics" && (
        <EnvironmentView locale={locale} />
      )}
    </main>
  );
}
