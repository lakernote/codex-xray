use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde::Serialize;
use serde_json::Value;

const MAX_CAPTURED_FRAMES: usize = 900;
const MAX_CAPTURED_FRAMES_PER_CHANNEL: usize = 300;
const MAX_CAPTURED_BODY_BYTES: usize = 128 * 1024;

static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_CORRELATION: AtomicU64 = AtomicU64::new(1);
static CAPTURED_FRAMES: OnceLock<Mutex<VecDeque<ProtocolFrame>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
pub struct ProtocolFrame {
    pub sequence: u64,
    pub captured_at: String,
    pub channel: String,
    pub direction: String,
    pub kind: String,
    pub method: Option<String>,
    pub correlation_id: Option<String>,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    pub bytes: u64,
    pub truncated: bool,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtocolCaptureSnapshot {
    pub generated_at: String,
    pub frames: Vec<ProtocolFrame>,
}

pub struct ProtocolRecord<'a> {
    pub channel: &'a str,
    pub direction: &'a str,
    pub kind: &'a str,
    pub method: Option<&'a str>,
    pub correlation_id: Option<&'a str>,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
}

pub fn new_correlation_id(prefix: &str) -> String {
    let id = NEXT_CORRELATION.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{id}")
}

pub fn record_json(metadata: ProtocolRecord<'_>, value: &Value) {
    let redacted = redact_sensitive(value, None);
    let body = serde_json::to_string_pretty(&redacted)
        .unwrap_or_else(|_| "<unable to serialize JSON>".to_string());
    record_text(metadata, &body);
}

pub fn record_text(metadata: ProtocolRecord<'_>, value: &str) {
    let bytes = value.len() as u64;
    let (body, truncated) = truncate_utf8(value, MAX_CAPTURED_BODY_BYTES);
    let frame = ProtocolFrame {
        sequence: NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        captured_at: Utc::now().to_rfc3339(),
        channel: metadata.channel.to_string(),
        direction: metadata.direction.to_string(),
        kind: metadata.kind.to_string(),
        method: metadata.method.map(str::to_string),
        correlation_id: metadata.correlation_id.map(str::to_string),
        status: metadata.status,
        duration_ms: metadata.duration_ms,
        bytes,
        truncated,
        body,
    };

    if let Ok(mut frames) = captured_frames().lock() {
        frames.push_back(frame);
        while frames
            .iter()
            .filter(|frame| frame.channel == metadata.channel)
            .count()
            > MAX_CAPTURED_FRAMES_PER_CHANNEL
        {
            if let Some(index) = frames
                .iter()
                .position(|frame| frame.channel == metadata.channel)
            {
                frames.remove(index);
            } else {
                break;
            }
        }
        while frames.len() > MAX_CAPTURED_FRAMES {
            frames.pop_front();
        }
    }
}

pub fn snapshot(channel: Option<&str>, after_sequence: Option<u64>) -> ProtocolCaptureSnapshot {
    let frames = captured_frames()
        .lock()
        .map(|frames| {
            frames
                .iter()
                .filter(|frame| channel.is_none_or(|channel| frame.channel == channel))
                .filter(|frame| after_sequence.is_none_or(|sequence| frame.sequence > sequence))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    ProtocolCaptureSnapshot {
        generated_at: Utc::now().to_rfc3339(),
        frames,
    }
}

fn captured_frames() -> &'static Mutex<VecDeque<ProtocolFrame>> {
    CAPTURED_FRAMES.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn truncate_utf8(value: &str, maximum: usize) -> (String, bool) {
    if value.len() <= maximum {
        return (value.to_string(), false);
    }
    let mut end = maximum;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (
        format!("{}\n… <{} bytes omitted>", &value[..end], value.len() - end),
        true,
    )
}

fn redact_sensitive(value: &Value, key: Option<&str>) -> Value {
    if key.is_some_and(is_sensitive_key) {
        return Value::String("<redacted>".to_string());
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), redact_sensitive(value, Some(key))))
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| redact_sensitive(value, None))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    matches!(
        normalized.as_str(),
        "authorization"
            | "apikey"
            | "password"
            | "secret"
            | "credential"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "clientsecret"
            | "privatekey"
    ) || normalized.ends_with("apikey")
        || normalized.ends_with("password")
        || normalized.ends_with("secret")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_credentials_without_hiding_token_usage() {
        let value = redact_sensitive(
            &json!({
                "api_key": "secret-value",
                "input_tokens": 42,
                "nested": {"Authorization": "Bearer secret"}
            }),
            None,
        );
        assert_eq!(value["api_key"], "<redacted>");
        assert_eq!(value["input_tokens"], 42);
        assert_eq!(value["nested"]["Authorization"], "<redacted>");
    }

    #[test]
    fn truncation_keeps_utf8_valid() {
        let (value, truncated) = truncate_utf8("中文协议记录", 5);
        assert!(truncated);
        assert!(value.starts_with('中'));
    }
}
