import { invoke } from "@tauri-apps/api/core";
import {
  ArrowRight,
  BrainCircuit,
  CheckCircle2,
  CircleHelp,
  Eye,
  EyeOff,
  ExternalLink,
  FlaskConical,
  Gauge,
  ImageOff,
  KeyRound,
  ListChecks,
  MessageSquareText,
  Plus,
  Power,
  RefreshCw,
  Save,
  SearchX,
  Workflow,
  Wrench,
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
  ProviderProfile,
  ProviderSnapshot,
  ProviderTestResult,
} from "./types";

export type ControlSection = "provider" | "diagnostics" | ConfigCategory;

type ProviderViewProps = {
  locale: Locale;
  onOpenUrl: (url: string) => void;
  surface: "access" | "console";
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
  profileName: string;
  providerId: string;
  name: string;
  model: string;
  baseUrl: string;
  envKey: string;
  protocol: "responses" | "chat_completions";
  contextWindow: number;
  credentialMode: "file" | "environment" | "none";
  apiKey: string;
};

type ProviderCapability =
  ProviderTestResult["capabilities"][number];

function CapabilityIcon({
  capability,
}: {
  capability: ProviderCapability["capability"];
}) {
  switch (capability) {
    case "model_catalog":
      return <ListChecks aria-hidden="true" />;
    case "text_generation":
      return <MessageSquareText aria-hidden="true" />;
    case "function_tools":
      return <Wrench aria-hidden="true" />;
    case "image_input":
      return <ImageOff aria-hidden="true" />;
    case "builtin_tools":
      return <SearchX aria-hidden="true" />;
    case "reasoning_compaction":
      return <BrainCircuit aria-hidden="true" />;
    case "context_window":
      return <Gauge aria-hidden="true" />;
  }
}

function capabilityTitle(
  capability: ProviderCapability["capability"],
  zh: boolean,
) {
  const labels = {
    model_catalog: zh ? "模型目录" : "Model catalog",
    text_generation: zh ? "文本生成" : "Text generation",
    function_tools: zh ? "函数工具" : "Function tools",
    image_input: zh ? "图片输入" : "Image input",
    builtin_tools: zh ? "Codex 内置工具" : "Codex built-in tools",
    reasoning_compaction: zh
      ? "Reasoning 与压缩"
      : "Reasoning & compaction",
    context_window: zh ? "上下文窗口" : "Context window",
  };
  return labels[capability];
}

function capabilityStatus(
  status: ProviderCapability["status"],
  zh: boolean,
) {
  const labels = {
    verified: zh ? "已实测" : "Verified",
    failed: zh ? "失败" : "Failed",
    bridge_supported: zh ? "桥接支持" : "Bridge support",
    limited: zh ? "受限" : "Limited",
    unverified: zh ? "未验证" : "Not verified",
    configured: zh ? "配置值" : "Configured",
  };
  return labels[status];
}

function capabilityDetail(item: ProviderCapability, zh: boolean) {
  const verified = item.status === "verified";
  switch (item.capability) {
    case "model_catalog":
      return verified
        ? zh
          ? "已从 Codex model/list 找到该模型；没有发起生成。"
          : "Found in Codex model/list; no generation request was made."
        : zh
          ? "Codex model/list 没有返回该模型。"
          : "The model was not returned by Codex model/list.";
    case "text_generation":
      if (item.status === "unverified") {
        return zh
          ? "本次只检查模型目录，没有发送生成请求。"
          : "Only the model catalog was checked; generation was not requested.";
      }
      return verified
        ? zh
          ? "最小文本请求已真实返回成功。"
          : "A real minimal text request completed successfully."
        : zh
          ? "最小文本请求没有成功。"
          : "The minimal text request did not succeed.";
    case "function_tools":
      return item.status === "bridge_supported"
        ? zh
          ? "X-Ray 可转换函数调用与结果；本次未实际调用工具。"
          : "X-Ray translates function calls and results; this probe did not invoke one."
        : zh
          ? "取决于模型服务的 Responses 实现；本次未调用工具。"
          : "Depends on the Provider's Responses implementation; no tool was invoked.";
    case "image_input":
      return item.status === "limited"
        ? zh
          ? "Chat 桥不发送图片：本轮图片会阻止请求，历史图片会被省略。"
          : "The Chat bridge does not send images: current images block the request and earlier images are omitted."
        : zh
          ? "取决于模型服务；本次未发送图片。"
          : "Provider-dependent; this probe did not send an image.";
    case "builtin_tools":
      return item.status === "limited"
        ? zh
          ? "内置 Web Search 等 Responses 专属工具不会转发给 Chat 上游。"
          : "Responses-only tools such as built-in Web Search are not forwarded to Chat upstreams."
        : zh
          ? "取决于模型服务；本次未验证内置工具。"
          : "Provider-dependent; built-in tools were not verified.";
    case "reasoning_compaction":
      return item.status === "limited"
        ? zh
          ? "加密 Reasoning 与服务端压缩记录不会传给 Chat 上游。"
          : "Encrypted reasoning and server-side compaction records are not sent to Chat upstreams."
        : zh
          ? "取决于模型服务；本次未验证 Reasoning 或服务端压缩。"
          : "Provider-dependent; reasoning and server-side compaction were not verified.";
    case "context_window":
      return item.status === "configured" && item.value
        ? zh
          ? `${Number(item.value).toLocaleString("zh-CN")} Token 是手动配置值，用于 Codex 显示和压缩；并非从上游探测。`
          : `${Number(item.value).toLocaleString("en-US")} tokens is a configured value used by Codex for display and compaction, not detected upstream.`
        : zh
          ? "本次最小请求无法得知模型服务的真实上下文上限。"
          : "The minimal probe cannot determine the Provider's actual context limit.";
  }
}

function providerTestMessage(result: ProviderTestResult, zh: boolean) {
  if (!result.success) return result.message;
  switch (result.check_kind) {
    case "codex_model_catalog":
      return zh
        ? "Codex 官方 model/list 已返回该模型；本次没有发起 LLM 生成。"
        : "Codex model/list returned this model; no LLM generation was requested.";
    case "chat_completions_request":
      return zh
        ? "真实 /chat/completions 请求成功；保存后由 X-Ray 本机兼容桥连接。"
        : "A real /chat/completions request succeeded; X-Ray will connect it through the local compatibility bridge after saving.";
    case "responses_request":
      return zh
        ? "真实 /responses 请求成功；模型、地址和凭据已通过本次探测。"
        : "A real /responses request succeeded; the model, endpoint, and credential passed this probe.";
  }
}

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
    noteZh: "Codex 内置连接，使用当前 Codex 登录。",
    noteEn: "Built into Codex and uses the current Codex sign-in.",
    docsUrl: "https://learn.chatgpt.com/docs/config-file/config-reference",
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
    id: "kimi",
    title: "Kimi",
    vendorZh: "月之暗面",
    vendorEn: "Moonshot AI",
    providerId: "kimi",
    name: "Kimi",
    model: "kimi-k2.6",
    baseUrl: "https://api.moonshot.cn/v1",
    envKey: "MOONSHOT_API_KEY",
    protocol: "chat_completions",
    contextWindow: 256_000,
    support: "bridge",
    noteZh: "官方 Chat 接口；支持流式输出和函数工具，经本机兼容桥接入 Codex。",
    noteEn:
      "Official Chat API with streaming and function tools, connected through the local compatibility bridge.",
    docsUrl: "https://platform.kimi.com/docs/guide/kimi-k2-6-quickstart",
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
    id: "xiaomi",
    title: "Xiaomi MiMo",
    vendorZh: "小米 MiMo",
    vendorEn: "Xiaomi MiMo",
    providerId: "xiaomi-mimo",
    name: "Xiaomi MiMo",
    model: "mimo-v2.5-pro",
    baseUrl: "https://api.xiaomimimo.com/v1",
    envKey: "MIMO_API_KEY",
    protocol: "responses",
    contextWindow: 1_000_000,
    support: "native",
    noteZh: "官方 Responses，可由 Codex 直接连接；支持流式输出、函数工具和深度思考。",
    noteEn:
      "Native Responses for direct Codex access, with streaming, function tools, and deep thinking.",
    docsUrl:
      "https://mimo.mi.com/docs/en-US/tokenplan/integration/codex-configuration",
  },
  {
    id: "custom",
    title: "Custom API",
    vendorZh: "自定义 Chat / Responses",
    vendorEn: "Custom Chat / Responses",
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
    profileName: preset.title,
    providerId: preset.providerId,
    name: preset.name,
    model:
      preset.id === "openai" && currentModel ? currentModel : preset.model,
    baseUrl: preset.baseUrl,
    envKey: preset.envKey,
    protocol: preset.protocol,
    contextWindow: preset.contextWindow,
    credentialMode: preset.id === "openai" ? "none" : "file",
    apiKey: "",
  };
}

function draftForNewProfile(
  preset: Preset,
  profiles: ProviderProfile[],
): Draft {
  const base = draftFromPreset(preset);
  if (
    preset.id === "openai" ||
    !profiles.some((profile) => profile.provider_id === preset.providerId)
  ) {
    return base;
  }

  const usedIds = new Set(profiles.map((profile) => profile.provider_id));
  let suffix = 2;
  while (usedIds.has(`${preset.providerId}-${suffix}`)) {
    suffix += 1;
  }
  return {
    ...base,
    profileName: `${preset.title} ${suffix}`,
    providerId: `${preset.providerId}-${suffix}`,
  };
}

function draftFromProvider(
  provider: ProviderDefinition,
  model: string,
): Draft {
  return {
    profileName: provider.name,
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
          : "file",
    apiKey: "",
  };
}

function draftFromProfile(profile: ProviderProfile): Draft {
  return {
    profileName: profile.name,
    providerId: profile.provider_id,
    name: profile.provider_name,
    model: profile.model,
    baseUrl: profile.base_url ?? "",
    envKey: profile.env_key ?? "",
    protocol: profile.protocol,
    contextWindow: profile.context_window,
    credentialMode:
      profile.credential_mode === "environment"
        ? "environment"
        : profile.credential_mode === "none"
          ? "none"
          : profile.builtin
            ? "none"
            : "file",
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
    return locale === "zh-CN"
      ? "请重新保存 API Key"
      : "Save the API key again";
  }
  if (provider.credential_source === "file") {
    return provider.credential_available
      ? locale === "zh-CN"
        ? "凭据文件已保存"
        : "Credential file saved"
      : locale === "zh-CN"
        ? "缺少凭据文件"
        : "Credential file missing";
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
  surface,
  hidden = false,
}: ProviderViewProps) {
  const initialSection: ControlSection =
    surface === "access" ? "provider" : "model";
  const [activeSection, setActiveSection] =
    useState<ControlSection>(initialSection);
  const [lastConfigSection, setLastConfigSection] =
    useState<ConfigCategory>("model");
  const [settingsMounted, setSettingsMounted] = useState(
    surface === "console",
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
  const activeProfile = useMemo(
    () =>
      snapshot?.profiles.find(
        (profile) =>
          profile.provider_id === snapshot.active_provider &&
          profile.model === snapshot.active_model,
      ) ?? null,
    [snapshot],
  );
  const activeCredentialLabel = activeDefinition?.builtin
    ? zh
      ? "Codex 登录 / 内置"
      : "Codex sign-in / built-in"
    : activeDefinition?.credential_source === "keychain"
      ? zh
        ? "请重新保存 API Key"
        : "Save the API key again"
      : activeDefinition?.credential_source === "file"
        ? activeDefinition.credential_available
          ? zh
            ? "凭据文件已保存"
            : "Credential file saved"
          : zh
            ? "缺少凭据文件"
            : "Credential file missing"
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
    const nextSection: ControlSection =
      surface === "access" ? "provider" : "model";
    setActiveSection(nextSection);
    if (surface === "console") {
      setLastConfigSection("model");
      setSettingsMounted(true);
    }
  }, [surface]);

  const selectPreset = (preset: Preset) => {
    setSelectedId(preset.id);
    setDraft(
      preset.id === "openai"
        ? draftFromPreset(
            preset,
            snapshot?.active_provider === preset.providerId
              ? snapshot.active_model
              : null,
          )
        : draftForNewProfile(preset, snapshot?.profiles ?? []),
    );
    setPreview(false);
    setNotice(null);
    setError(null);
  };

  const selectProfile = (profile: ProviderProfile) => {
    const preset = PRESETS.find(
      (candidate) => candidate.providerId === profile.provider_id,
    );
    setSelectedId(preset?.id ?? "custom");
    setDraft(draftFromProfile(profile));
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
    profile_name: draft.profileName.trim() || null,
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
      editingBuiltin ||
      draft.credentialMode !== "file"
        ? null
        : draft.apiKey.trim() || null,
    protocol: editingBuiltin ? "responses" : draft.protocol,
    context_window: draft.contextWindow,
    expected_version: snapshot?.version ?? null,
  };

  const targetDefinition = snapshot?.providers.find(
    (provider) => provider.id === request.provider_id,
  );
  const targetProfile = snapshot?.profiles.find(
    (profile) => profile.provider_id === request.provider_id,
  );
  const hasStoredCredential =
    (targetDefinition?.credential_source === request.credential_mode &&
      targetDefinition.credential_available) ||
    (targetProfile?.credential_mode === request.credential_mode &&
      targetProfile.credential_available);
  const credentialPath =
    targetDefinition?.credential_path ?? targetProfile?.credential_path ?? null;
  const configurationComplete =
    Boolean(request.profile_name) &&
    request.provider_id.length > 0 &&
    request.model.length > 0 &&
    (request.protocol !== "chat_completions" ||
      (request.context_window >= 8_192 &&
        request.context_window <= 4_000_000)) &&
    (editingBuiltin ||
      (Boolean(request.base_url) &&
        (request.credential_mode !== "environment" ||
          Boolean(request.env_key)) &&
        (request.credential_mode !== "file" ||
          Boolean(request.api_key) ||
          hasStoredCredential)));
  const hasProviderChanges = Boolean(
    snapshot &&
      (request.provider_id !== snapshot.active_provider ||
        request.model !== (snapshot.active_model ?? "") ||
        request.profile_name !== (targetProfile?.name ?? null) ||
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
                  : "file") ||
            Boolean(request.api_key)))),
  );
  const canPreview = configurationComplete && hasProviderChanges;
  const profileRequest = (profile: ProviderProfile): ProviderApplyRequest => ({
    profile_name: profile.name,
    provider_id: profile.provider_id,
    model: profile.model,
    name: profile.builtin ? null : profile.provider_name,
    base_url: profile.builtin ? null : profile.base_url,
    env_key:
      profile.builtin || profile.credential_mode !== "environment"
        ? null
        : profile.env_key,
    credential_mode: profile.builtin
      ? null
      : profile.credential_mode === "environment"
        ? "environment"
        : profile.credential_mode === "none"
          ? "none"
          : "file",
    api_key: null,
    protocol: profile.builtin ? "responses" : profile.protocol,
    context_window: profile.context_window,
    expected_version: snapshot?.version ?? null,
  });

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

  const saveProfile = async () => {
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      await invoke("save_provider_profile", { request });
      setDraft((current) => ({ ...current, apiKey: "" }));
      await load();
      setNotice(
        zh
          ? "方案已保存；当前 Codex 模型接入没有改变。"
          : "Profile saved; the active Codex provider was not changed.",
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  const activateProfile = async (profile: ProviderProfile) => {
    if (
      profile.provider_id === snapshot?.active_provider &&
      profile.model === snapshot?.active_model
    ) {
      selectProfile(profile);
      return;
    }
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const next = await invoke<ProviderSnapshot>("apply_provider_config", {
        request: profileRequest(profile),
      });
      setSnapshot(next);
      const activated =
        next.profiles.find(
          (candidate) => candidate.provider_id === next.active_provider,
        ) ?? profile;
      selectProfile(activated);
      setNotice(
        profile.protocol === "chat_completions"
          ? zh
            ? `已切换到“${profile.name}”；新任务将通过 X-Ray 本机 Chat 兼容桥。`
            : `Switched to “${profile.name}”; new tasks use the local X-Ray Chat bridge.`
          : zh
            ? `已切换到“${profile.name}”；新任务将使用此方案。`
            : `Switched to “${profile.name}”; new tasks will use this profile.`,
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
        credentialMode: request.credential_mode,
      });
      setDraft((current) => ({ ...current, apiKey: "" }));
      await load();
      setNotice(
        zh
          ? "已删除这套接入配置的 API Key。"
          : "The API key for this connection was removed.",
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };

  const revealCredentialFile = async () => {
    if (!credentialPath) return;
    setError(null);
    try {
      await invoke("reveal_local_path", { path: credentialPath });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
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
          <h1>
            {surface === "access"
              ? zh
                ? "模型接入"
                : "Model access"
              : zh
                ? "控制台"
                : "Console"}
          </h1>
          <span className="header-note">
            {surface === "access"
              ? zh
                ? "连接模型服务，保存多套配置并快速切换"
                : "Connect model services, save profiles, and switch quickly"
              : zh
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

      {surface === "console" && (
        <nav
          className="control-center-nav"
          aria-label={zh ? "Codex 设置分类" : "Codex setting categories"}
        >
          {(
            [
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
      )}

      {activeSection === "provider" ? (
        <>
      <section className="provider-active">
        <div>
          <p>{zh ? "当前接入" : "Active connection"}</p>
          <h2>
            {activeProfile?.name ??
              activeDefinition?.name ??
              snapshot?.active_provider ??
              "—"}
          </h2>
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
            <p>{zh ? "配置方案" : "Profiles"}</p>
            <span>
              {zh
                ? `${snapshot?.profiles.length ?? 0} 套，独立保存模型与凭据`
                : `${snapshot?.profiles.length ?? 0} saved with separate models and credentials`}
            </span>
          </div>
          <div className="provider-profile-list">
            {snapshot?.profiles.map((profile) => {
              const active =
                profile.provider_id === snapshot.active_provider &&
                profile.model === snapshot.active_model;
              const selected =
                profile.provider_id === request.provider_id &&
                profile.name === request.profile_name;
              return (
                <div
                  key={profile.id}
                  className={`provider-profile-row${active ? " active" : ""}${selected ? " selected" : ""}`}
                >
                  <button
                    type="button"
                    className="provider-profile-select"
                    onClick={() => selectProfile(profile)}
                  >
                    <span>
                      <strong>{profile.name}</strong>
                      <small>{profile.model}</small>
                    </span>
                    <span className="provider-profile-meta">
                      <em
                        title={
                          profile.protocol === "chat_completions"
                            ? zh
                              ? "由 X-Ray 把 Codex Responses 请求转换成 Chat Completions"
                              : "X-Ray translates Codex Responses requests to Chat Completions"
                            : zh
                              ? "Codex 直接使用 Responses API"
                              : "Codex uses the Responses API directly"
                        }
                      >
                        {profile.protocol === "chat_completions" ? (
                          <Workflow aria-hidden="true" />
                        ) : (
                          <MessageSquareText aria-hidden="true" />
                        )}
                        {profile.protocol === "chat_completions"
                          ? zh
                            ? "Chat 桥接"
                            : "Chat bridge"
                          : "Responses"}
                      </em>
                      {profile.protocol === "chat_completions" && (
                        <em
                          className="limited"
                          title={
                            zh
                              ? "图片输入、Codex 内置工具、加密 Reasoning 与服务端压缩受限"
                              : "Image input, Codex built-in tools, encrypted reasoning, and server-side compaction are limited"
                          }
                        >
                          <ImageOff aria-hidden="true" />
                          {zh ? "能力受限" : "Limits"}
                        </em>
                      )}
                      <small
                        className={
                          profile.credential_available ? "ready" : "missing"
                        }
                      >
                        <KeyRound aria-hidden="true" />
                        {profile.builtin
                          ? zh
                            ? "Codex 登录"
                            : "Codex sign-in"
                          : profile.credential_available
                            ? zh
                              ? "凭据可用"
                              : "Credential ready"
                            : zh
                              ? "缺少凭据"
                              : "Credential missing"}
                      </small>
                    </span>
                  </button>
                  <button
                    type="button"
                    className="provider-profile-activate"
                    disabled={active || saving || !profile.credential_available}
                    onClick={() => void activateProfile(profile)}
                    aria-label={
                      active
                        ? zh
                          ? `${profile.name} 当前已启用`
                          : `${profile.name} is active`
                        : zh
                          ? `启用 ${profile.name}`
                          : `Activate ${profile.name}`
                    }
                  >
                    {active ? (
                      <CheckCircle2 aria-hidden="true" />
                    ) : (
                      <Power aria-hidden="true" />
                    )}
                    {active ? (zh ? "使用中" : "Active") : zh ? "启用" : "Use"}
                  </button>
                </div>
              );
            })}
          </div>

          <div className="provider-template-heading">
            <span>
              <Plus aria-hidden="true" />
              {zh ? "新建方案" : "New profile"}
            </span>
            <small>
              {zh ? "选择模板后修改并保存" : "Choose a template, edit, and save"}
            </small>
          </div>
          <div className="provider-preset-list provider-template-list">
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
            <label className="provider-field-wide">
              <span>{zh ? "方案名称" : "Profile name"}</span>
              <input
                value={draft.profileName}
                onChange={(event) =>
                  setDraft({ ...draft, profileName: event.target.value })
                }
                placeholder={
                  zh
                    ? "例如：OpenAI 官方、GLM 工作账号"
                    : "For example: OpenAI Official, GLM Work"
                }
              />
              <small>
                {zh
                  ? "用于方案列表。重复选择同一模板时会自动创建独立接入 ID 和凭据。"
                  : "Shown in the profile list. Reusing a template automatically creates an independent Provider ID and credential."}
              </small>
            </label>
            <label>
              <span>{zh ? "接入 ID" : "Provider ID"}</span>
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
                    <>
                      <small
                        className={
                          bridgeStatus?.running
                            ? "provider-bridge-note running"
                            : "provider-bridge-note error"
                        }
                      >
                        {bridgeStatus?.running
                          ? zh
                            ? "本机兼容桥正在运行"
                            : "Local compatibility bridge is running"
                          : zh
                            ? `兼容桥未运行${bridgeStatus?.last_error ? `：${bridgeStatus.last_error}` : ""}`
                            : `Bridge is not running${bridgeStatus?.last_error ? `: ${bridgeStatus.last_error}` : ""}`}
                      </small>
                      <dl className="provider-bridge-capabilities">
                        <div>
                          <dt>{zh ? "已转换" : "Translated"}</dt>
                          <dd>
                            {zh
                              ? "系统与对话消息、流式文本、函数工具、并行调用、Token 用量"
                              : "System and chat messages, streaming text, function tools, parallel calls, and token usage"}
                          </dd>
                        </div>
                        <div>
                          <dt>{zh ? "边界" : "Boundary"}</dt>
                          <dd>
                            {zh
                              ? "按文本模型工作；本轮图片会被阻止，历史图片会省略。Codex 内置 Web Search、服务端压缩和加密 Reasoning 不会发送给 Chat 上游"
                              : "Runs as a text model: current images are blocked and earlier images are omitted. Codex-native web search, server compaction, and encrypted reasoning are not sent upstream"}
                          </dd>
                        </div>
                      </dl>
                    </>
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
                {draft.credentialMode === "file" && (
                  <div className="provider-auth-storage provider-field-wide">
                    <label>
                      <span>API Key</span>
                      <div className="provider-secret-input">
                        <KeyRound aria-hidden="true" />
                        <input
                          type={showApiKey ? "text" : "password"}
                          value={draft.apiKey}
                          onChange={(event) =>
                            setDraft({ ...draft, apiKey: event.target.value })
                          }
                          placeholder={
                            hasStoredCredential
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
                            showApiKey
                              ? zh
                                ? "隐藏"
                                : "Hide"
                              : zh
                                ? "显示"
                                : "Show"
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
                          {hasStoredCredential
                            ? zh
                              ? "API Key 已保存；留空不会覆盖"
                              : "API key saved; leave blank to keep it"
                            : zh
                              ? "保存在本机，不写入 Codex 配置"
                              : "Stored locally, never written to Codex configuration"}
                        </span>
                        <span className="provider-secret-actions">
                          {draft.credentialMode === "file" &&
                            hasStoredCredential &&
                            credentialPath && (
                              <button
                                type="button"
                                disabled={saving}
                                onClick={() => void revealCredentialFile()}
                              >
                                {zh ? "在访达中显示" : "Show in folder"}
                              </button>
                            )}
                          {hasStoredCredential && (
                            <button
                              type="button"
                              disabled={saving}
                              onClick={() => void clearCredential()}
                            >
                              {zh ? "删除" : "Remove"}
                            </button>
                          )}
                        </span>
                      </small>
                      {draft.credentialMode === "file" && credentialPath && (
                        <code className="provider-credential-path">
                          {credentialPath}
                        </code>
                      )}
                    </label>
                  </div>
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
                            credentialMode: "file",
                            apiKey: "",
                          })
                        }
                      >
                        {zh ? "改为凭据文件" : "Use a credential file"}
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
                            credentialMode: "file",
                            apiKey: "",
                          })
                        }
                      >
                        {zh ? "改为凭据文件" : "Use a credential file"}
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
                    ? "请补全接入名称、模型、接口地址和认证信息"
                    : "Complete the provider, model, API URL, and authentication"
                  : !hasProviderChanges
                    ? zh
                      ? "方案已保存且当前正在使用"
                      : "This saved profile is already active"
                    : zh
                      ? "可以只保存方案，或预览后启用"
                      : "Save the profile only, or preview and activate it"}
              </span>
              <div>
                <button
                  className="secondary-action"
                  disabled={
                    !configurationComplete || loading || saving || testing
                  }
                  onClick={() => void saveProfile()}
                >
                  <Save aria-hidden="true" />
                  {saving
                    ? zh
                      ? "保存中…"
                      : "Saving…"
                    : zh
                      ? "保存方案"
                      : "Save profile"}
                </button>
                <button
                  className="secondary-action"
                  disabled={
                    !configurationComplete || loading || saving || testing
                  }
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
                  disabled={!canPreview || loading || saving || testing}
                  onClick={() => {
                    setPreview(true);
                    setError(null);
                    setNotice(null);
                  }}
                >
                  {zh ? "预览并启用" : "Preview & activate"}
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
                    {request.profile_name} · {request.provider_id} /{" "}
                    {request.model}
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
                  {request.credential_mode === "file"
                      ? zh
                        ? "API Key 已保存在本机"
                        : "API key stored locally"
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
                      ? "保存并启用"
                      : "Save & activate"}
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
              <div className="provider-test-summary">
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
                <span>{providerTestMessage(testResult, zh)}</span>
                {(testResult.check_kind === "responses_request" ||
                  testResult.check_kind === "chat_completions_request") && (
                  <small>
                    {zh
                      ? "已发送一次最小生成请求，可能产生极少量 API 费用。"
                      : "A minimal generation request was sent and may incur a very small API charge."}
                  </small>
                )}
              </div>
              <div className="provider-capability-report">
                <header>
                  <strong>{zh ? "能力与限制" : "Capabilities & limits"}</strong>
                  <span>
                    {zh
                      ? "仅明确标注“已实测”的项目由本次请求验证"
                      : "Only items marked “Verified” were exercised by this probe"}
                  </span>
                </header>
                <div role="list">
                  {testResult.capabilities.map((item) => (
                    <div
                      key={item.capability}
                      className={`provider-capability-row status-${item.status}`}
                      role="listitem"
                    >
                      <CapabilityIcon capability={item.capability} />
                      <strong>{capabilityTitle(item.capability, zh)}</strong>
                      <span>{capabilityDetail(item, zh)}</span>
                      <small>{capabilityStatus(item.status, zh)}</small>
                    </div>
                  ))}
                </div>
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
          <strong>{zh ? "已有接入配置" : "Configured providers"}</strong>
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
            {zh ? "没有读到模型接入配置。" : "No provider configuration found."}
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
