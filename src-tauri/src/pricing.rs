use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::{OnceLock, RwLock};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

const PRICING_CONFIG_SCHEMA_VERSION: u32 = 2;
pub const DEFAULT_PRICING_UPDATED_AT: &str = "2026-07-26";
pub const DEFAULT_PRICING_BASIS: &str = "OpenAI Standard API 公开单价快照";

/// USD cost estimate for one Codex token event.
///
/// Rates are stored per token. The source session already reports reasoning
/// tokens inside output tokens, so reasoning is never charged a second time.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct EstimatedCost {
    pub uncached_input_usd: f64,
    pub cached_input_usd: f64,
    pub output_usd: f64,
    pub total_usd: f64,
    pub cache_savings_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PricingOverride {
    pub model: String,
    pub input_per_million: f64,
    pub cached_input_per_million: f64,
    pub output_per_million: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PricingVersion {
    pub effective_from: String,
    pub created_at: String,
    pub overrides: Vec<PricingOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PricingRateDefinition {
    pub model: String,
    pub input_per_million: f64,
    pub cached_input_per_million: f64,
    pub output_per_million: f64,
    pub has_long_context_tier: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingConfigSnapshot {
    pub config_path: String,
    pub updated_at: Option<String>,
    pub defaults_updated_at: String,
    pub overrides: Vec<PricingOverride>,
    pub versions: Vec<PricingVersion>,
    pub defaults: Vec<PricingRateDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingApplyRequest {
    pub overrides: Vec<PricingOverride>,
    pub effective_from: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PricingConfigFile {
    schema_version: u32,
    updated_at: Option<String>,
    #[serde(default)]
    overrides: Vec<PricingOverride>,
    #[serde(default)]
    versions: Vec<PricingVersion>,
}

#[derive(Debug, Clone, Default)]
struct ActivePricing {
    updated_at: Option<String>,
    versions: Vec<PricingVersion>,
}

static ACTIVE_PRICING: OnceLock<RwLock<ActivePricing>> = OnceLock::new();

#[derive(Clone, Copy)]
struct Rates {
    input: f64,
    cached_input: f64,
    output: f64,
    long_context: Option<LongContextRates>,
}

#[derive(Clone, Copy)]
struct LongContextRates {
    threshold: u64,
    input: f64,
    cached_input: f64,
    output: f64,
}

const OPENAI_LONG_CONTEXT_THRESHOLD: u64 = 272_000;

/// Estimate the Standard-tier API-equivalent cost for a Codex usage event.
///
/// This intentionally does not claim to be a bill. ChatGPT plan usage, Credits,
/// negotiated enterprise pricing, and Priority/Fast multipliers are separate.
#[cfg(test)]
pub fn estimate_standard_api_cost(
    model: &str,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
) -> Option<EstimatedCost> {
    estimate_standard_api_cost_on(
        model,
        input_tokens,
        cached_input_tokens,
        output_tokens,
        None,
    )
}

pub fn estimate_standard_api_cost_at(
    model: &str,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    timestamp_ms: i64,
) -> Option<EstimatedCost> {
    estimate_standard_api_cost_on(
        model,
        input_tokens,
        cached_input_tokens,
        output_tokens,
        Some(timestamp_ms),
    )
}

fn estimate_standard_api_cost_on(
    model: &str,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    timestamp_ms: Option<i64>,
) -> Option<EstimatedCost> {
    let rates =
        active_pricing_override(model, timestamp_ms).or_else(|| pricing_for_model(model))?;
    let cached = cached_input_tokens.min(input_tokens);
    let uncached = input_tokens.saturating_sub(cached);
    let active = rates
        .long_context
        .filter(|long| input_tokens > long.threshold);
    let input_rate = active.map_or(rates.input, |long| long.input);
    let cached_rate = active.map_or(rates.cached_input, |long| long.cached_input);
    let output_rate = active.map_or(rates.output, |long| long.output);

    let uncached_input_usd = uncached as f64 * input_rate;
    let cached_input_usd = cached as f64 * cached_rate;
    let output_usd = output_tokens as f64 * output_rate;
    let cache_savings_usd = cached as f64 * (input_rate - cached_rate).max(0.0);

    Some(EstimatedCost {
        uncached_input_usd,
        cached_input_usd,
        output_usd,
        total_usd: uncached_input_usd + cached_input_usd + output_usd,
        cache_savings_usd,
    })
}

pub fn pricing_config_snapshot(path: &Path) -> Result<PricingConfigSnapshot, String> {
    let config = read_pricing_config(path)?;
    let overrides = current_overrides(&config.versions);
    Ok(PricingConfigSnapshot {
        config_path: path.to_string_lossy().into_owned(),
        updated_at: config.updated_at,
        defaults_updated_at: DEFAULT_PRICING_UPDATED_AT.to_string(),
        overrides,
        versions: config.versions,
        defaults: default_pricing_definitions(),
    })
}

pub fn activate_pricing_config(path: &Path) -> Result<(), String> {
    let config = read_pricing_config(path)?;
    set_active_pricing(config.updated_at, config.versions);
    Ok(())
}

pub fn save_pricing_config(
    path: &Path,
    request: PricingApplyRequest,
) -> Result<PricingConfigSnapshot, String> {
    let overrides = validate_overrides(request.overrides)?;
    let effective_from = validate_effective_date(&request.effective_from)?;
    let updated_at = Utc::now().to_rfc3339();
    let mut previous = read_pricing_config(path)?;
    previous
        .versions
        .retain(|version| version.effective_from != effective_from);
    previous.versions.push(PricingVersion {
        effective_from,
        created_at: updated_at.clone(),
        overrides,
    });
    previous.versions.sort_by(|left, right| {
        left.effective_from
            .cmp(&right.effective_from)
            .then_with(|| left.created_at.cmp(&right.created_at))
    });
    let config = PricingConfigFile {
        schema_version: PRICING_CONFIG_SCHEMA_VERSION,
        updated_at: Some(updated_at.clone()),
        overrides: Vec::new(),
        versions: previous.versions,
    };
    let content = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("单价设置无法序列化：{error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "单价设置路径没有父目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建单价设置目录：{error}"))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, content).map_err(|error| format!("无法暂存单价设置：{error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("无法保存单价设置：{error}"))?;
    set_active_pricing(Some(updated_at), config.versions.clone());
    pricing_config_snapshot(path)
}

pub fn reset_pricing_config(path: &Path) -> Result<PricingConfigSnapshot, String> {
    save_pricing_config(
        path,
        PricingApplyRequest {
            overrides: Vec::new(),
            effective_from: Utc::now().date_naive().to_string(),
        },
    )
}

pub fn pricing_metadata() -> (String, String) {
    let active = ACTIVE_PRICING.get_or_init(|| RwLock::new(ActivePricing::default()));
    let guard = active.read().ok();
    let customized = guard
        .as_ref()
        .is_some_and(|pricing| !current_overrides(&pricing.versions).is_empty());
    let basis = if customized {
        format!("{DEFAULT_PRICING_BASIS} + 用户自定义版本化单价")
    } else {
        DEFAULT_PRICING_BASIS.to_string()
    };
    let updated_at = if customized {
        guard
            .and_then(|pricing| pricing.updated_at.clone())
            .unwrap_or_else(|| DEFAULT_PRICING_UPDATED_AT.to_string())
    } else {
        DEFAULT_PRICING_UPDATED_AT.to_string()
    };
    (basis, updated_at)
}

fn openai_rates(input_per_million: f64, cached_per_million: f64, output_per_million: f64) -> Rates {
    Rates {
        input: input_per_million / 1_000_000.0,
        cached_input: cached_per_million / 1_000_000.0,
        output: output_per_million / 1_000_000.0,
        long_context: None,
    }
}

fn with_long_context(
    mut rates: Rates,
    input_per_million: f64,
    cached_per_million: f64,
    output_per_million: f64,
) -> Rates {
    rates.long_context = Some(LongContextRates {
        threshold: OPENAI_LONG_CONTEXT_THRESHOLD,
        input: input_per_million / 1_000_000.0,
        cached_input: cached_per_million / 1_000_000.0,
        output: output_per_million / 1_000_000.0,
    });
    rates
}

fn active_pricing_override(raw_model: &str, timestamp_ms: Option<i64>) -> Option<Rates> {
    let active = ACTIVE_PRICING.get_or_init(|| RwLock::new(ActivePricing::default()));
    let guard = active.read().ok()?;
    let event_date = timestamp_ms
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|timestamp| timestamp.date_naive().to_string())
        .unwrap_or_else(|| Utc::now().date_naive().to_string());
    let custom = pricing_override_for_versions(&guard.versions, raw_model, &event_date)?;
    Some(openai_rates(
        custom.input_per_million,
        custom.cached_input_per_million,
        custom.output_per_million,
    ))
}

fn pricing_override_for_versions<'a>(
    versions: &'a [PricingVersion],
    raw_model: &str,
    event_date: &str,
) -> Option<&'a PricingOverride> {
    let key = normalize_model(raw_model);
    versions
        .iter()
        .rev()
        .find(|version| version.effective_from.as_str() <= event_date)?
        .overrides
        .iter()
        .find(|candidate| normalize_model(&candidate.model) == key)
}

fn set_active_pricing(updated_at: Option<String>, versions: Vec<PricingVersion>) {
    let active = ACTIVE_PRICING.get_or_init(|| RwLock::new(ActivePricing::default()));
    if let Ok(mut guard) = active.write() {
        *guard = ActivePricing {
            updated_at,
            versions,
        };
    }
}

fn read_pricing_config(path: &Path) -> Result<PricingConfigFile, String> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PricingConfigFile {
                schema_version: PRICING_CONFIG_SCHEMA_VERSION,
                ..PricingConfigFile::default()
            });
        }
        Err(error) => return Err(format!("无法读取单价设置：{error}")),
    };
    let config = serde_json::from_slice::<PricingConfigFile>(&content)
        .map_err(|error| format!("单价设置格式无效：{error}"))?;
    if !matches!(config.schema_version, 1 | PRICING_CONFIG_SCHEMA_VERSION) {
        return Err(format!("不支持的单价设置版本：{}", config.schema_version));
    }
    let mut versions = if config.schema_version == 1 {
        let overrides = validate_overrides(config.overrides)?;
        if overrides.is_empty() {
            Vec::new()
        } else {
            vec![PricingVersion {
                effective_from: "1970-01-01".to_string(),
                created_at: config
                    .updated_at
                    .clone()
                    .unwrap_or_else(|| Utc::now().to_rfc3339()),
                overrides,
            }]
        }
    } else {
        validate_versions(config.versions)?
    };
    versions.sort_by(|left, right| {
        left.effective_from
            .cmp(&right.effective_from)
            .then_with(|| left.created_at.cmp(&right.created_at))
    });
    Ok(PricingConfigFile {
        schema_version: PRICING_CONFIG_SCHEMA_VERSION,
        updated_at: config.updated_at,
        overrides: Vec::new(),
        versions,
    })
}

fn validate_effective_date(value: &str) -> Result<String, String> {
    let value = value.trim();
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|date| date.to_string())
        .map_err(|_| "单价生效日期必须是 YYYY-MM-DD。".to_string())
}

fn validate_versions(versions: Vec<PricingVersion>) -> Result<Vec<PricingVersion>, String> {
    let mut seen = HashSet::new();
    let mut validated = Vec::with_capacity(versions.len());
    for version in versions {
        let effective_from = validate_effective_date(&version.effective_from)?;
        if !seen.insert(effective_from.clone()) {
            return Err(format!("单价生效日期重复：{effective_from}"));
        }
        DateTime::parse_from_rfc3339(&version.created_at)
            .map_err(|_| format!("单价版本时间无效：{}", version.created_at))?;
        validated.push(PricingVersion {
            effective_from,
            created_at: version.created_at,
            overrides: validate_overrides(version.overrides)?,
        });
    }
    Ok(validated)
}

fn current_overrides(versions: &[PricingVersion]) -> Vec<PricingOverride> {
    let today = Utc::now().date_naive().to_string();
    versions
        .iter()
        .rev()
        .find(|version| version.effective_from.as_str() <= today.as_str())
        .map(|version| version.overrides.clone())
        .unwrap_or_default()
}

fn validate_overrides(overrides: Vec<PricingOverride>) -> Result<Vec<PricingOverride>, String> {
    let mut seen = HashSet::new();
    let mut validated = Vec::with_capacity(overrides.len());
    for mut item in overrides {
        item.model = item.model.trim().to_string();
        if item.model.is_empty() {
            return Err("模型 ID 不能为空".to_string());
        }
        if item.model.len() > 200 {
            return Err(format!("模型 ID 过长：{}", item.model));
        }
        let key = normalize_model(&item.model);
        if !seen.insert(key) {
            return Err(format!("模型单价重复：{}", item.model));
        }
        for (label, value) in [
            ("输入", item.input_per_million),
            ("缓存输入", item.cached_input_per_million),
            ("输出", item.output_per_million),
        ] {
            if !value.is_finite() || !(0.0..=1_000_000.0).contains(&value) {
                return Err(format!(
                    "{} 的{label}单价必须在 0 到 1,000,000 USD / 1M Token 之间",
                    item.model
                ));
            }
        }
        validated.push(item);
    }
    validated.sort_by(|left, right| left.model.cmp(&right.model));
    Ok(validated)
}

fn normalize_model(model: &str) -> String {
    strip_date_suffix(model.trim()).to_ascii_lowercase()
}

fn default_pricing_definitions() -> Vec<PricingRateDefinition> {
    [
        "gpt-5",
        "gpt-5.1",
        "gpt-5.1-codex",
        "gpt-5.2",
        "gpt-5.2-codex",
        "gpt-5.3-codex",
        "gpt-5.4",
        "gpt-5.4-mini",
        "gpt-5.4-nano",
        "gpt-5.5",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
    ]
    .into_iter()
    .filter_map(|model| {
        let rates = pricing_for_model(model)?;
        Some(PricingRateDefinition {
            model: model.to_string(),
            input_per_million: rates.input * 1_000_000.0,
            cached_input_per_million: rates.cached_input * 1_000_000.0,
            output_per_million: rates.output * 1_000_000.0,
            has_long_context_tier: rates.long_context.is_some(),
        })
    })
    .collect()
}

fn pricing_for_model(raw_model: &str) -> Option<Rates> {
    let normalized = normalize_model(raw_model);
    let model = normalized.as_str();
    match model {
        "gpt-5" => Some(openai_rates(1.25, 0.125, 10.0)),
        "gpt-5.1" | "gpt-5.1-codex" => Some(openai_rates(1.25, 0.125, 10.0)),
        "gpt-5.2" | "gpt-5.2-codex" | "gpt-5.3-codex" => Some(openai_rates(1.75, 0.175, 14.0)),
        "gpt-5.4" => Some(with_long_context(
            openai_rates(2.5, 0.25, 15.0),
            5.0,
            0.5,
            22.5,
        )),
        "gpt-5.4-mini" => Some(openai_rates(0.75, 0.075, 4.5)),
        "gpt-5.4-nano" => Some(openai_rates(0.2, 0.02, 1.25)),
        "gpt-5.5" => Some(with_long_context(
            openai_rates(5.0, 0.5, 30.0),
            10.0,
            1.0,
            45.0,
        )),
        "gpt-5.6-sol" => Some(with_long_context(
            openai_rates(5.0, 0.5, 30.0),
            10.0,
            1.0,
            45.0,
        )),
        "gpt-5.6-terra" => Some(with_long_context(
            openai_rates(2.5, 0.25, 15.0),
            5.0,
            0.5,
            22.5,
        )),
        "gpt-5.6-luna" => Some(with_long_context(
            openai_rates(1.0, 0.1, 6.0),
            2.0,
            0.2,
            9.0,
        )),
        _ => None,
    }
}

fn strip_date_suffix(model: &str) -> &str {
    let bytes = model.as_bytes();
    if bytes.len() >= 11 {
        let suffix = &bytes[bytes.len() - 11..];
        if suffix[0] == b'-'
            && suffix[1..5].iter().all(u8::is_ascii_digit)
            && suffix[5] == b'-'
            && suffix[6..8].iter().all(u8::is_ascii_digit)
            && suffix[8] == b'-'
            && suffix[9..11].iter().all(u8::is_ascii_digit)
        {
            return &model[..model.len() - 11];
        }
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_cached_input_separately() {
        let cost =
            estimate_standard_api_cost("gpt-5.4", 1_000_000, 900_000, 10_000).expect("known model");
        // Long-context pricing applies to the entire request.
        assert!((cost.uncached_input_usd - 0.5).abs() < 1e-9);
        assert!((cost.cached_input_usd - 0.45).abs() < 1e-9);
        assert!((cost.output_usd - 0.225).abs() < 1e-9);
        assert!((cost.cache_savings_usd - 4.05).abs() < 1e-9);
    }

    #[test]
    fn accepts_dated_model_names() {
        assert!(estimate_standard_api_cost("gpt-5.6-sol-2026-06-18", 100, 50, 10).is_some());
    }

    #[test]
    fn leaves_unpublished_models_unpriced() {
        assert!(estimate_standard_api_cost("gpt-5.3-codex-spark-preview", 100, 50, 10).is_none());
    }

    #[test]
    fn validates_and_canonicalizes_custom_pricing_keys() {
        let result = validate_overrides(vec![
            PricingOverride {
                model: " glm-5 ".to_string(),
                input_per_million: 1.0,
                cached_input_per_million: 0.1,
                output_per_million: 8.0,
            },
            PricingOverride {
                model: "gpt-5.6-sol".to_string(),
                input_per_million: 3.0,
                cached_input_per_million: 0.3,
                output_per_million: 18.0,
            },
        ])
        .expect("valid pricing");
        assert_eq!(result[0].model, "glm-5");
        assert_eq!(result[1].model, "gpt-5.6-sol");

        let duplicate = validate_overrides(vec![
            PricingOverride {
                model: "gpt-5.6-sol".to_string(),
                input_per_million: 1.0,
                cached_input_per_million: 0.1,
                output_per_million: 8.0,
            },
            PricingOverride {
                model: "GPT-5.6-SOL-2026-06-18".to_string(),
                input_per_million: 2.0,
                cached_input_per_million: 0.2,
                output_per_million: 10.0,
            },
        ]);
        assert!(duplicate.is_err());
    }

    #[test]
    fn rejects_invalid_custom_prices() {
        let result = validate_overrides(vec![PricingOverride {
            model: "glm-5".to_string(),
            input_per_million: -1.0,
            cached_input_per_million: 0.1,
            output_per_million: f64::NAN,
        }]);
        assert!(result.is_err());
    }

    #[test]
    fn selects_custom_price_by_event_date() {
        let versions = vec![
            PricingVersion {
                effective_from: "2026-01-01".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                overrides: vec![PricingOverride {
                    model: "gpt-5.6-sol".to_string(),
                    input_per_million: 2.0,
                    cached_input_per_million: 0.2,
                    output_per_million: 12.0,
                }],
            },
            PricingVersion {
                effective_from: "2026-07-01".to_string(),
                created_at: "2026-07-01T00:00:00Z".to_string(),
                overrides: vec![PricingOverride {
                    model: "gpt-5.6-sol".to_string(),
                    input_per_million: 5.0,
                    cached_input_per_million: 0.5,
                    output_per_million: 30.0,
                }],
            },
        ];
        assert_eq!(
            pricing_override_for_versions(&versions, "gpt-5.6-sol", "2026-06-30")
                .map(|rate| rate.input_per_million),
            Some(2.0)
        );
        assert_eq!(
            pricing_override_for_versions(&versions, "gpt-5.6-sol", "2026-07-01")
                .map(|rate| rate.input_per_million),
            Some(5.0)
        );
        assert!(pricing_override_for_versions(&versions, "gpt-5.6-sol", "2025-12-31").is_none());
    }
}
