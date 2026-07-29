use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, FixedOffset, Local, NaiveDate, Offset};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pricing::{EstimatedCost, estimate_standard_api_cost_at};

const RECENT_FILE_WINDOW: Duration = Duration::from_secs(3 * 24 * 60 * 60);

#[derive(Debug, Clone, Default, PartialEq)]
struct TokenUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
}

impl TokenUsage {
    fn add(&mut self, other: &Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(other.cached_input_tokens);
        self.cache_write_input_tokens = self
            .cache_write_input_tokens
            .saturating_add(other.cache_write_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.reasoning_output_tokens = self
            .reasoning_output_tokens
            .saturating_add(other.reasoning_output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }

    fn subtract(&self, previous: Option<&Self>) -> Self {
        let difference = |current: u64, old: u64| {
            if current >= old {
                current - old
            } else {
                current
            }
        };
        let previous = previous.cloned().unwrap_or_default();
        Self {
            input_tokens: difference(self.input_tokens, previous.input_tokens),
            cached_input_tokens: difference(self.cached_input_tokens, previous.cached_input_tokens),
            cache_write_input_tokens: difference(
                self.cache_write_input_tokens,
                previous.cache_write_input_tokens,
            ),
            output_tokens: difference(self.output_tokens, previous.output_tokens),
            reasoning_output_tokens: difference(
                self.reasoning_output_tokens,
                previous.reasoning_output_tokens,
            ),
            total_tokens: difference(self.total_tokens, previous.total_tokens),
        }
    }
}

#[derive(Debug, Clone)]
struct UsageEvent {
    timestamp_ms: i64,
    timestamp: String,
    model: String,
    usage: TokenUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTodayUsage {
    pub date: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    pub uncached_input_tokens: u64,
    pub cache_hit_percent: f64,
    pub files_scanned: usize,
    pub token_events: usize,
    pub duplicate_events_skipped: usize,
    pub malformed_lines_skipped: usize,
    pub latest_event_at: Option<String>,
    #[serde(default)]
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub uncached_input_cost_usd: f64,
    #[serde(default)]
    pub cached_input_cost_usd: f64,
    #[serde(default)]
    pub output_cost_usd: f64,
    #[serde(default)]
    pub cache_savings_usd: f64,
    #[serde(default)]
    pub priced_tokens: u64,
    #[serde(default)]
    pub unpriced_tokens: u64,
    #[serde(default)]
    pub models: Vec<String>,
}

pub fn scan_today_usage() -> Result<LocalTodayUsage, String> {
    let homes = codex_homes()?;
    let files = discover_recent_logs(&homes)?;
    let today = Local::now().date_naive();
    let offset = Local::now().offset().fix();
    let mut seen = HashSet::new();
    let mut total = TokenUsage::default();
    let mut token_events = 0;
    let mut duplicate_events_skipped = 0;
    let mut malformed_lines_skipped = 0;
    let mut latest_event: Option<(i64, String)> = None;
    let mut estimated_cost = EstimatedCost::default();
    let mut priced_tokens = 0_u64;
    let mut unpriced_tokens = 0_u64;
    let mut models = BTreeSet::new();

    for path in &files {
        let file =
            File::open(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
        let parsed = parse_log(BufReader::new(file), today, offset);
        malformed_lines_skipped += parsed.malformed_lines;

        for event in parsed.events {
            let signature = format!(
                "{}:{}:{}:{}:{}:{}:{}:{}",
                event.timestamp_ms,
                event.model,
                event.usage.input_tokens,
                event.usage.cached_input_tokens,
                event.usage.cache_write_input_tokens,
                event.usage.output_tokens,
                event.usage.reasoning_output_tokens,
                event.usage.total_tokens
            );
            if !seen.insert(signature) {
                duplicate_events_skipped += 1;
                continue;
            }

            token_events += 1;
            models.insert(event.model.clone());
            if let Some(cost) = estimate_standard_api_cost_at(
                &event.model,
                event.usage.input_tokens,
                event.usage.cached_input_tokens,
                event.usage.output_tokens,
                event.timestamp_ms,
            ) {
                estimated_cost.uncached_input_usd += cost.uncached_input_usd;
                estimated_cost.cached_input_usd += cost.cached_input_usd;
                estimated_cost.output_usd += cost.output_usd;
                estimated_cost.total_usd += cost.total_usd;
                estimated_cost.cache_savings_usd += cost.cache_savings_usd;
                priced_tokens = priced_tokens.saturating_add(event.usage.total_tokens);
            } else {
                unpriced_tokens = unpriced_tokens.saturating_add(event.usage.total_tokens);
            }
            total.add(&event.usage);
            if latest_event
                .as_ref()
                .is_none_or(|(timestamp, _)| event.timestamp_ms > *timestamp)
            {
                latest_event = Some((event.timestamp_ms, event.timestamp));
            }
        }
    }

    let uncached_input_tokens = total.input_tokens.saturating_sub(total.cached_input_tokens);
    let cache_hit_percent = if total.input_tokens == 0 {
        0.0
    } else {
        ((total.cached_input_tokens as f64 / total.input_tokens as f64) * 10_000.0).round() / 100.0
    };

    Ok(LocalTodayUsage {
        date: today.to_string(),
        input_tokens: total.input_tokens,
        cached_input_tokens: total.cached_input_tokens,
        cache_write_input_tokens: total.cache_write_input_tokens,
        output_tokens: total.output_tokens,
        reasoning_output_tokens: total.reasoning_output_tokens,
        total_tokens: total.total_tokens,
        uncached_input_tokens,
        cache_hit_percent,
        files_scanned: files.len(),
        token_events,
        duplicate_events_skipped,
        malformed_lines_skipped,
        latest_event_at: latest_event.map(|(_, timestamp)| timestamp),
        estimated_cost_usd: estimated_cost.total_usd,
        uncached_input_cost_usd: estimated_cost.uncached_input_usd,
        cached_input_cost_usd: estimated_cost.cached_input_usd,
        output_cost_usd: estimated_cost.output_usd,
        cache_savings_usd: estimated_cost.cache_savings_usd,
        priced_tokens,
        unpriced_tokens,
        models: models.into_iter().collect(),
    })
}

fn codex_homes() -> Result<Vec<PathBuf>, String> {
    if let Ok(configured) = env::var("CODEX_HOME") {
        let homes = configured
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if !homes.is_empty() {
            return Ok(homes);
        }
    }

    let home = env::var_os("HOME").ok_or_else(|| "无法确定用户主目录".to_string())?;
    Ok(vec![PathBuf::from(home).join(".codex")])
}

fn discover_recent_logs(homes: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let cutoff = SystemTime::now()
        .checked_sub(RECENT_FILE_WINDOW)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut output = Vec::new();

    for home in homes {
        let active_dir = home.join("sessions");
        let archived_dir = home.join("archived_sessions");
        let mut selected = HashMap::<PathBuf, PathBuf>::new();

        collect_logs(&archived_dir, &archived_dir, cutoff, &mut selected)?;
        collect_logs(&active_dir, &active_dir, cutoff, &mut selected)?;
        output.extend(selected.into_values());
    }

    output.sort();
    Ok(output)
}

fn collect_logs(
    root: &Path,
    directory: &Path,
    cutoff: SystemTime,
    selected: &mut HashMap<PathBuf, PathBuf>,
) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("无法扫描 {}：{error}", directory.display()))?;

    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() {
            collect_logs(root, &path, cutoff, selected)?;
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        if metadata.modified().is_ok_and(|modified| modified < cutoff) {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        selected.insert(relative, path);
    }
    Ok(())
}

struct ParsedLog {
    events: Vec<UsageEvent>,
    malformed_lines: usize,
}

fn parse_log<R: BufRead>(reader: R, today: NaiveDate, offset: FixedOffset) -> ParsedLog {
    let mut events = Vec::new();
    let mut malformed_lines = 0;
    let mut previous_total: Option<TokenUsage> = None;
    let mut current_model = "unknown".to_string();

    for line in reader.lines() {
        let Ok(line) = line else {
            malformed_lines += 1;
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            malformed_lines += 1;
            continue;
        };
        let payload = entry.get("payload").unwrap_or(&Value::Null);

        if entry.get("type").and_then(Value::as_str) == Some("turn_context") {
            if let Some(model) = payload.get("model").and_then(Value::as_str) {
                current_model = model.to_string();
            }
            continue;
        }

        if entry.get("type").and_then(Value::as_str) != Some("event_msg")
            || payload.get("type").and_then(Value::as_str) != Some("token_count")
        {
            continue;
        }
        let Some(info) = payload.get("info").filter(|value| value.is_object()) else {
            continue;
        };

        let total = info.get("total_token_usage").and_then(parse_usage);
        if total.as_ref().is_some_and(|value| {
            previous_total
                .as_ref()
                .is_some_and(|previous| previous == value)
        }) {
            continue;
        }
        let usage = info
            .get("last_token_usage")
            .and_then(parse_usage)
            .or_else(|| {
                total
                    .as_ref()
                    .map(|value| value.subtract(previous_total.as_ref()))
            });
        if let Some(total) = total {
            previous_total = Some(total);
        }
        let Some(usage) = usage else {
            continue;
        };
        if usage.input_tokens == 0
            && usage.cached_input_tokens == 0
            && usage.output_tokens == 0
            && usage.reasoning_output_tokens == 0
        {
            continue;
        }

        let Some(timestamp) = entry.get("timestamp").and_then(Value::as_str) else {
            continue;
        };
        let Ok(parsed_timestamp) = DateTime::parse_from_rfc3339(timestamp) else {
            continue;
        };
        if parsed_timestamp.with_timezone(&offset).date_naive() != today {
            continue;
        }

        let model = info
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(&current_model)
            .to_string();
        events.push(UsageEvent {
            timestamp_ms: parsed_timestamp.timestamp_millis(),
            timestamp: timestamp.to_string(),
            model,
            usage,
        });
    }

    ParsedLog {
        events,
        malformed_lines,
    }
}

fn parse_usage(value: &Value) -> Option<TokenUsage> {
    if !value.is_object() {
        return None;
    }
    let read = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    let input_tokens = read("input_tokens");
    let output_tokens = read("output_tokens");
    let reported_total = read("total_tokens");

    Some(TokenUsage {
        input_tokens,
        cached_input_tokens: read("cached_input_tokens").min(input_tokens),
        cache_write_input_tokens: read("cache_write_input_tokens"),
        output_tokens,
        reasoning_output_tokens: read("reasoning_output_tokens"),
        total_tokens: if reported_total == 0 {
            input_tokens.saturating_add(output_tokens)
        } else {
            reported_total
        },
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn adds_last_usage_and_ignores_duplicate_cumulative_snapshot() {
        let log = concat!(
            "{\"timestamp\":\"2026-07-26T00:00:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":40,\"output_tokens\":10,\"reasoning_output_tokens\":2,\"total_tokens\":110},\"last_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":40,\"output_tokens\":10,\"reasoning_output_tokens\":2,\"total_tokens\":110}}}}\n",
            "{\"timestamp\":\"2026-07-26T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":150,\"cached_input_tokens\":70,\"output_tokens\":15,\"reasoning_output_tokens\":3,\"total_tokens\":165},\"last_token_usage\":{\"input_tokens\":50,\"cached_input_tokens\":30,\"output_tokens\":5,\"reasoning_output_tokens\":1,\"total_tokens\":55}}}}\n",
            "{\"timestamp\":\"2026-07-26T00:00:02Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":150,\"cached_input_tokens\":70,\"output_tokens\":15,\"reasoning_output_tokens\":3,\"total_tokens\":165},\"last_token_usage\":{\"input_tokens\":50,\"cached_input_tokens\":30,\"output_tokens\":5,\"reasoning_output_tokens\":1,\"total_tokens\":55}}}}\n"
        );
        let parsed = parse_log(
            Cursor::new(log),
            NaiveDate::from_ymd_opt(2026, 7, 26).expect("valid date"),
            FixedOffset::east_opt(8 * 60 * 60).expect("valid offset"),
        );
        assert_eq!(parsed.events.len(), 2);
        let mut total = TokenUsage::default();
        for event in parsed.events {
            total.add(&event.usage);
        }
        assert_eq!(total.input_tokens, 150);
        assert_eq!(total.cached_input_tokens, 70);
        assert_eq!(total.output_tokens, 15);
        assert_eq!(total.total_tokens, 165);
    }

    #[test]
    fn derives_delta_when_last_usage_is_missing() {
        let log = concat!(
            "{\"timestamp\":\"2026-07-26T00:00:00Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":80,\"cached_input_tokens\":32,\"output_tokens\":8,\"total_tokens\":88}}}}\n",
            "{\"timestamp\":\"2026-07-26T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":110,\"cached_input_tokens\":48,\"output_tokens\":12,\"total_tokens\":122}}}}\n"
        );
        let parsed = parse_log(
            Cursor::new(log),
            NaiveDate::from_ymd_opt(2026, 7, 26).expect("valid date"),
            FixedOffset::east_opt(8 * 60 * 60).expect("valid offset"),
        );
        assert_eq!(parsed.events[1].usage.input_tokens, 30);
        assert_eq!(parsed.events[1].usage.cached_input_tokens, 16);
        assert_eq!(parsed.events[1].usage.total_tokens, 34);
    }
}
