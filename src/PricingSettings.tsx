import { invoke } from "@tauri-apps/api/core";
import {
  BadgeDollarSign,
  Plus,
  RotateCcw,
  Settings2,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Locale } from "./i18n";
import type {
  ModelCostEstimate,
  PricingConfigSnapshot,
  PricingOverride,
  PricingRateDefinition,
} from "./types";

type PricingSettingsProps = {
  locale: Locale;
  observedModels: ModelCostEstimate[];
  onApplied: () => Promise<void> | void;
};

type DraftRow = {
  key: string;
  model: string;
  input: string;
  cachedInput: string;
  output: string;
  customized: boolean;
  observed: boolean;
  manuallyAdded: boolean;
  defaultRate: PricingRateDefinition | null;
};

type RateField = "input" | "cachedInput" | "output";

function canonicalModel(model: string) {
  return model
    .trim()
    .replace(/-\d{4}-\d{2}-\d{2}$/u, "")
    .toLocaleLowerCase();
}

function rateText(value: number) {
  return Number.isInteger(value) ? String(value) : String(Number(value.toFixed(6)));
}

function buildRows(
  snapshot: PricingConfigSnapshot,
  observedModels: ModelCostEstimate[],
): DraftRow[] {
  const defaults = new Map(
    snapshot.defaults.map((rate) => [canonicalModel(rate.model), rate]),
  );
  const overrides = new Map(
    snapshot.overrides.map((rate) => [canonicalModel(rate.model), rate]),
  );
  const rows = new Map<string, DraftRow>();

  for (const observed of observedModels) {
    const key = canonicalModel(observed.model);
    if (!key || rows.has(key)) continue;
    const custom = overrides.get(key);
    const fallback = defaults.get(key) ?? null;
    const effective = custom ?? fallback;
    rows.set(key, {
      key,
      model: custom?.model ?? observed.model,
      input: effective ? rateText(effective.input_per_million) : "",
      cachedInput: effective
        ? rateText(effective.cached_input_per_million)
        : "",
      output: effective ? rateText(effective.output_per_million) : "",
      customized: Boolean(custom),
      observed: true,
      manuallyAdded: false,
      defaultRate: fallback,
    });
  }

  for (const custom of snapshot.overrides) {
    const key = canonicalModel(custom.model);
    if (!key || rows.has(key)) continue;
    rows.set(key, {
      key,
      model: custom.model,
      input: rateText(custom.input_per_million),
      cachedInput: rateText(custom.cached_input_per_million),
      output: rateText(custom.output_per_million),
      customized: true,
      observed: false,
      manuallyAdded: true,
      defaultRate: defaults.get(key) ?? null,
    });
  }

  if (rows.size === 0) {
    for (const fallback of snapshot.defaults) {
      const key = canonicalModel(fallback.model);
      rows.set(key, {
        key,
        model: fallback.model,
        input: rateText(fallback.input_per_million),
        cachedInput: rateText(fallback.cached_input_per_million),
        output: rateText(fallback.output_per_million),
        customized: false,
        observed: false,
        manuallyAdded: false,
        defaultRate: fallback,
      });
    }
  }

  return [...rows.values()].sort((left, right) => {
    if (left.observed !== right.observed) return left.observed ? -1 : 1;
    return left.model.localeCompare(right.model);
  });
}

function parseRate(value: string) {
  if (value.trim() === "") return null;
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0 || parsed > 1_000_000) {
    return null;
  }
  return parsed;
}

function todayDate() {
  const now = new Date();
  const local = new Date(now.getTime() - now.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 10);
}

export default function PricingSettings({
  locale,
  observedModels,
  onApplied,
}: PricingSettingsProps) {
  const zh = locale === "zh-CN";
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [snapshot, setSnapshot] = useState<PricingConfigSnapshot | null>(null);
  const [rows, setRows] = useState<DraftRow[]>([]);
  const [newModel, setNewModel] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmReset, setConfirmReset] = useState(false);
  const [effectiveFrom, setEffectiveFrom] = useState(todayDate);

  const invalidRows = useMemo(
    () =>
      rows.filter(
        (row) =>
          row.customized &&
          (parseRate(row.input) === null ||
            parseRate(row.cachedInput) === null ||
            parseRate(row.output) === null),
      ),
    [rows],
  );

  const customizedCount = rows.filter((row) => row.customized).length;

  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !saving) setOpen(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [open, saving]);

  async function loadConfig() {
    setLoading(true);
    setError(null);
    setNotice(null);
    try {
      const next = await invoke<PricingConfigSnapshot>("get_pricing_config");
      setSnapshot(next);
      setRows(buildRows(next, observedModels));
      setEffectiveFrom(todayDate());
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  }

  async function showDialog() {
    setOpen(true);
    setConfirmReset(false);
    await loadConfig();
  }

  function updateRate(key: string, field: RateField, value: string) {
    setRows((current) =>
      current.map((row) =>
        row.key === key
          ? { ...row, [field]: value, customized: true }
          : row,
      ),
    );
    setNotice(null);
  }

  function resetRow(key: string) {
    setRows((current) =>
      current.flatMap((row) => {
        if (row.key !== key) return [row];
        if (!row.defaultRate && row.manuallyAdded && !row.observed) return [];
        return [
          {
            ...row,
            input: row.defaultRate
              ? rateText(row.defaultRate.input_per_million)
              : "",
            cachedInput: row.defaultRate
              ? rateText(row.defaultRate.cached_input_per_million)
              : "",
            output: row.defaultRate
              ? rateText(row.defaultRate.output_per_million)
              : "",
            customized: false,
          },
        ];
      }),
    );
    setNotice(null);
  }

  function addModel() {
    const model = newModel.trim();
    const key = canonicalModel(model);
    if (!key) {
      setError(zh ? "请先输入模型 ID。" : "Enter a model ID first.");
      return;
    }
    if (rows.some((row) => row.key === key)) {
      setError(
        zh ? "这个模型已经在列表中。" : "This model is already in the list.",
      );
      return;
    }
    const fallback =
      snapshot?.defaults.find(
        (candidate) => canonicalModel(candidate.model) === key,
      ) ?? null;
    setRows((current) => [
      ...current,
      {
        key,
        model,
        input: fallback ? rateText(fallback.input_per_million) : "",
        cachedInput: fallback
          ? rateText(fallback.cached_input_per_million)
          : "",
        output: fallback ? rateText(fallback.output_per_million) : "",
        customized: !fallback,
        observed: false,
        manuallyAdded: true,
        defaultRate: fallback,
      },
    ]);
    setNewModel("");
    setError(null);
    setNotice(null);
  }

  async function save() {
    if (invalidRows.length > 0) {
      setError(
        zh
          ? "请补全标红模型的三个单价；允许填 0。"
          : "Complete all three prices in the highlighted rows; zero is allowed.",
      );
      return;
    }
    const overrides: PricingOverride[] = rows
      .filter((row) => row.customized)
      .map((row) => ({
        model: row.model,
        input_per_million: parseRate(row.input) as number,
        cached_input_per_million: parseRate(row.cachedInput) as number,
        output_per_million: parseRate(row.output) as number,
      }));
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const next = await invoke<PricingConfigSnapshot>(
        "apply_pricing_config",
        { request: { overrides, effective_from: effectiveFrom } },
      );
      setSnapshot(next);
      setRows(buildRows(next, observedModels));
      await onApplied();
      setNotice(
        zh
          ? `已保存 ${effectiveFrom} 起生效的单价版本；更早记录继续使用当时版本。`
          : `Saved a price version effective ${effectiveFrom}; earlier records keep their previous rates.`,
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  }

  async function resetAll() {
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const next = await invoke<PricingConfigSnapshot>(
        "reset_pricing_config",
      );
      setSnapshot(next);
      setRows(buildRows(next, observedModels));
      setConfirmReset(false);
      await onApplied();
      setNotice(
        zh
          ? "已新增从今天起恢复公开默认单价的版本；过去记录保持原价格。"
          : "Added a version that restores public defaults from today; earlier records keep their prior rates.",
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      <button
        type="button"
        className="pricing-settings-trigger"
        onClick={showDialog}
      >
        <Settings2 aria-hidden="true" />
        {zh ? "单价设置" : "Pricing"}
      </button>

      {open && (
        <div
          className="pricing-dialog-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget && !saving) setOpen(false);
          }}
        >
          <section
            className="pricing-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="pricing-dialog-title"
          >
            <header>
              <div className="pricing-dialog-title">
                <span className="pricing-dialog-icon">
                  <BadgeDollarSign aria-hidden="true" />
                </span>
                <div>
                  <p>API-EQUIVALENT PRICING</p>
                  <h2 id="pricing-dialog-title">
                    {zh ? "成本估算单价" : "Cost estimation prices"}
                  </h2>
                  <span>
                    {zh
                      ? "单位：USD / 100 万 Token"
                      : "Unit: USD per 1 million tokens"}
                  </span>
                </div>
              </div>
              <button
                type="button"
                className="dialog-close"
                aria-label={zh ? "关闭单价设置" : "Close pricing settings"}
                onClick={() => setOpen(false)}
                disabled={saving}
              >
                <X aria-hidden="true" />
              </button>
            </header>

            <div className="pricing-dialog-body">
              <div className="pricing-explainer">
              <strong>
                {zh
                  ? "按事件日期选择单价版本"
                  : "Rates are selected by each event date"}
              </strong>
              <p>
                {zh
                  ? "保存新价格不会改写旧月份；这仍是“如果改走 API”的等价估算，不是 Codex 套餐账单。"
                  : "New prices do not rewrite earlier months. This remains an API-equivalent estimate, not a Codex plan bill."}
              </p>
              </div>

              <div className="pricing-version-controls">
              <label htmlFor="pricing-effective-from">
                <span>{zh ? "生效日期" : "Effective date"}</span>
                <input
                  id="pricing-effective-from"
                  type="date"
                  value={effectiveFrom}
                  max={todayDate()}
                  onChange={(event) => setEffectiveFrom(event.target.value)}
                />
              </label>
              <p>
                {zh
                  ? "同一天再次保存会替换当天版本；之前日期不受影响。"
                  : "Saving again for the same date replaces that day's version; earlier dates are unchanged."}
              </p>
              </div>

              <div className="pricing-add-model">
              <label htmlFor="pricing-new-model">
                {zh ? "添加其他模型" : "Add another model"}
              </label>
              <div>
                <input
                  id="pricing-new-model"
                  value={newModel}
                  placeholder={zh ? "例如 glm-5" : "For example, glm-5"}
                  onChange={(event) => setNewModel(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") addModel();
                  }}
                />
                <button type="button" onClick={addModel}>
                  <Plus aria-hidden="true" />
                  {zh ? "添加" : "Add"}
                </button>
              </div>
              </div>

              <div className="pricing-table" aria-busy={loading}>
              <div className="pricing-table-header" aria-hidden="true">
                <span>{zh ? "模型" : "Model"}</span>
                <span>{zh ? "输入" : "Input"}</span>
                <span>{zh ? "缓存输入" : "Cached input"}</span>
                <span>{zh ? "输出" : "Output"}</span>
                <span>{zh ? "操作" : "Action"}</span>
              </div>
              <div className="pricing-table-body">
                {loading ? (
                  <div className="pricing-loading">
                    {zh ? "正在读取单价设置…" : "Loading pricing settings…"}
                  </div>
                ) : (
                  rows.map((row) => {
                    const invalid = invalidRows.some(
                      (candidate) => candidate.key === row.key,
                    );
                    return (
                      <div
                        className={`pricing-row${invalid ? " invalid" : ""}`}
                        key={row.key}
                      >
                        <div className="pricing-model">
                          <code>{row.model}</code>
                          <span
                            className={
                              row.customized
                                ? "pricing-source custom"
                                : row.defaultRate
                                  ? "pricing-source"
                                  : "pricing-source unpriced"
                            }
                          >
                            {row.customized
                              ? zh
                                ? "自定义固定价"
                                : "Custom flat rate"
                              : row.defaultRate
                                ? row.defaultRate.has_long_context_tier
                                  ? zh
                                    ? "公开默认 · 含长上下文档"
                                    : "Public default · long-context tier"
                                  : zh
                                    ? "公开默认"
                                    : "Public default"
                                : zh
                                  ? "未定价"
                                  : "Unpriced"}
                          </span>
                        </div>
                        {(
                          [
                            ["input", row.input, zh ? "输入" : "input"],
                            [
                              "cachedInput",
                              row.cachedInput,
                              zh ? "缓存输入" : "cached input",
                            ],
                            ["output", row.output, zh ? "输出" : "output"],
                          ] as const
                        ).map(([field, value, label]) => (
                          <label key={field}>
                            <span>{label}</span>
                            <input
                              type="number"
                              min="0"
                              max="1000000"
                              step="0.001"
                              inputMode="decimal"
                              value={value}
                              placeholder="—"
                              aria-label={`${row.model} ${label}, USD / 1M Token`}
                              aria-invalid={invalid}
                              onChange={(event) =>
                                updateRate(
                                  row.key,
                                  field as RateField,
                                  event.target.value,
                                )
                              }
                            />
                          </label>
                        ))}
                        <button
                          type="button"
                          className="pricing-row-reset"
                          aria-label={
                            row.defaultRate || row.observed
                              ? `${zh ? "恢复默认" : "Restore default"} ${row.model}`
                              : `${zh ? "删除" : "Remove"} ${row.model}`
                          }
                          onClick={() => resetRow(row.key)}
                          disabled={!row.customized && !row.manuallyAdded}
                        >
                          {row.defaultRate || row.observed ? (
                            <RotateCcw aria-hidden="true" />
                          ) : (
                            <Trash2 aria-hidden="true" />
                          )}
                        </button>
                      </div>
                    );
                  })
                )}
              </div>
              </div>

              <div className="pricing-dialog-meta">
              <span>
                {zh
                  ? `内置单价快照：${snapshot?.defaults_updated_at ?? "—"}`
                  : `Built-in price snapshot: ${snapshot?.defaults_updated_at ?? "—"}`}
              </span>
              <span title={snapshot?.config_path ?? ""}>
                {zh
                  ? `${snapshot?.versions.length ?? 0} 个历史版本 · 当前自定义 ${customizedCount} 个模型`
                  : `${snapshot?.versions.length ?? 0} historical version${snapshot?.versions.length === 1 ? "" : "s"} · ${customizedCount} current custom model${customizedCount === 1 ? "" : "s"}`}
              </span>
              </div>

              {(snapshot?.versions.length ?? 0) > 0 && (
                <details className="pricing-version-history">
                <summary>
                  {zh
                    ? `查看 ${snapshot?.versions.length ?? 0} 个价格版本`
                    : `View ${snapshot?.versions.length ?? 0} price version${snapshot?.versions.length === 1 ? "" : "s"}`}
                </summary>
                <ol>
                  {[...(snapshot?.versions ?? [])]
                    .reverse()
                    .map((version) => (
                      <li key={`${version.effective_from}-${version.created_at}`}>
                        <strong>{version.effective_from}</strong>
                        <span>
                          {version.overrides.length > 0
                            ? zh
                              ? `${version.overrides.length} 个自定义模型`
                              : `${version.overrides.length} custom model${version.overrides.length === 1 ? "" : "s"}`
                            : zh
                              ? "恢复公开默认单价"
                              : "Public defaults restored"}
                        </span>
                      </li>
                    ))}
                </ol>
                </details>
              )}

              {(error || notice) && (
                <p
                  className={error ? "pricing-message error" : "pricing-message"}
                  role={error ? "alert" : "status"}
                  aria-live="polite"
                >
                  {error ?? notice}
                </p>
              )}
            </div>

            <footer>
              <div className="pricing-reset-all">
                {confirmReset ? (
                  <>
                    <span>
                      {zh
                        ? "清除全部自定义单价？"
                        : "Clear all custom prices?"}
                    </span>
                    <button type="button" onClick={resetAll} disabled={saving}>
                      {zh ? "确认恢复" : "Confirm"}
                    </button>
                    <button
                      type="button"
                      onClick={() => setConfirmReset(false)}
                      disabled={saving}
                    >
                      {zh ? "取消" : "Cancel"}
                    </button>
                  </>
                ) : (
                  <button
                    type="button"
                    onClick={() => setConfirmReset(true)}
                    disabled={saving || (customizedCount === 0 && !error)}
                  >
                    <RotateCcw aria-hidden="true" />
                    {zh ? "全部恢复默认" : "Restore all defaults"}
                  </button>
                )}
              </div>
              <button
                type="button"
                className="pricing-cancel"
                onClick={() => setOpen(false)}
                disabled={saving}
              >
                {zh ? "关闭" : "Close"}
              </button>
              <button
                type="button"
                className="primary-action"
                onClick={save}
                disabled={
                  saving ||
                  loading ||
                  invalidRows.length > 0 ||
                  !effectiveFrom
                }
              >
                {saving
                  ? zh
                    ? "保存并重算中…"
                    : "Saving & recalculating…"
                  : zh
                    ? "保存并重新计算"
                    : "Save & recalculate"}
              </button>
            </footer>
          </section>
        </div>
      )}
    </>
  );
}
