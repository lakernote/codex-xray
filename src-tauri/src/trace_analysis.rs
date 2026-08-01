use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use memchr::memmem;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codex::ThreadMetadata;
use crate::pricing::estimate_standard_api_cost_at;
use crate::storage::{
    TraceMirrorPhaseEvent, TraceMirrorSession, TraceMirrorToolEvent, TraceMirrorTurn,
    TraceMirrorUsageEvent, read_index_entries, write_index_entries, write_trace_mirror,
};

const TRACE_INDEX_SCHEMA_VERSION: u32 = 13;
const TRACE_INDEX_NAMESPACE: &str = "trace-files";
const MAX_TRACE_FILES: usize = 240;
const MAX_PARSED_LINE_BYTES: usize = 512 * 1024;
const LARGE_OUTPUT_BYTES: u64 = 128 * 1024;
const ACTIVE_FILE_WINDOW: Duration = Duration::from_secs(5 * 60);
const MAX_TRACE_DETAIL_CHARS: usize = 48_000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RawUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
}

impl RawUsage {
    fn subtract(&self, previous: Option<&Self>) -> Self {
        let previous = previous.cloned().unwrap_or_default();
        let difference = |current: u64, old: u64| {
            if current >= old {
                current - old
            } else {
                current
            }
        };
        Self {
            input_tokens: difference(self.input_tokens, previous.input_tokens),
            cached_input_tokens: difference(self.cached_input_tokens, previous.cached_input_tokens),
            output_tokens: difference(self.output_tokens, previous.output_tokens),
            reasoning_output_tokens: difference(
                self.reasoning_output_tokens,
                previous.reasoning_output_tokens,
            ),
            total_tokens: difference(self.total_tokens, previous.total_tokens),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct TraceUsageEvent {
    source_order: usize,
    timestamp_ms: i64,
    turn_id: String,
    model: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
    context_window: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TraceEventField {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct TraceToolEvent {
    source_order: usize,
    completed_source_order: Option<usize>,
    execution_completed_source_order: Option<usize>,
    timestamp_ms: i64,
    completed_at_ms: Option<i64>,
    execution_completed_at_ms: Option<i64>,
    turn_id: String,
    call_id: Option<String>,
    source_type: String,
    execution_end_source_type: Option<String>,
    result_source_type: Option<String>,
    name: String,
    category: String,
    server: Option<String>,
    subject: Option<String>,
    detail: Option<String>,
    arguments: Vec<TraceEventField>,
    arguments_json: Option<String>,
    result_fields: Vec<TraceEventField>,
    result_json: Option<String>,
    signature: String,
    repeated: bool,
    failed: bool,
    output_bytes: u64,
    exit_code: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct TracePhaseEvent {
    source_order: usize,
    source_end_order: Option<usize>,
    timestamp_ms: i64,
    turn_id: String,
    phase: String,
    source_type: String,
    role: Option<String>,
    content: Option<String>,
    content_parts: usize,
    content_bytes: u64,
    summary_parts: usize,
    encrypted_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct TraceCompactionEvent {
    source_order: usize,
    notification_source_order: Option<usize>,
    timestamp_ms: i64,
    window_number: Option<usize>,
    history_items: usize,
    encrypted_summary_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct CompactedLine {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    entry_type: String,
    payload: CompactedPayload,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct CompactedPayload {
    window_number: Option<usize>,
    replacement_history: Vec<CompactedHistoryItem>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct CompactedHistoryItem {
    #[serde(rename = "type")]
    kind: String,
    encrypted_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct TraceTurnRecord {
    id: String,
    model: String,
    reasoning_effort: Option<String>,
    summary_mode: Option<String>,
    started_source_order: Option<usize>,
    context_source_order: Option<usize>,
    started_at_ms: Option<i64>,
    completed_source_order: Option<usize>,
    completed_at_ms: Option<i64>,
    duration_ms: Option<u64>,
    tool_events: Vec<TraceToolEvent>,
    phase_events: Vec<TracePhaseEvent>,
    compaction_events: Vec<TraceCompactionEvent>,
    structured_failures: usize,
    developer_context_bytes: u64,
    world_state_bytes: u64,
    turn_context_bytes: u64,
    memory_context_bytes: u64,
    memory_citations: usize,
    unattributed_large_outputs: usize,
    unattributed_large_output_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RepeatedSubject {
    subject: String,
    count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct IndexedTraceFile {
    length: u64,
    modified_ms: u128,
    session_id: Option<String>,
    parent_id: Option<String>,
    forked_at_ms: Option<i64>,
    source: Option<String>,
    cwd: Option<String>,
    session_meta_source_order: Option<usize>,
    session_meta_at_ms: Option<i64>,
    conversation_name: Option<String>,
    official_status: Option<String>,
    started_at: Option<String>,
    updated_at: Option<String>,
    model: Option<String>,
    base_instructions_bytes: u64,
    dynamic_tool_definitions: usize,
    dynamic_tool_bytes: u64,
    turns: BTreeMap<String, TraceTurnRecord>,
    open_turn_ids: BTreeSet<String>,
    usage_events: Vec<TraceUsageEvent>,
    tool_calls: usize,
    failed_tool_calls: usize,
    repeated_tool_calls: usize,
    repeated_reads: usize,
    top_repeated_tool: Option<RepeatedSubject>,
    top_repeated_path: Option<RepeatedSubject>,
    context_compactions: usize,
    large_tool_outputs: usize,
    large_tool_output_bytes: u64,
    malformed_lines: usize,
    last_turn_open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceIndex {
    schema_version: u32,
    files: BTreeMap<String, IndexedTraceFile>,
}

impl Default for TraceIndex {
    fn default() -> Self {
        Self {
            schema_version: TRACE_INDEX_SCHEMA_VERSION,
            files: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceInsight {
    pub kind: String,
    pub severity: String,
    pub value: f64,
    pub subject: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSessionSummary {
    pub id: String,
    pub conversation_name: Option<String>,
    pub project: String,
    pub project_path: String,
    pub session_path: Option<String>,
    pub analysis_state: String,
    pub source: String,
    pub model: String,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    pub status: String,
    pub status_source: String,
    pub is_subagent: bool,
    pub parent_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub turns: usize,
    pub tool_calls: usize,
    pub failed_tool_calls: usize,
    pub repeated_tool_calls: usize,
    pub repeated_reads: usize,
    pub context_compactions: usize,
    pub large_tool_outputs: usize,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub uncached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    pub cache_hit_percent: f64,
    pub context_growth_tokens: u64,
    pub estimated_cost_usd: f64,
    pub high_cost_turns: usize,
    pub issue_score: u64,
    pub insights: Vec<TraceInsight>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceTimelineEvent {
    pub source_order: usize,
    pub source_end_order: Option<usize>,
    pub timestamp: Option<String>,
    pub completed_at: Option<String>,
    pub execution_completed_at: Option<String>,
    pub kind: String,
    pub category: String,
    pub label: String,
    pub status: String,
    pub sequence: Option<usize>,
    pub call_id: Option<String>,
    pub source_type: Option<String>,
    pub execution_end_source_type: Option<String>,
    pub result_source_type: Option<String>,
    pub server: Option<String>,
    pub subject: Option<String>,
    pub detail: Option<String>,
    pub arguments: Vec<TraceEventField>,
    pub arguments_json: Option<String>,
    pub result_fields: Vec<TraceEventField>,
    pub result_json: Option<String>,
    pub content: Option<String>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    pub context_window: Option<u64>,
    pub context_delta_tokens: Option<i64>,
    pub context_before_tokens: Option<u64>,
    pub context_after_tokens: Option<u64>,
    pub context_reclaimed_tokens: Option<u64>,
    pub cache_hit_percent: Option<f64>,
    pub estimated_cost_usd: Option<f64>,
    pub content_parts: usize,
    pub content_bytes: u64,
    pub summary_parts: usize,
    pub encrypted_bytes: u64,
    pub output_bytes: u64,
    pub exit_code: Option<i64>,
    pub repeated: bool,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceTurnSummary {
    pub id: String,
    pub sequence: usize,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub summary_mode: Option<String>,
    pub status: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub uncached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    pub cache_hit_percent: f64,
    pub peak_input_tokens: u64,
    pub first_input_tokens: u64,
    pub last_input_tokens: u64,
    pub model_passes: usize,
    pub context_window: Option<u64>,
    pub context_utilization_percent: Option<f64>,
    pub context_growth_tokens: u64,
    pub estimated_cost_usd: f64,
    pub tool_calls: usize,
    pub failed_tool_calls: usize,
    pub repeated_tool_calls: usize,
    pub repeated_reads: usize,
    pub context_compactions: usize,
    pub estimated_reclaimed_tokens: u64,
    pub local_context_bytes: u64,
    pub session_context_bytes: u64,
    pub developer_context_bytes: u64,
    pub world_state_bytes: u64,
    pub turn_context_bytes: u64,
    pub memory_context_bytes: u64,
    pub memory_citations: usize,
    pub large_tool_outputs: usize,
    pub large_tool_output_bytes: u64,
    pub issue_score: u64,
    pub insights: Vec<TraceInsight>,
    pub timeline: Vec<TraceTimelineEvent>,
    pub timeline_events_omitted: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceToolAggregate {
    pub name: String,
    pub calls: usize,
    pub failures: usize,
    pub repeats: usize,
    pub large_outputs: usize,
    pub output_bytes: u64,
    pub subjects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSessionDetail {
    pub session: TraceSessionSummary,
    pub flagged_turns: usize,
    pub flagged_tokens: u64,
    pub flagged_cost_usd: f64,
    pub model_passes: usize,
    pub estimated_reclaimed_tokens: u64,
    pub local_context_bytes: u64,
    pub memory_context_bytes: u64,
    pub memory_citations: usize,
    pub turns: Vec<TraceTurnSummary>,
    pub tools: Vec<TraceToolAggregate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TraceTotals {
    pub sessions: usize,
    pub running_sessions: usize,
    pub subagent_sessions: usize,
    pub turns: usize,
    pub tool_calls: usize,
    pub failed_tool_calls: usize,
    pub repeated_tool_calls: usize,
    pub repeated_reads: usize,
    pub context_compactions: usize,
    pub large_tool_outputs: usize,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub uncached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub subagent_tokens: u64,
    pub estimated_cost_usd: f64,
    pub subagent_cost_usd: f64,
    pub cache_hit_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSnapshot {
    pub generated_at: String,
    pub files_indexed: usize,
    pub files_scanned: usize,
    pub files_reused: usize,
    pub official_threads_matched: usize,
    pub elapsed_ms: u128,
    pub coverage_start: Option<String>,
    pub coverage_end: Option<String>,
    pub totals: TraceTotals,
    pub sessions: Vec<TraceSessionSummary>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionCategoryUsage {
    pub category: String,
    pub calls: usize,
    pub failures: usize,
    pub repeated_calls: usize,
    pub timed_calls: usize,
    pub duration_ms: u64,
    pub output_bytes: u64,
    pub unique_items: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionUsageItem {
    pub category: String,
    pub name: String,
    pub server: Option<String>,
    pub calls: usize,
    pub failures: usize,
    pub repeated_calls: usize,
    pub timed_calls: usize,
    pub duration_ms: u64,
    pub average_duration_ms: Option<u64>,
    pub output_bytes: u64,
    pub projects: usize,
    pub sessions: usize,
    pub turns: usize,
    pub last_used_at: Option<String>,
    pub occurrences: Vec<ExtensionUsageOccurrence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionUsageOccurrence {
    pub project: String,
    pub session_id: String,
    pub turn_id: String,
    pub call_id: Option<String>,
    pub used_at: Option<String>,
    pub failed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionUsageSnapshot {
    pub generated_at: String,
    pub analyzed_sessions: usize,
    pub current_sessions: usize,
    pub stale_sessions: usize,
    pub projects: usize,
    pub turns: usize,
    pub calls: usize,
    pub failures: usize,
    pub repeated_calls: usize,
    pub timed_calls: usize,
    pub duration_ms: u64,
    pub output_bytes: u64,
    pub categories: Vec<ExtensionCategoryUsage>,
    pub items: Vec<ExtensionUsageItem>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct TraceIndexCache {
    index: Option<TraceIndex>,
}

fn merge_thread_metadata(index: &mut TraceIndex, metadata: Option<&[ThreadMetadata]>) -> usize {
    for record in index.files.values_mut() {
        record.official_status = None;
    }
    let Some(metadata) = metadata else {
        return 0;
    };
    let by_id = metadata
        .iter()
        .map(|thread| (thread.id.as_str(), thread))
        .collect::<HashMap<_, _>>();
    let mut matched = 0;
    for record in index.files.values_mut() {
        let Some(thread) = record
            .session_id
            .as_deref()
            .and_then(|session_id| by_id.get(session_id))
        else {
            continue;
        };
        matched += 1;
        record.conversation_name = thread.name.clone();
        record.cwd = Some(thread.cwd.clone());
        record.official_status = thread.status.clone();
    }
    matched
}

#[derive(Default)]
struct SessionUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
    cost_usd: f64,
    context_growth_tokens: u64,
    high_cost_turns: usize,
}

#[derive(Default)]
struct TurnUsage {
    first_input_tokens: Option<u64>,
    last_input_tokens: Option<u64>,
    max_input_tokens: u64,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
    cost_usd: f64,
    context_window: Option<u64>,
}

pub fn build_trace_snapshot_cached(
    index_path: &Path,
    cache: &mut TraceIndexCache,
    thread_metadata: Option<&[ThreadMetadata]>,
) -> Result<TraceSnapshot, String> {
    build_trace_snapshot_internal(index_path, Some(cache), thread_metadata)
}

pub fn build_trace_catalog(
    index_path: &Path,
    thread_metadata: &[ThreadMetadata],
) -> Result<TraceSnapshot, String> {
    let started = Instant::now();
    let mut warnings = Vec::new();
    let index = load_index(index_path, &mut warnings);
    let records_by_session = index
        .files
        .iter()
        .filter_map(|(path, record)| {
            record
                .session_id
                .as_deref()
                .map(|session_id| (session_id, (path.as_str(), record)))
        })
        .collect::<HashMap<_, _>>();
    let replay_records = index
        .files
        .values()
        .filter_map(|record| {
            record
                .session_id
                .as_deref()
                .map(|session_id| (session_id, record))
        })
        .collect::<HashMap<_, _>>();
    let now = SystemTime::now();
    let mut sessions = thread_metadata
        .iter()
        .map(|thread| {
            let mut summary =
                if let Some((path, record)) = records_by_session.get(thread.id.as_str()).copied() {
                    let replayed_prefix = replayed_prefix_len(record, &replay_records);
                    let mut summary = summarize_session(record, replayed_prefix, now);
                    summary.session_path = Some(path.to_string());
                    summary.analysis_state = if indexed_record_is_current(path, record) {
                        "ready"
                    } else {
                        "stale"
                    }
                    .to_string();
                    summary
                } else {
                    empty_trace_summary(thread)
                };
            apply_thread_metadata_to_summary(&mut summary, thread);
            summary
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.project.cmp(&right.project))
    });

    let analyzed = sessions
        .iter()
        .filter(|session| session.analysis_state != "not_analyzed")
        .cloned()
        .collect::<Vec<_>>();
    let totals = calculate_trace_totals(&analyzed);
    let coverage_start = analyzed
        .iter()
        .filter_map(|session| session.started_at.clone())
        .min();
    let coverage_end = analyzed
        .iter()
        .filter_map(|session| session.updated_at.clone())
        .max();
    Ok(TraceSnapshot {
        generated_at: Utc::now().to_rfc3339(),
        files_indexed: index.files.len(),
        files_scanned: 0,
        files_reused: analyzed
            .iter()
            .filter(|session| session.analysis_state == "ready")
            .count(),
        official_threads_matched: thread_metadata.len(),
        elapsed_ms: started.elapsed().as_millis(),
        coverage_start,
        coverage_end,
        totals,
        sessions,
        warnings,
    })
}

#[derive(Default)]
struct ExtensionUsageAccumulator {
    calls: usize,
    failures: usize,
    repeated_calls: usize,
    timed_calls: usize,
    duration_ms: u64,
    output_bytes: u64,
    projects: HashSet<String>,
    sessions: HashSet<String>,
    turns: HashSet<String>,
    last_used_ms: Option<i64>,
    occurrences: Vec<ExtensionUsageOccurrenceAccumulator>,
}

#[derive(Debug, Clone)]
struct ExtensionUsageOccurrenceAccumulator {
    project: String,
    session_id: String,
    turn_id: String,
    call_id: Option<String>,
    timestamp_ms: i64,
    failed: bool,
}

pub fn build_extension_usage_cached(
    index_path: &Path,
    cache: &mut TraceIndexCache,
) -> Result<ExtensionUsageSnapshot, String> {
    if cache.index.is_none() {
        cache.index = load_detail_index(index_path)?;
    }
    let Some(index) = cache.index.as_ref() else {
        return Ok(empty_extension_usage_snapshot());
    };

    // A session can temporarily appear at both its active and archived path.
    // Prefer the current record, then the newest persisted parse.
    let mut selected = HashMap::<String, (&str, &IndexedTraceFile, bool)>::new();
    for (path, record) in &index.files {
        let Some(session_id) = record.session_id.as_ref() else {
            continue;
        };
        let current = indexed_record_is_current(path, record);
        let replace = selected
            .get(session_id)
            .is_none_or(|(_, existing, existing_current)| {
                (current && !existing_current)
                    || (current == *existing_current && record.modified_ms > existing.modified_ms)
            });
        if replace {
            selected.insert(session_id.clone(), (path.as_str(), record, current));
        }
    }

    let mut projects = HashSet::new();
    let mut current_sessions = 0;
    let mut stale_sessions = 0;
    let mut turns = 0;
    let mut categories = HashMap::<String, ExtensionUsageAccumulator>::new();
    let mut items = HashMap::<(String, String, Option<String>), ExtensionUsageAccumulator>::new();

    for (session_id, (_, record, current)) in &selected {
        if *current {
            current_sessions += 1;
        } else {
            stale_sessions += 1;
        }
        let project = project_identity(record.cwd.as_deref()).1;
        projects.insert(project.clone());
        turns += record.turns.len();

        for turn in record.turns.values() {
            for event in record_tool_events_after_fork(record, turn) {
                let category = normalized_tool_category(&event.category);
                let name = extension_usage_name(event, &category);
                let duration = tool_event_duration(event);
                let turn_key = format!("{session_id}:{}", event.turn_id);

                let category_usage = categories.entry(category.clone()).or_default();
                accumulate_extension_event(
                    category_usage,
                    event,
                    duration,
                    &project,
                    session_id,
                    &turn_key,
                );

                let item_usage = items
                    .entry((category, name, event.server.clone()))
                    .or_default();
                accumulate_extension_event(
                    item_usage, event, duration, &project, session_id, &turn_key,
                );
            }
        }
    }

    let mut category_rows = categories
        .into_iter()
        .map(|(category, usage)| ExtensionCategoryUsage {
            category,
            calls: usage.calls,
            failures: usage.failures,
            repeated_calls: usage.repeated_calls,
            timed_calls: usage.timed_calls,
            duration_ms: usage.duration_ms,
            output_bytes: usage.output_bytes,
            unique_items: 0,
        })
        .collect::<Vec<_>>();
    for row in &mut category_rows {
        row.unique_items = items
            .keys()
            .filter(|(category, _, _)| category == &row.category)
            .count();
    }
    category_rows.sort_by(|left, right| {
        extension_category_rank(&left.category)
            .cmp(&extension_category_rank(&right.category))
            .then_with(|| right.calls.cmp(&left.calls))
    });

    let mut item_rows = items
        .into_iter()
        .map(|((category, name, server), mut usage)| {
            usage
                .occurrences
                .sort_by_key(|occurrence| std::cmp::Reverse(occurrence.timestamp_ms));
            let occurrences = usage
                .occurrences
                .into_iter()
                .take(24)
                .map(|occurrence| ExtensionUsageOccurrence {
                    project: occurrence.project,
                    session_id: occurrence.session_id,
                    turn_id: occurrence.turn_id,
                    call_id: occurrence.call_id,
                    used_at: millis_to_rfc3339(occurrence.timestamp_ms),
                    failed: occurrence.failed,
                })
                .collect();
            ExtensionUsageItem {
                category,
                name,
                server,
                calls: usage.calls,
                failures: usage.failures,
                repeated_calls: usage.repeated_calls,
                timed_calls: usage.timed_calls,
                duration_ms: usage.duration_ms,
                average_duration_ms: (usage.timed_calls > 0)
                    .then(|| usage.duration_ms / usage.timed_calls as u64),
                output_bytes: usage.output_bytes,
                projects: usage.projects.len(),
                sessions: usage.sessions.len(),
                turns: usage.turns.len(),
                last_used_at: usage.last_used_ms.and_then(millis_to_rfc3339),
                occurrences,
            }
        })
        .collect::<Vec<_>>();
    item_rows.sort_by(|left, right| {
        right
            .calls
            .cmp(&left.calls)
            .then_with(|| right.failures.cmp(&left.failures))
            .then_with(|| right.duration_ms.cmp(&left.duration_ms))
            .then_with(|| left.name.cmp(&right.name))
    });

    let calls = category_rows.iter().map(|row| row.calls).sum();
    let failures = category_rows.iter().map(|row| row.failures).sum();
    let repeated_calls = category_rows.iter().map(|row| row.repeated_calls).sum();
    let timed_calls = category_rows.iter().map(|row| row.timed_calls).sum();
    let duration_ms = category_rows.iter().map(|row| row.duration_ms).sum();
    let output_bytes = category_rows.iter().map(|row| row.output_bytes).sum();
    let mut warnings = Vec::new();
    if stale_sessions > 0 {
        warnings.push(format!(
            "{stale_sessions} 个已持久化分析在 Session 变化后尚未重新解剖，统计可能少于最新实际值"
        ));
    }
    if selected.is_empty() {
        warnings.push("尚无已解剖 Session；请先在执行解剖页选择一个对话并点击解剖".to_string());
    }

    Ok(ExtensionUsageSnapshot {
        generated_at: Utc::now().to_rfc3339(),
        analyzed_sessions: selected.len(),
        current_sessions,
        stale_sessions,
        projects: projects.len(),
        turns,
        calls,
        failures,
        repeated_calls,
        timed_calls,
        duration_ms,
        output_bytes,
        categories: category_rows,
        items: item_rows,
        warnings,
    })
}

fn empty_extension_usage_snapshot() -> ExtensionUsageSnapshot {
    ExtensionUsageSnapshot {
        generated_at: Utc::now().to_rfc3339(),
        analyzed_sessions: 0,
        current_sessions: 0,
        stale_sessions: 0,
        projects: 0,
        turns: 0,
        calls: 0,
        failures: 0,
        repeated_calls: 0,
        timed_calls: 0,
        duration_ms: 0,
        output_bytes: 0,
        categories: Vec::new(),
        items: Vec::new(),
        warnings: vec!["尚无已解剖 Session；请先在执行解剖页选择一个对话并点击解剖".to_string()],
    }
}

fn record_tool_events_after_fork<'a>(
    session: &'a IndexedTraceFile,
    turn: &'a TraceTurnRecord,
) -> impl Iterator<Item = &'a TraceToolEvent> {
    turn.tool_events
        .iter()
        .filter(|event| event_is_after_fork(session, event.timestamp_ms))
}

fn normalized_tool_category(category: &str) -> String {
    if category.is_empty() {
        "tool".to_string()
    } else {
        category.to_string()
    }
}

fn extension_usage_name(event: &TraceToolEvent, category: &str) -> String {
    match category {
        "skill" => event
            .detail
            .clone()
            .or_else(|| event.subject.clone())
            .unwrap_or_else(|| event.name.clone()),
        _ => event.name.clone(),
    }
}

fn tool_event_duration(event: &TraceToolEvent) -> Option<u64> {
    event.completed_at_ms.and_then(|completed| {
        completed
            .checked_sub(event.timestamp_ms)
            .map(|duration| duration.max(0) as u64)
    })
}

fn accumulate_extension_event(
    usage: &mut ExtensionUsageAccumulator,
    event: &TraceToolEvent,
    duration: Option<u64>,
    project: &str,
    session_id: &str,
    turn_key: &str,
) {
    usage.calls += 1;
    usage.failures += usize::from(event.failed);
    usage.repeated_calls += usize::from(event.repeated);
    usage.output_bytes = usage.output_bytes.saturating_add(event.output_bytes);
    if let Some(duration) = duration {
        usage.timed_calls += 1;
        usage.duration_ms = usage.duration_ms.saturating_add(duration);
    }
    usage.projects.insert(project.to_string());
    usage.sessions.insert(session_id.to_string());
    usage.turns.insert(turn_key.to_string());
    usage.occurrences.push(ExtensionUsageOccurrenceAccumulator {
        project: project.to_string(),
        session_id: session_id.to_string(),
        turn_id: event.turn_id.clone(),
        call_id: event.call_id.clone(),
        timestamp_ms: event.timestamp_ms,
        failed: event.failed,
    });
    usage.last_used_ms = Some(
        usage
            .last_used_ms
            .map_or(event.timestamp_ms, |value| value.max(event.timestamp_ms)),
    );
}

fn extension_category_rank(category: &str) -> usize {
    match category {
        "mcp" => 0,
        "skill" => 1,
        "cli" => 2,
        "browser" => 3,
        "automation" => 4,
        "agent" => 5,
        "file" => 6,
        "tool" => 7,
        _ => 8,
    }
}

fn calculate_trace_totals(sessions: &[TraceSessionSummary]) -> TraceTotals {
    let mut totals = TraceTotals {
        sessions: sessions.len(),
        ..TraceTotals::default()
    };
    for session in sessions {
        totals.running_sessions += usize::from(
            session.status == "running"
                || session.status == "waiting_approval"
                || session.status == "waiting_input",
        );
        totals.subagent_sessions += usize::from(session.is_subagent);
        totals.turns += session.turns;
        totals.tool_calls += session.tool_calls;
        totals.failed_tool_calls += session.failed_tool_calls;
        totals.repeated_tool_calls += session.repeated_tool_calls;
        totals.repeated_reads += session.repeated_reads;
        totals.context_compactions += session.context_compactions;
        totals.large_tool_outputs += session.large_tool_outputs;
        totals.input_tokens = totals.input_tokens.saturating_add(session.input_tokens);
        totals.cached_input_tokens = totals
            .cached_input_tokens
            .saturating_add(session.cached_input_tokens);
        totals.uncached_input_tokens = totals
            .uncached_input_tokens
            .saturating_add(session.uncached_input_tokens);
        totals.output_tokens = totals.output_tokens.saturating_add(session.output_tokens);
        totals.total_tokens = totals.total_tokens.saturating_add(session.total_tokens);
        totals.estimated_cost_usd += session.estimated_cost_usd;
        if session.is_subagent {
            totals.subagent_tokens = totals.subagent_tokens.saturating_add(session.total_tokens);
            totals.subagent_cost_usd += session.estimated_cost_usd;
        }
    }
    totals.cache_hit_percent = if totals.input_tokens == 0 {
        0.0
    } else {
        ((totals.cached_input_tokens as f64 / totals.input_tokens as f64) * 10_000.0).round()
            / 100.0
    };
    totals
}

fn indexed_record_is_current(path: &str, record: &IndexedTraceFile) -> bool {
    fs::metadata(path).is_ok_and(|metadata| {
        metadata.len() == record.length && modified_millis(&metadata) == record.modified_ms
    })
}

fn apply_thread_metadata_to_summary(summary: &mut TraceSessionSummary, thread: &ThreadMetadata) {
    summary.id = thread.id.clone();
    summary.conversation_name = thread.name.clone();
    (summary.project, summary.project_path) = project_identity(Some(&thread.cwd));
    summary.session_path = thread.path.clone().or_else(|| summary.session_path.clone());
    summary.started_at =
        timestamp_value_to_rfc3339(thread.created_at).or_else(|| summary.started_at.clone());
    summary.updated_at =
        timestamp_value_to_rfc3339(thread.updated_at).or_else(|| summary.updated_at.clone());
    if let Some(status) = &thread.status {
        summary.status = status.clone();
        summary.status_source = "app_server".to_string();
    }
    summary.parent_id = thread
        .parent_thread_id
        .clone()
        .or_else(|| summary.parent_id.clone());
    summary.is_subagent = summary.parent_id.is_some();
}

fn empty_trace_summary(thread: &ThreadMetadata) -> TraceSessionSummary {
    let (project, project_path) = project_identity(Some(&thread.cwd));
    TraceSessionSummary {
        id: thread.id.clone(),
        conversation_name: thread.name.clone(),
        project,
        project_path,
        session_path: thread.path.clone(),
        analysis_state: "not_analyzed".to_string(),
        source: "codex".to_string(),
        model: "—".to_string(),
        started_at: timestamp_value_to_rfc3339(thread.created_at),
        updated_at: timestamp_value_to_rfc3339(thread.updated_at),
        status: thread
            .status
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        status_source: if thread.status.is_some() {
            "app_server".to_string()
        } else {
            "unknown".to_string()
        },
        is_subagent: thread.parent_thread_id.is_some(),
        parent_id: thread.parent_thread_id.clone(),
        duration_ms: None,
        turns: 0,
        tool_calls: 0,
        failed_tool_calls: 0,
        repeated_tool_calls: 0,
        repeated_reads: 0,
        context_compactions: 0,
        large_tool_outputs: 0,
        input_tokens: 0,
        cached_input_tokens: 0,
        uncached_input_tokens: 0,
        output_tokens: 0,
        reasoning_output_tokens: 0,
        total_tokens: 0,
        cache_hit_percent: 0.0,
        context_growth_tokens: 0,
        estimated_cost_usd: 0.0,
        high_cost_turns: 0,
        issue_score: 0,
        insights: Vec::new(),
    }
}

fn timestamp_value_to_rfc3339(value: i64) -> Option<String> {
    if value <= 0 {
        return None;
    }
    if value >= 10_000_000_000 {
        DateTime::<Utc>::from_timestamp_millis(value).map(|timestamp| timestamp.to_rfc3339())
    } else {
        DateTime::<Utc>::from_timestamp(value, 0).map(|timestamp| timestamp.to_rfc3339())
    }
}

fn build_trace_snapshot_internal(
    index_path: &Path,
    cache: Option<&mut TraceIndexCache>,
    thread_metadata: Option<&[ThreadMetadata]>,
) -> Result<TraceSnapshot, String> {
    let started = Instant::now();
    let mut warnings = Vec::new();
    let previous_index = load_index(index_path, &mut warnings);
    let files = discover_trace_files()?;
    let mut next_index = TraceIndex::default();
    let mut files_scanned = 0;
    let mut files_reused = 0;

    for path in &files {
        let metadata =
            fs::metadata(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
        let key = path.to_string_lossy().to_string();
        let length = metadata.len();
        let modified_ms = modified_millis(&metadata);
        let indexed = match previous_index.files.get(&key) {
            Some(previous) if previous.length == length && previous.modified_ms == modified_ms => {
                files_reused += 1;
                previous.clone()
            }
            _ => {
                files_scanned += 1;
                parse_trace_file(path, length, modified_ms)?
            }
        };
        next_index.files.insert(key, indexed);
    }

    let official_threads_matched = merge_thread_metadata(&mut next_index, thread_metadata);
    let records_by_session = next_index
        .files
        .values()
        .filter_map(|record| {
            record
                .session_id
                .as_deref()
                .map(|session_id| (session_id, record))
        })
        .collect::<HashMap<_, _>>();
    let now = SystemTime::now();
    let mut sessions = Vec::new();
    let mut malformed_lines = 0;

    for record in next_index.files.values() {
        malformed_lines += record.malformed_lines;
        let replayed_prefix = replayed_prefix_len(record, &records_by_session);
        sessions.push(summarize_session(record, replayed_prefix, now));
    }

    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.issue_score.cmp(&left.issue_score))
    });
    sessions.truncate(80);

    let totals = calculate_trace_totals(&sessions);

    if malformed_lines > 0 {
        warnings.push(format!("{malformed_lines} 行本地 session 无法解析，已跳过"));
    }
    if files.len() == MAX_TRACE_FILES {
        warnings.push(format!(
            "为保持低开销，执行解剖只索引最近修改的 {MAX_TRACE_FILES} 个 session 文件"
        ));
    }
    if let Err(error) = write_index(index_path, &next_index) {
        warnings.push(format!("执行解剖增量索引未保存：{error}"));
    }

    let coverage_start = sessions
        .iter()
        .filter_map(|session| session.started_at.clone())
        .min();
    let coverage_end = sessions
        .iter()
        .filter_map(|session| session.updated_at.clone())
        .max();
    let files_indexed = next_index.files.len();
    if let Some(cache) = cache {
        cache.index = Some(next_index);
    }

    Ok(TraceSnapshot {
        generated_at: Utc::now().to_rfc3339(),
        files_indexed,
        files_scanned,
        files_reused,
        official_threads_matched,
        elapsed_ms: started.elapsed().as_millis(),
        coverage_start,
        coverage_end,
        totals,
        sessions,
        warnings,
    })
}

pub fn get_trace_session_detail_cached(
    index_path: &Path,
    session_id: &str,
    cache: &mut TraceIndexCache,
) -> Result<Option<TraceSessionDetail>, String> {
    if cache.index.is_none() {
        cache.index = load_detail_index(index_path)?;
    }
    let Some(index) = cache.index.as_ref() else {
        return Ok(None);
    };
    let records_by_session = index
        .files
        .values()
        .filter_map(|record| {
            record
                .session_id
                .as_deref()
                .map(|session_id| (session_id, record))
        })
        .collect::<HashMap<_, _>>();
    let Some(record) = records_by_session.get(session_id).copied() else {
        return Ok(None);
    };
    let replayed_prefix = replayed_prefix_len(record, &records_by_session);
    Ok(Some(summarize_session_detail(
        record,
        replayed_prefix,
        SystemTime::now(),
    )))
}

pub fn analyze_trace_session_cached(
    index_path: &Path,
    session_id: &str,
    preferred_path: Option<&str>,
    cache: &mut TraceIndexCache,
) -> Result<Option<TraceSessionDetail>, String> {
    if session_id.len() < 8
        || !session_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err("Session ID 不合法".to_string());
    }
    let preferred = preferred_path
        .map(PathBuf::from)
        .filter(|path| trace_path_matches_session(path, session_id));
    let Some(path) = preferred.or(find_trace_file_by_session_id(session_id)?) else {
        return Err(format!(
            "没有找到 {session_id} 对应的本地 session 文件；它可能尚未同步到本机"
        ));
    };
    let metadata =
        fs::metadata(&path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    let key = path.to_string_lossy().to_string();
    let record = parse_trace_file(&path, metadata.len(), modified_millis(&metadata))?;
    if record.session_id.as_deref() != Some(session_id) {
        return Err("找到的 session 文件与所选对话不一致".to_string());
    }

    if cache.index.is_none() {
        cache.index = Some(load_detail_index(index_path)?.unwrap_or_default());
    }
    let index = cache.index.as_mut().expect("trace index is initialized");
    index.files.insert(key, record);
    write_index(index_path, index)?;
    get_trace_session_detail_cached(index_path, session_id, cache)
}

fn trace_path_matches_session(path: &Path, session_id: &str) -> bool {
    path.is_file()
        && path.extension().and_then(|value| value.to_str()) == Some("jsonl")
        && path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.contains(session_id))
}

fn find_trace_file_by_session_id(session_id: &str) -> Result<Option<PathBuf>, String> {
    for home in codex_homes()? {
        for root in [home.join("sessions"), home.join("archived_sessions")] {
            if let Some(path) = find_trace_file_in_directory(&root, session_id)? {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

fn find_trace_file_in_directory(
    directory: &Path,
    session_id: &str,
) -> Result<Option<PathBuf>, String> {
    if !directory.exists() {
        return Ok(None);
    }
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("无法扫描 {}：{error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() {
            if let Some(found) = find_trace_file_in_directory(&path, session_id)? {
                return Ok(Some(found));
            }
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl")
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.contains(session_id))
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn load_detail_index(index_path: &Path) -> Result<Option<TraceIndex>, String> {
    let entries = read_index_entries(
        index_path,
        TRACE_INDEX_NAMESPACE,
        TRACE_INDEX_SCHEMA_VERSION,
    )?;
    if entries.is_empty() {
        return Ok(None);
    }
    let files = entries
        .into_iter()
        .map(|(session_path, payload)| {
            serde_json::from_slice::<IndexedTraceFile>(&payload)
                .map(|indexed| (session_path, indexed))
                .map_err(|error| format!("过程索引记录无法解析：{error}"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let index = TraceIndex {
        schema_version: TRACE_INDEX_SCHEMA_VERSION,
        files,
    };
    write_trace_mirror(index_path, &trace_mirror_sessions(&index))?;
    Ok(Some(index))
}

fn summarize_session_detail(
    record: &IndexedTraceFile,
    replayed_prefix: usize,
    now: SystemTime,
) -> TraceSessionDetail {
    let mut usage_by_turn = HashMap::<String, Vec<&TraceUsageEvent>>::new();
    for event in record.usage_events.iter().skip(replayed_prefix) {
        usage_by_turn
            .entry(event.turn_id.clone())
            .or_default()
            .push(event);
    }

    let mut turn_ids = record.turns.keys().cloned().collect::<BTreeSet<_>>();
    turn_ids.extend(usage_by_turn.keys().cloned());
    let mut turns = turn_ids
        .into_iter()
        .map(|turn_id| {
            let record_for_turn = record.turns.get(&turn_id);
            let usage_events = usage_by_turn.remove(&turn_id).unwrap_or_default();
            summarize_turn(record, &turn_id, record_for_turn, &usage_events, now)
        })
        .filter(|turn| turn.total_tokens > 0 || turn.tool_calls > 0 || turn.context_compactions > 0)
        .collect::<Vec<_>>();
    turns.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    for (index, turn) in turns.iter_mut().enumerate() {
        turn.sequence = index + 1;
    }

    let mut tool_map =
        BTreeMap::<String, (usize, usize, usize, usize, u64, BTreeSet<String>)>::new();
    for turn in record.turns.values() {
        for event in turn
            .tool_events
            .iter()
            .filter(|event| event_is_after_fork(record, event.timestamp_ms))
        {
            let aggregate = tool_map.entry(event.name.clone()).or_default();
            aggregate.0 += 1;
            aggregate.1 += usize::from(event.failed);
            aggregate.2 += usize::from(event.repeated);
            if event.output_bytes >= LARGE_OUTPUT_BYTES {
                aggregate.3 += 1;
            }
            aggregate.4 = aggregate.4.saturating_add(event.output_bytes);
            if let Some(subject) = &event.subject {
                aggregate.5.insert(subject.clone());
            }
        }
    }
    let mut tools = tool_map
        .into_iter()
        .map(
            |(name, (calls, failures, repeats, large_outputs, output_bytes, subjects))| {
                TraceToolAggregate {
                    name,
                    calls,
                    failures,
                    repeats,
                    large_outputs,
                    output_bytes,
                    subjects: subjects.into_iter().take(5).collect(),
                }
            },
        )
        .collect::<Vec<_>>();
    tools.sort_by(|left, right| {
        right
            .failures
            .cmp(&left.failures)
            .then_with(|| right.repeats.cmp(&left.repeats))
            .then_with(|| right.calls.cmp(&left.calls))
    });

    let flagged = turns
        .iter()
        .filter(|turn| {
            turn.insights
                .iter()
                .any(|insight| insight.severity != "info")
        })
        .collect::<Vec<_>>();
    let flagged_turns = flagged.len();
    let flagged_tokens = flagged
        .iter()
        .fold(0_u64, |total, turn| total.saturating_add(turn.total_tokens));
    let flagged_cost_usd = flagged.iter().map(|turn| turn.estimated_cost_usd).sum();
    let mut session = summarize_session(record, replayed_prefix, now);
    session.turns = turns.len();
    session.tool_calls = turns.iter().map(|turn| turn.tool_calls).sum();
    session.failed_tool_calls = turns.iter().map(|turn| turn.failed_tool_calls).sum();
    session.repeated_tool_calls = turns.iter().map(|turn| turn.repeated_tool_calls).sum();
    session.repeated_reads = turns.iter().map(|turn| turn.repeated_reads).sum();
    session.context_compactions = turns.iter().map(|turn| turn.context_compactions).sum();
    session.large_tool_outputs = turns.iter().map(|turn| turn.large_tool_outputs).sum();
    let model_passes = turns.iter().map(|turn| turn.model_passes).sum();
    let estimated_reclaimed_tokens = turns
        .iter()
        .map(|turn| turn.estimated_reclaimed_tokens)
        .fold(0_u64, u64::saturating_add);
    let local_context_bytes = turns
        .iter()
        .map(|turn| turn.local_context_bytes)
        .fold(0_u64, u64::saturating_add);
    let memory_context_bytes = turns
        .iter()
        .map(|turn| turn.memory_context_bytes)
        .fold(0_u64, u64::saturating_add);
    let memory_citations = turns.iter().map(|turn| turn.memory_citations).sum();

    TraceSessionDetail {
        session,
        flagged_turns,
        flagged_tokens,
        flagged_cost_usd,
        model_passes,
        estimated_reclaimed_tokens,
        local_context_bytes,
        memory_context_bytes,
        memory_citations,
        turns,
        tools,
    }
}

fn summarize_turn(
    session: &IndexedTraceFile,
    turn_id: &str,
    record: Option<&TraceTurnRecord>,
    events: &[&TraceUsageEvent],
    now: SystemTime,
) -> TraceTurnSummary {
    let mut usage = TurnUsage::default();
    let mut timeline = Vec::new();
    let mut previous_input_tokens = None;
    for (event_index, event) in events.iter().enumerate() {
        usage.first_input_tokens.get_or_insert(event.input_tokens);
        usage.last_input_tokens = Some(event.input_tokens);
        usage.max_input_tokens = usage.max_input_tokens.max(event.input_tokens);
        usage.input_tokens = usage.input_tokens.saturating_add(event.input_tokens);
        usage.cached_input_tokens = usage
            .cached_input_tokens
            .saturating_add(event.cached_input_tokens.min(event.input_tokens));
        usage.output_tokens = usage.output_tokens.saturating_add(event.output_tokens);
        usage.reasoning_output_tokens = usage
            .reasoning_output_tokens
            .saturating_add(event.reasoning_output_tokens);
        usage.total_tokens = usage.total_tokens.saturating_add(event.total_tokens);
        usage.context_window = usage.context_window.or(event.context_window);
        let event_cost = estimate_standard_api_cost_at(
            &event.model,
            event.input_tokens,
            event.cached_input_tokens,
            event.output_tokens,
            event.timestamp_ms,
        );
        if let Some(cost) = &event_cost {
            usage.cost_usd += cost.total_usd;
        }
        let context_delta_tokens =
            previous_input_tokens.map(|previous| signed_token_delta(event.input_tokens, previous));
        previous_input_tokens = Some(event.input_tokens);
        timeline.push(TraceTimelineEvent {
            source_order: event.source_order,
            source_end_order: None,
            timestamp: millis_to_rfc3339(event.timestamp_ms),
            completed_at: None,
            execution_completed_at: None,
            kind: "tokens".to_string(),
            category: "usage".to_string(),
            label: event.model.clone(),
            status: "info".to_string(),
            sequence: Some(event_index + 1),
            call_id: None,
            source_type: Some("event_msg.token_count".to_string()),
            execution_end_source_type: None,
            result_source_type: None,
            server: None,
            subject: None,
            detail: None,
            arguments: Vec::new(),
            arguments_json: None,
            result_fields: Vec::new(),
            result_json: None,
            content: None,
            input_tokens: event.input_tokens,
            cached_input_tokens: event.cached_input_tokens,
            output_tokens: event.output_tokens,
            reasoning_output_tokens: event.reasoning_output_tokens,
            total_tokens: event.total_tokens,
            context_window: event.context_window,
            context_delta_tokens,
            context_before_tokens: None,
            context_after_tokens: None,
            context_reclaimed_tokens: None,
            cache_hit_percent: Some(percentage(event.cached_input_tokens, event.input_tokens)),
            estimated_cost_usd: event_cost.map(|cost| cost.total_usd),
            content_parts: 0,
            content_bytes: 0,
            summary_parts: 0,
            encrypted_bytes: 0,
            output_bytes: 0,
            exit_code: None,
            repeated: false,
            duration_ms: None,
        });
    }

    let mut path_counts = HashMap::<String, usize>::new();
    let mut repeated_tool_calls = 0;
    let mut failed_tool_events = 0;
    let mut large_tool_outputs = 0;
    let mut large_tool_output_bytes = 0_u64;
    let mut tool_calls = 0;
    if let Some(record) = record {
        if let Some(started_at_ms) = record.started_at_ms {
            let first_turn_source_order = session
                .turns
                .values()
                .filter_map(|turn| turn.started_source_order)
                .min();
            let starts_session = record.started_source_order == first_turn_source_order;
            timeline.push(TraceTimelineEvent {
                source_order: if starts_session {
                    session
                        .session_meta_source_order
                        .or(record.started_source_order)
                        .unwrap_or_default()
                } else {
                    record.started_source_order.unwrap_or_default()
                },
                source_end_order: record.context_source_order,
                timestamp: millis_to_rfc3339(if starts_session {
                    session.session_meta_at_ms.unwrap_or(started_at_ms)
                } else {
                    started_at_ms
                }),
                completed_at: None,
                execution_completed_at: None,
                kind: "started".to_string(),
                category: "lifecycle".to_string(),
                label: "turn".to_string(),
                status: "info".to_string(),
                sequence: None,
                call_id: None,
                source_type: Some(
                    if starts_session {
                        "session_meta + event_msg.task_started + turn_context"
                    } else {
                        "event_msg.task_started + turn_context"
                    }
                    .to_string(),
                ),
                execution_end_source_type: None,
                result_source_type: None,
                server: None,
                subject: None,
                detail: None,
                arguments: Vec::new(),
                arguments_json: None,
                result_fields: Vec::new(),
                result_json: None,
                content: None,
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                reasoning_output_tokens: 0,
                total_tokens: 0,
                context_window: None,
                context_delta_tokens: None,
                context_before_tokens: None,
                context_after_tokens: None,
                context_reclaimed_tokens: None,
                cache_hit_percent: None,
                estimated_cost_usd: None,
                content_parts: 0,
                content_bytes: 0,
                summary_parts: 0,
                encrypted_bytes: 0,
                output_bytes: 0,
                exit_code: None,
                repeated: false,
                duration_ms: None,
            });
        }
        for phase in record
            .phase_events
            .iter()
            .filter(|event| event_is_after_fork(session, event.timestamp_ms))
        {
            timeline.push(TraceTimelineEvent {
                source_order: phase.source_order,
                source_end_order: phase.source_end_order,
                timestamp: millis_to_rfc3339(phase.timestamp_ms),
                completed_at: None,
                execution_completed_at: None,
                kind: "phase".to_string(),
                category: if phase.phase == "user_prompt" {
                    "input".to_string()
                } else {
                    "model".to_string()
                },
                label: phase.phase.clone(),
                status: "info".to_string(),
                sequence: None,
                call_id: None,
                source_type: Some(phase.source_type.clone()),
                execution_end_source_type: None,
                result_source_type: None,
                server: None,
                subject: phase.role.clone(),
                detail: None,
                arguments: Vec::new(),
                arguments_json: None,
                result_fields: Vec::new(),
                result_json: None,
                content: phase.content.clone(),
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                reasoning_output_tokens: 0,
                total_tokens: 0,
                context_window: None,
                context_delta_tokens: None,
                context_before_tokens: None,
                context_after_tokens: None,
                context_reclaimed_tokens: None,
                cache_hit_percent: None,
                estimated_cost_usd: None,
                content_parts: phase.content_parts,
                content_bytes: phase.content_bytes,
                summary_parts: phase.summary_parts,
                encrypted_bytes: phase.encrypted_bytes,
                output_bytes: 0,
                exit_code: None,
                repeated: false,
                duration_ms: None,
            });
        }
        for event in record
            .tool_events
            .iter()
            .filter(|event| event_is_after_fork(session, event.timestamp_ms))
        {
            tool_calls += 1;
            repeated_tool_calls += usize::from(event.repeated);
            failed_tool_events += usize::from(event.failed);
            if event.output_bytes >= LARGE_OUTPUT_BYTES {
                large_tool_outputs += 1;
                large_tool_output_bytes =
                    large_tool_output_bytes.saturating_add(event.output_bytes);
            }
            if let Some(subject) = &event.subject {
                *path_counts.entry(subject.clone()).or_default() += 1;
            }
            let request_event = TraceTimelineEvent {
                source_order: event.source_order,
                source_end_order: None,
                timestamp: millis_to_rfc3339(event.timestamp_ms),
                completed_at: None,
                execution_completed_at: None,
                kind: "tool_request".to_string(),
                category: if event.category.is_empty() {
                    "tool".to_string()
                } else {
                    event.category.clone()
                },
                label: event.name.clone(),
                status: if event.completed_at_ms.is_none() {
                    "pending"
                } else {
                    "info"
                }
                .to_string(),
                sequence: None,
                call_id: event.call_id.clone(),
                source_type: Some(format!("response_item.{}", event.source_type)),
                execution_end_source_type: None,
                result_source_type: None,
                server: event.server.clone(),
                subject: event.subject.clone(),
                detail: event.detail.clone(),
                arguments: event.arguments.clone(),
                arguments_json: event.arguments_json.clone(),
                result_fields: Vec::new(),
                result_json: None,
                content: None,
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                reasoning_output_tokens: 0,
                total_tokens: 0,
                context_window: None,
                context_delta_tokens: None,
                context_before_tokens: None,
                context_after_tokens: None,
                context_reclaimed_tokens: None,
                cache_hit_percent: None,
                estimated_cost_usd: None,
                content_parts: 0,
                content_bytes: 0,
                summary_parts: 0,
                encrypted_bytes: 0,
                output_bytes: 0,
                exit_code: None,
                repeated: event.repeated,
                duration_ms: None,
            };
            timeline.push(request_event.clone());

            if let (Some(source_order), Some(completed_at_ms), Some(source_type)) = (
                event.execution_completed_source_order,
                event.execution_completed_at_ms,
                event.execution_end_source_type.as_ref(),
            ) {
                let mut execution_event = request_event.clone();
                execution_event.source_order = source_order;
                execution_event.timestamp = millis_to_rfc3339(completed_at_ms);
                execution_event.kind = "tool_execution".to_string();
                execution_event.status = "success".to_string();
                execution_event.source_type = Some(source_type.clone());
                execution_event.arguments_json = None;
                execution_event.repeated = false;
                execution_event.duration_ms = completed_at_ms
                    .checked_sub(event.timestamp_ms)
                    .map(|duration| duration.max(0) as u64);
                timeline.push(execution_event);
            }

            if let (Some(source_order), Some(completed_at_ms), Some(source_type)) = (
                event.completed_source_order,
                event.completed_at_ms,
                event.result_source_type.as_ref(),
            ) {
                let mut result_event = request_event;
                result_event.source_order = source_order;
                result_event.timestamp = millis_to_rfc3339(completed_at_ms);
                result_event.kind = "tool_result".to_string();
                result_event.status = if event.failed { "failed" } else { "success" }.to_string();
                result_event.source_type = Some(source_type.clone());
                result_event.arguments_json = None;
                result_event.result_fields = event.result_fields.clone();
                result_event.result_json = event.result_json.clone();
                result_event.output_bytes = event.output_bytes;
                result_event.exit_code = event.exit_code;
                result_event.repeated = false;
                result_event.duration_ms = completed_at_ms
                    .checked_sub(
                        event
                            .execution_completed_at_ms
                            .unwrap_or(event.timestamp_ms),
                    )
                    .map(|duration| duration.max(0) as u64);
                timeline.push(result_event);
            }
        }
        for compaction in &record.compaction_events {
            let before_tokens = events
                .iter()
                .rev()
                .find(|event| event.source_order < compaction.source_order)
                .map(|event| event.input_tokens);
            let after_boundary = compaction
                .notification_source_order
                .unwrap_or(compaction.source_order);
            let after_tokens = events
                .iter()
                .find(|event| event.source_order > after_boundary)
                .map(|event| event.input_tokens);
            let reclaimed_tokens = before_tokens
                .zip(after_tokens)
                .map(|(before, after)| before.saturating_sub(after))
                .filter(|value| *value > 0);
            timeline.push(TraceTimelineEvent {
                source_order: compaction.source_order,
                source_end_order: compaction.notification_source_order,
                timestamp: millis_to_rfc3339(compaction.timestamp_ms),
                completed_at: None,
                execution_completed_at: None,
                kind: "compaction".to_string(),
                category: "context".to_string(),
                label: "context".to_string(),
                status: "warning".to_string(),
                sequence: compaction.window_number,
                call_id: None,
                source_type: Some(
                    if compaction.notification_source_order.is_some() {
                        "compacted + event_msg.context_compacted"
                    } else {
                        "compacted"
                    }
                    .to_string(),
                ),
                execution_end_source_type: None,
                result_source_type: None,
                server: None,
                subject: None,
                detail: None,
                arguments: Vec::new(),
                arguments_json: None,
                result_fields: Vec::new(),
                result_json: None,
                content: None,
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                reasoning_output_tokens: 0,
                total_tokens: 0,
                context_window: None,
                context_delta_tokens: before_tokens
                    .zip(after_tokens)
                    .map(|(before, after)| signed_token_delta(after, before)),
                context_before_tokens: before_tokens,
                context_after_tokens: after_tokens,
                context_reclaimed_tokens: reclaimed_tokens,
                cache_hit_percent: None,
                estimated_cost_usd: None,
                content_parts: compaction.history_items,
                content_bytes: 0,
                summary_parts: 0,
                encrypted_bytes: compaction.encrypted_summary_bytes,
                output_bytes: 0,
                exit_code: None,
                repeated: false,
                duration_ms: None,
            });
        }
        large_tool_outputs += record.unattributed_large_outputs;
        large_tool_output_bytes =
            large_tool_output_bytes.saturating_add(record.unattributed_large_output_bytes);
        if let Some(completed_at_ms) = record.completed_at_ms {
            timeline.push(TraceTimelineEvent {
                source_order: record.completed_source_order.unwrap_or_default(),
                source_end_order: None,
                timestamp: millis_to_rfc3339(completed_at_ms),
                completed_at: None,
                execution_completed_at: None,
                kind: "completed".to_string(),
                category: "lifecycle".to_string(),
                label: "turn".to_string(),
                status: "success".to_string(),
                sequence: None,
                call_id: None,
                source_type: Some("event_msg.task_complete".to_string()),
                execution_end_source_type: None,
                result_source_type: None,
                server: None,
                subject: None,
                detail: None,
                arguments: Vec::new(),
                arguments_json: None,
                result_fields: Vec::new(),
                result_json: None,
                content: None,
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                reasoning_output_tokens: 0,
                total_tokens: 0,
                context_window: None,
                context_delta_tokens: None,
                context_before_tokens: None,
                context_after_tokens: None,
                context_reclaimed_tokens: None,
                cache_hit_percent: None,
                estimated_cost_usd: None,
                content_parts: 0,
                content_bytes: 0,
                summary_parts: 0,
                encrypted_bytes: 0,
                output_bytes: 0,
                exit_code: None,
                repeated: false,
                duration_ms: record.duration_ms,
            });
        }
    }
    timeline.sort_by(|left, right| {
        left.source_order
            .cmp(&right.source_order)
            .then_with(|| left.timestamp.cmp(&right.timestamp))
    });
    let timeline_events_omitted = 0;

    let repeated_reads = path_counts
        .values()
        .map(|count| count.saturating_sub(1))
        .sum();
    let top_repeated_path = path_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 1)
        .map(|(path, _)| path);
    let failed_tool_calls = record
        .map(|record| failed_tool_events.max(record.structured_failures))
        .unwrap_or(failed_tool_events);
    let uncached_input_tokens = usage.input_tokens.saturating_sub(usage.cached_input_tokens);
    let cache_hit_percent = percentage(usage.cached_input_tokens, usage.input_tokens);
    let context_growth_tokens = usage
        .max_input_tokens
        .saturating_sub(usage.first_input_tokens.unwrap_or(0));
    let context_utilization_percent = usage
        .context_window
        .filter(|window| *window > 0)
        .map(|window| ((usage.max_input_tokens as f64 / window as f64) * 10_000.0).round() / 100.0);
    let mut insights = Vec::new();
    if context_growth_tokens >= 100_000 {
        insights.push(insight(
            "context_growth",
            severity_for_ratio(context_growth_tokens, 500_000),
            context_growth_tokens as f64,
            None,
        ));
    }
    if usage.input_tokens >= 100_000 && cache_hit_percent < 75.0 {
        insights.push(insight(
            "low_cache_hit",
            if cache_hit_percent < 40.0 {
                "high"
            } else {
                "medium"
            },
            cache_hit_percent,
            None,
        ));
    }
    if repeated_reads > 0 {
        insights.push(insight(
            "repeated_file_read",
            severity_for_count(repeated_reads, 8),
            repeated_reads as f64,
            top_repeated_path,
        ));
    }
    if failed_tool_calls > 0 {
        insights.push(insight(
            "tool_failure",
            severity_for_count(failed_tool_calls, 5),
            failed_tool_calls as f64,
            None,
        ));
    }
    if repeated_tool_calls > 0 {
        insights.push(insight(
            "repeated_tool_call",
            severity_for_count(repeated_tool_calls, 8),
            repeated_tool_calls as f64,
            None,
        ));
    }
    if large_tool_outputs > 0 {
        insights.push(insight(
            "large_tool_output",
            severity_for_ratio(large_tool_output_bytes, 2 * 1024 * 1024),
            large_tool_output_bytes as f64,
            None,
        ));
    }
    let context_compactions = record.map_or(0, |record| record.compaction_events.len());
    if context_compactions > 0 {
        insights.push(insight(
            "context_compaction",
            severity_for_count(context_compactions, 3),
            context_compactions as f64,
            None,
        ));
    }
    if usage.cost_usd >= 1.0 || usage.total_tokens >= 500_000 {
        insights.push(insight(
            "high_cost_turn",
            if usage.cost_usd >= 3.0 || usage.total_tokens >= 2_000_000 {
                "high"
            } else {
                "medium"
            },
            1.0,
            None,
        ));
    }
    insights.sort_by_key(|item| match item.severity.as_str() {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 3,
    });
    let issue_score = insights
        .iter()
        .map(|item| match item.severity.as_str() {
            "high" => 5,
            "medium" => 3,
            "low" => 1,
            _ => 0,
        })
        .sum();
    let modified = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_millis(
            session.modified_ms.min(u64::MAX as u128) as u64,
        ))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let recently_modified = now
        .duration_since(modified)
        .is_ok_and(|elapsed| elapsed <= ACTIVE_FILE_WINDOW);
    let status = if session.open_turn_ids.contains(turn_id) && recently_modified {
        "running"
    } else if session.open_turn_ids.contains(turn_id) {
        "interrupted"
    } else {
        "completed"
    }
    .to_string();
    let started_at = record
        .and_then(|record| record.started_at_ms)
        .and_then(millis_to_rfc3339);
    let completed_at = record
        .and_then(|record| record.completed_at_ms)
        .and_then(millis_to_rfc3339);
    let duration_ms = record.and_then(|record| record.duration_ms).or_else(|| {
        started_at
            .as_deref()
            .and_then(timestamp_millis)
            .zip(completed_at.as_deref().and_then(timestamp_millis))
            .map(|(start, end)| end.saturating_sub(start).max(0) as u64)
    });
    let starts_session = record.is_some_and(|turn| {
        turn.started_source_order
            == session
                .turns
                .values()
                .filter_map(|candidate| candidate.started_source_order)
                .min()
    });
    let developer_context_bytes = record.map_or(0, |turn| turn.developer_context_bytes);
    let world_state_bytes = record.map_or(0, |turn| turn.world_state_bytes);
    let turn_context_bytes = record.map_or(0, |turn| turn.turn_context_bytes);
    let session_context_bytes = if starts_session {
        session
            .base_instructions_bytes
            .saturating_add(session.dynamic_tool_bytes)
    } else {
        0
    };
    let local_context_bytes = developer_context_bytes
        .saturating_add(world_state_bytes)
        .saturating_add(turn_context_bytes)
        .saturating_add(session_context_bytes);
    let estimated_reclaimed_tokens = timeline
        .iter()
        .filter_map(|event| event.context_reclaimed_tokens)
        .fold(0_u64, u64::saturating_add);

    TraceTurnSummary {
        id: turn_id.to_string(),
        sequence: 0,
        model: record
            .map(|record| record.model.clone())
            .filter(|model| !model.is_empty())
            .or_else(|| events.first().map(|event| event.model.clone()))
            .unwrap_or_else(|| "unknown".to_string()),
        reasoning_effort: record.and_then(|record| record.reasoning_effort.clone()),
        summary_mode: record.and_then(|record| record.summary_mode.clone()),
        status,
        started_at,
        completed_at,
        duration_ms,
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        uncached_input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_output_tokens: usage.reasoning_output_tokens,
        total_tokens: usage.total_tokens,
        cache_hit_percent,
        peak_input_tokens: usage.max_input_tokens,
        first_input_tokens: usage.first_input_tokens.unwrap_or(0),
        last_input_tokens: usage.last_input_tokens.unwrap_or(0),
        model_passes: events.len(),
        context_window: usage.context_window,
        context_utilization_percent,
        context_growth_tokens,
        estimated_cost_usd: usage.cost_usd,
        tool_calls,
        failed_tool_calls,
        repeated_tool_calls,
        repeated_reads,
        context_compactions,
        estimated_reclaimed_tokens,
        local_context_bytes,
        session_context_bytes,
        developer_context_bytes,
        world_state_bytes,
        turn_context_bytes,
        memory_context_bytes: record.map_or(0, |turn| turn.memory_context_bytes),
        memory_citations: record.map_or(0, |turn| turn.memory_citations),
        large_tool_outputs,
        large_tool_output_bytes,
        issue_score,
        insights,
        timeline,
        timeline_events_omitted,
    }
}

fn percentage(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        ((part as f64 / total as f64) * 10_000.0).round() / 100.0
    }
}

fn signed_token_delta(current: u64, previous: u64) -> i64 {
    let delta = current as i128 - previous as i128;
    delta.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn event_is_after_fork(session: &IndexedTraceFile, timestamp_ms: i64) -> bool {
    session.parent_id.is_none()
        || session
            .forked_at_ms
            .is_none_or(|forked_at| timestamp_ms > forked_at)
}

fn millis_to_rfc3339(value: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(value).map(|timestamp| timestamp.to_rfc3339())
}

fn summarize_session(
    record: &IndexedTraceFile,
    replayed_prefix: usize,
    now: SystemTime,
) -> TraceSessionSummary {
    let mut usage = SessionUsage::default();
    let mut by_turn = HashMap::<String, TurnUsage>::new();

    for event in record.usage_events.iter().skip(replayed_prefix) {
        usage.input_tokens = usage.input_tokens.saturating_add(event.input_tokens);
        usage.cached_input_tokens = usage
            .cached_input_tokens
            .saturating_add(event.cached_input_tokens.min(event.input_tokens));
        usage.output_tokens = usage.output_tokens.saturating_add(event.output_tokens);
        usage.reasoning_output_tokens = usage
            .reasoning_output_tokens
            .saturating_add(event.reasoning_output_tokens);
        usage.total_tokens = usage.total_tokens.saturating_add(event.total_tokens);
        let turn = by_turn.entry(event.turn_id.clone()).or_default();
        turn.first_input_tokens.get_or_insert(event.input_tokens);
        turn.max_input_tokens = turn.max_input_tokens.max(event.input_tokens);
        turn.input_tokens = turn.input_tokens.saturating_add(event.input_tokens);
        turn.cached_input_tokens = turn
            .cached_input_tokens
            .saturating_add(event.cached_input_tokens);
        turn.output_tokens = turn.output_tokens.saturating_add(event.output_tokens);
        turn.reasoning_output_tokens = turn
            .reasoning_output_tokens
            .saturating_add(event.reasoning_output_tokens);
        turn.total_tokens = turn.total_tokens.saturating_add(event.total_tokens);
        turn.context_window = turn.context_window.or(event.context_window);
        if let Some(cost) = estimate_standard_api_cost_at(
            &event.model,
            event.input_tokens,
            event.cached_input_tokens,
            event.output_tokens,
            event.timestamp_ms,
        ) {
            usage.cost_usd += cost.total_usd;
            turn.cost_usd += cost.total_usd;
        }
    }
    for turn in by_turn.values() {
        usage.context_growth_tokens = usage.context_growth_tokens.saturating_add(
            turn.max_input_tokens
                .saturating_sub(turn.first_input_tokens.unwrap_or(0)),
        );
        if turn.cost_usd >= 1.0 || turn.total_tokens >= 500_000 {
            usage.high_cost_turns += 1;
        }
    }

    let uncached_input_tokens = usage.input_tokens.saturating_sub(usage.cached_input_tokens);
    let cache_hit_percent = if usage.input_tokens == 0 {
        0.0
    } else {
        ((usage.cached_input_tokens as f64 / usage.input_tokens as f64) * 10_000.0).round() / 100.0
    };
    let modified = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_millis(
            record.modified_ms.min(u64::MAX as u128) as u64,
        ))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let recently_modified = now
        .duration_since(modified)
        .is_ok_and(|elapsed| elapsed <= ACTIVE_FILE_WINDOW);
    let status = if record.last_turn_open && recently_modified {
        "running"
    } else if record.last_turn_open && record.failed_tool_calls > 0 {
        "failed"
    } else if record.last_turn_open {
        "interrupted"
    } else {
        "completed"
    }
    .to_string();

    let mut insights = Vec::new();
    if usage.context_growth_tokens >= 100_000 {
        insights.push(insight(
            "context_growth",
            severity_for_ratio(usage.context_growth_tokens, 500_000),
            usage.context_growth_tokens as f64,
            None,
        ));
    }
    if usage.input_tokens >= 100_000 && cache_hit_percent < 75.0 {
        insights.push(insight(
            "low_cache_hit",
            if cache_hit_percent < 40.0 {
                "high"
            } else {
                "medium"
            },
            cache_hit_percent,
            None,
        ));
    }
    if record.repeated_reads > 0 {
        insights.push(insight(
            "repeated_file_read",
            severity_for_count(record.repeated_reads, 8),
            record.repeated_reads as f64,
            record
                .top_repeated_path
                .as_ref()
                .map(|item| item.subject.clone()),
        ));
    }
    if record.failed_tool_calls > 0 {
        insights.push(insight(
            "tool_failure",
            severity_for_count(record.failed_tool_calls, 5),
            record.failed_tool_calls as f64,
            None,
        ));
    }
    if record.repeated_tool_calls > 0 {
        insights.push(insight(
            "repeated_tool_call",
            severity_for_count(record.repeated_tool_calls, 8),
            record.repeated_tool_calls as f64,
            record
                .top_repeated_tool
                .as_ref()
                .map(|item| item.subject.clone()),
        ));
    }
    if record.large_tool_outputs > 0 {
        insights.push(insight(
            "large_tool_output",
            severity_for_ratio(record.large_tool_output_bytes, 2 * 1024 * 1024),
            record.large_tool_output_bytes as f64,
            None,
        ));
    }
    if record.context_compactions > 0 {
        insights.push(insight(
            "context_compaction",
            severity_for_count(record.context_compactions, 3),
            record.context_compactions as f64,
            None,
        ));
    }
    if usage.high_cost_turns > 0 {
        insights.push(insight(
            "high_cost_turn",
            severity_for_count(usage.high_cost_turns, 4),
            usage.high_cost_turns as f64,
            None,
        ));
    }
    if record.parent_id.is_some() && usage.total_tokens > 0 {
        insights.push(insight("subagent_spend", "info", usage.cost_usd, None));
    }

    insights.sort_by_key(|item| match item.severity.as_str() {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 3,
    });
    let issue_score = insights
        .iter()
        .map(|item| match item.severity.as_str() {
            "high" => 5,
            "medium" => 3,
            "low" => 1,
            _ => 0,
        })
        .sum();
    let duration_ms = record
        .turns
        .values()
        .filter_map(|turn| turn.duration_ms)
        .max()
        .or_else(|| {
            record
                .started_at
                .as_deref()
                .and_then(timestamp_millis)
                .zip(record.updated_at.as_deref().and_then(timestamp_millis))
                .map(|(start, end)| end.saturating_sub(start).max(0) as u64)
        });

    TraceSessionSummary {
        id: record
            .session_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        conversation_name: record.conversation_name.clone(),
        project: project_identity(record.cwd.as_deref()).0,
        project_path: project_identity(record.cwd.as_deref()).1,
        session_path: None,
        analysis_state: "ready".to_string(),
        source: record
            .source
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        model: record
            .model
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        started_at: record.started_at.clone(),
        updated_at: record.updated_at.clone(),
        status: record.official_status.clone().unwrap_or(status),
        status_source: if record.official_status.is_some() {
            "app_server".to_string()
        } else {
            "local_events".to_string()
        },
        is_subagent: record.parent_id.is_some(),
        parent_id: record.parent_id.clone(),
        duration_ms,
        turns: record.turns.len(),
        tool_calls: record.tool_calls,
        failed_tool_calls: record.failed_tool_calls,
        repeated_tool_calls: record.repeated_tool_calls,
        repeated_reads: record.repeated_reads,
        context_compactions: record.context_compactions,
        large_tool_outputs: record.large_tool_outputs,
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        uncached_input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_output_tokens: usage.reasoning_output_tokens,
        total_tokens: usage.total_tokens,
        cache_hit_percent,
        context_growth_tokens: usage.context_growth_tokens,
        estimated_cost_usd: usage.cost_usd,
        high_cost_turns: usage.high_cost_turns,
        issue_score,
        insights,
    }
}

fn insight(kind: &str, severity: &str, value: f64, subject: Option<String>) -> TraceInsight {
    TraceInsight {
        kind: kind.to_string(),
        severity: severity.to_string(),
        value,
        subject,
    }
}

fn severity_for_count(value: usize, high_at: usize) -> &'static str {
    if value >= high_at {
        "high"
    } else if value >= 3 {
        "medium"
    } else {
        "low"
    }
}

fn severity_for_ratio(value: u64, high_at: u64) -> &'static str {
    if value >= high_at { "high" } else { "medium" }
}

fn merge_mirrored_phase_events(events: &mut Vec<TracePhaseEvent>) {
    events.sort_by_key(|event| event.source_order);
    let mut merged = Vec::<TracePhaseEvent>::with_capacity(events.len());

    for event in events.drain(..) {
        let Some(previous) = merged.last_mut() else {
            merged.push(event);
            continue;
        };
        let same_content = match (&previous.content, &event.content) {
            (Some(left), Some(right)) => left.trim() == right.trim(),
            (None, None) => true,
            _ => false,
        };
        let is_mirror_pair = previous.phase == event.phase
            && same_content
            && previous.source_order.abs_diff(event.source_order) <= 1
            && previous.timestamp_ms.abs_diff(event.timestamp_ms) <= 50
            && ((previous.source_type.starts_with("event_msg.")
                && event.source_type.starts_with("response_item."))
                || (previous.source_type.starts_with("response_item.")
                    && event.source_type.starts_with("event_msg.")));

        if !is_mirror_pair {
            merged.push(event);
            continue;
        }

        previous.source_end_order = Some(
            event
                .source_end_order
                .unwrap_or(event.source_order)
                .max(previous.source_end_order.unwrap_or(previous.source_order)),
        );
        previous.source_order = previous.source_order.min(event.source_order);
        previous.timestamp_ms = previous.timestamp_ms.min(event.timestamp_ms);
        previous.source_type = format!("{} + {}", previous.source_type, event.source_type);
        previous.role = previous.role.clone().or(event.role);
        previous.content_parts = previous.content_parts.max(event.content_parts);
        previous.content_bytes = previous.content_bytes.max(event.content_bytes);
        previous.summary_parts = previous.summary_parts.max(event.summary_parts);
        previous.encrypted_bytes = previous.encrypted_bytes.max(event.encrypted_bytes);
    }

    *events = merged;
}

fn parse_trace_file(
    path: &Path,
    length: u64,
    modified_ms: u128,
) -> Result<IndexedTraceFile, String> {
    let file = File::open(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut line = Vec::new();
    let mut session_id = None;
    let mut parent_id = None;
    let mut forked_at_ms = None;
    let mut source = None;
    let mut cwd = None;
    let mut session_meta_source_order = None;
    let mut session_meta_at_ms = None;
    let mut started_at = None;
    let mut updated_at = None;
    let mut model = None;
    let mut base_instructions_bytes = 0_u64;
    let mut dynamic_tool_definitions = 0_usize;
    let mut dynamic_tool_bytes = 0_u64;
    let mut current_model = "unknown".to_string();
    let mut previous_total: Option<RawUsage> = None;
    let mut current_turn = "unassigned".to_string();
    let mut turns = BTreeMap::<String, TraceTurnRecord>::new();
    let mut usage_events = Vec::new();
    let mut tool_calls = 0;
    let mut failed_tool_calls = 0;
    let mut context_compactions = 0;
    let mut large_tool_outputs = 0;
    let mut large_tool_output_bytes = 0_u64;
    let mut malformed_lines = 0;
    let mut call_counts = HashMap::<String, usize>::new();
    let mut tool_counts = HashMap::<String, usize>::new();
    let mut read_counts = HashMap::<String, usize>::new();
    let mut pending_calls = HashMap::<String, (String, usize)>::new();
    let mut open_turns = HashSet::<String>::new();
    let mut turns_with_context = HashSet::<String>::new();
    let mut source_order = 0_usize;

    loop {
        line.clear();
        let bytes_read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
        if bytes_read == 0 {
            break;
        }
        source_order = source_order.saturating_add(1);
        let relevant = [
            b"session_meta".as_slice(),
            b"turn_context".as_slice(),
            b"\"type\":\"world_state\"".as_slice(),
            b"\"type\":\"compacted\"".as_slice(),
            b"token_count".as_slice(),
            b"task_started".as_slice(),
            b"task_complete".as_slice(),
            b"user_message".as_slice(),
            b"agent_message".as_slice(),
            b"function_call".as_slice(),
            b"custom_tool_call".as_slice(),
            b"tool_search_call".as_slice(),
            b"tool_search_output".as_slice(),
            b"\"role\":\"user\"".as_slice(),
            b"\"role\":\"developer\"".as_slice(),
            b"\"type\":\"reasoning\"".as_slice(),
            b"\"phase\":".as_slice(),
            b"context_compacted".as_slice(),
            b"patch_apply_end".as_slice(),
            b"mcp_tool_call_end".as_slice(),
            b"web_search_end".as_slice(),
        ]
        .iter()
        .any(|needle| memmem::find(&line, needle).is_some());
        if !relevant {
            continue;
        }

        if memmem::find(&line, b"\"type\":\"compacted\"").is_some()
            && let Ok(compacted) = serde_json::from_slice::<CompactedLine>(&line)
            && compacted.entry_type == "compacted"
        {
            let Some(timestamp_ms) = compacted.timestamp.as_deref().and_then(timestamp_millis)
            else {
                malformed_lines += 1;
                continue;
            };
            if let Some(timestamp) = &compacted.timestamp {
                if started_at
                    .as_ref()
                    .is_none_or(|current| timestamp < current)
                {
                    started_at = Some(timestamp.clone());
                }
                if updated_at
                    .as_ref()
                    .is_none_or(|current| timestamp > current)
                {
                    updated_at = Some(timestamp.clone());
                }
            }
            let history_items = compacted.payload.replacement_history.len();
            let encrypted_summary_bytes = compacted
                .payload
                .replacement_history
                .iter()
                .find(|item| item.kind == "compaction")
                .and_then(|item| item.encrypted_content.as_deref())
                .map_or(0, |value| value.len() as u64);
            let turn = turns.entry(current_turn.clone()).or_default();
            turn.id = current_turn.clone();
            turn.model = current_model.clone();
            turn.compaction_events.push(TraceCompactionEvent {
                source_order,
                notification_source_order: None,
                timestamp_ms,
                window_number: compacted.payload.window_number,
                history_items,
                encrypted_summary_bytes,
            });
            continue;
        }

        let is_output = memmem::find(&line, b"function_call_output").is_some()
            || memmem::find(&line, b"custom_tool_call_output").is_some()
            || memmem::find(&line, b"tool_search_output").is_some();
        if is_output && line.len() as u64 >= LARGE_OUTPUT_BYTES {
            large_tool_outputs += 1;
            large_tool_output_bytes = large_tool_output_bytes.saturating_add(line.len() as u64);
        }
        if line.len() > MAX_PARSED_LINE_BYTES {
            if is_output {
                let turn = turns.entry(current_turn.clone()).or_default();
                turn.id = current_turn.clone();
                turn.unattributed_large_outputs += 1;
                turn.unattributed_large_output_bytes = turn
                    .unattributed_large_output_bytes
                    .saturating_add(line.len() as u64);
            }
            continue;
        }

        let Ok(entry) = serde_json::from_slice::<Value>(&line) else {
            malformed_lines += 1;
            continue;
        };
        let timestamp = entry
            .get("timestamp")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(timestamp) = &timestamp {
            if started_at
                .as_ref()
                .is_none_or(|current| timestamp < current)
            {
                started_at = Some(timestamp.clone());
            }
            if updated_at
                .as_ref()
                .is_none_or(|current| timestamp > current)
            {
                updated_at = Some(timestamp.clone());
            }
        }
        let payload = entry.get("payload").unwrap_or(&Value::Null);
        let entry_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
        let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");

        if entry_type == "session_meta" {
            session_meta_source_order.get_or_insert(source_order);
            session_meta_at_ms =
                session_meta_at_ms.or_else(|| timestamp.as_deref().and_then(timestamp_millis));
            session_id = session_id.or_else(|| {
                payload
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            parent_id = parent_id.or_else(|| {
                payload
                    .get("forked_from_id")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        payload
                            .pointer("/source/subagent/thread_spawn/parent_thread_id")
                            .and_then(Value::as_str)
                    })
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            });
            if parent_id.is_some() && forked_at_ms.is_none() {
                forked_at_ms = timestamp.as_deref().and_then(timestamp_millis);
            }
            source = source.or_else(|| {
                payload
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        payload
                            .get("source")
                            .and_then(Value::as_object)
                            .and_then(|source| source.keys().next().cloned())
                    })
            });
            cwd = cwd.or_else(|| {
                payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            base_instructions_bytes = base_instructions_bytes.max(
                payload
                    .get("base_instructions")
                    .map_or(0, serialized_value_bytes),
            );
            if let Some(dynamic_tools) = payload.get("dynamic_tools") {
                dynamic_tool_definitions =
                    dynamic_tool_definitions.max(value_item_count(dynamic_tools));
                dynamic_tool_bytes = dynamic_tool_bytes.max(serialized_value_bytes(dynamic_tools));
            }
            continue;
        }
        if entry_type == "world_state" {
            let turn = turns.entry(current_turn.clone()).or_default();
            turn.id = current_turn.clone();
            turn.model = current_model.clone();
            turn.world_state_bytes = turn
                .world_state_bytes
                .saturating_add(serialized_value_bytes(payload));
            continue;
        }
        if entry_type == "turn_context" {
            if let Some(next_model) = payload.get("model").and_then(Value::as_str) {
                current_model = next_model.to_string();
                model = Some(next_model.to_string());
            }
            if let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) {
                current_turn = turn_id.to_string();
                turns_with_context.insert(current_turn.clone());
                let turn = turns.entry(current_turn.clone()).or_default();
                turn.id = current_turn.clone();
                turn.model = current_model.clone();
                turn.reasoning_effort = payload
                    .get("effort")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        payload
                            .pointer("/collaboration_mode/settings/reasoning_effort")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });
                turn.summary_mode = payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                turn.context_source_order = Some(source_order);
                turn.turn_context_bytes = turn
                    .turn_context_bytes
                    .saturating_add(serialized_value_bytes(payload));
            }
            continue;
        }
        if entry_type == "event_msg" && payload_type == "task_started" {
            if let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) {
                current_turn = turn_id.to_string();
                open_turns.insert(current_turn.clone());
                let turn = turns.entry(current_turn.clone()).or_default();
                turn.id = current_turn.clone();
                turn.model = current_model.clone();
                turn.started_source_order = Some(source_order);
                turn.started_at_ms = payload
                    .get("started_at")
                    .and_then(timestamp_value_millis)
                    .or_else(|| timestamp.as_deref().and_then(timestamp_millis));
            }
            continue;
        }
        if entry_type == "event_msg" && payload_type == "task_complete" {
            if let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) {
                open_turns.remove(turn_id);
                let turn = turns.entry(turn_id.to_string()).or_default();
                turn.id = turn_id.to_string();
                turn.completed_source_order = Some(source_order);
                turn.completed_at_ms = payload
                    .get("completed_at")
                    .and_then(timestamp_value_millis)
                    .or_else(|| timestamp.as_deref().and_then(timestamp_millis));
                turn.duration_ms = payload.get("duration_ms").and_then(Value::as_u64);
            }
            continue;
        }
        if entry_type == "event_msg" && payload_type == "context_compacted" {
            let timestamp_ms = timestamp
                .as_deref()
                .and_then(timestamp_millis)
                .unwrap_or_default();
            let turn = turns.entry(current_turn.clone()).or_default();
            turn.id = current_turn.clone();
            turn.model = current_model.clone();
            if let Some(compaction) = turn
                .compaction_events
                .iter_mut()
                .rev()
                .find(|event| event.notification_source_order.is_none())
            {
                compaction.notification_source_order = Some(source_order);
            } else {
                turn.compaction_events.push(TraceCompactionEvent {
                    source_order,
                    notification_source_order: None,
                    timestamp_ms,
                    ..TraceCompactionEvent::default()
                });
            }
            context_compactions += 1;
            continue;
        }
        if entry_type == "event_msg"
            && payload_type == "patch_apply_end"
            && payload.get("success").and_then(Value::as_bool) == Some(false)
        {
            failed_tool_calls += 1;
            let turn_id = payload
                .get("turn_id")
                .and_then(Value::as_str)
                .unwrap_or(&current_turn)
                .to_string();
            let turn = turns.entry(turn_id.clone()).or_default();
            turn.id = turn_id;
            turn.model = current_model.clone();
            turn.structured_failures += 1;
            continue;
        }
        if entry_type == "event_msg"
            && payload_type == "mcp_tool_call_end"
            && payload.pointer("/result/isError").and_then(Value::as_bool) == Some(true)
        {
            failed_tool_calls += 1;
            let turn_id = payload
                .get("turn_id")
                .and_then(Value::as_str)
                .unwrap_or(&current_turn)
                .to_string();
            let turn = turns.entry(turn_id.clone()).or_default();
            turn.id = turn_id;
            turn.model = current_model.clone();
            turn.structured_failures += 1;
            continue;
        }
        if entry_type == "event_msg" && payload_type == "token_count" {
            let Some(info) = payload.get("info").filter(|value| value.is_object()) else {
                continue;
            };
            let total = info.get("total_token_usage").and_then(parse_usage);
            if total.as_ref().is_some_and(|value| {
                previous_total.as_ref().is_some_and(|previous| {
                    previous.input_tokens == value.input_tokens
                        && previous.cached_input_tokens == value.cached_input_tokens
                        && previous.output_tokens == value.output_tokens
                        && previous.total_tokens == value.total_tokens
                })
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
            if usage.total_tokens == 0 && usage.input_tokens == 0 && usage.output_tokens == 0 {
                continue;
            }
            let Some(timestamp_ms) = timestamp.as_deref().and_then(timestamp_millis) else {
                continue;
            };
            let event_model = info
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&current_model)
                .to_string();
            usage_events.push(TraceUsageEvent {
                source_order,
                timestamp_ms,
                turn_id: current_turn.clone(),
                model: event_model,
                input_tokens: usage.input_tokens,
                cached_input_tokens: usage.cached_input_tokens.min(usage.input_tokens),
                output_tokens: usage.output_tokens,
                reasoning_output_tokens: usage.reasoning_output_tokens,
                total_tokens: usage.total_tokens,
                context_window: info.get("model_context_window").and_then(Value::as_u64),
            });
            continue;
        }
        if entry_type == "event_msg" && payload_type == "web_search_end" {
            if let Some(call_id) = payload.get("call_id").and_then(Value::as_str)
                && let Some((turn_id, event_index)) = pending_calls.get(call_id)
                && let Some(event) = turns
                    .get_mut(turn_id)
                    .and_then(|turn| turn.tool_events.get_mut(*event_index))
            {
                event.execution_completed_source_order = Some(source_order);
                event.execution_completed_at_ms = timestamp.as_deref().and_then(timestamp_millis);
                event.execution_end_source_type = Some("event_msg.web_search_end".to_string());
            }
            continue;
        }
        if entry_type == "event_msg"
            && matches!(payload_type, "user_message" | "agent_message")
            && !current_turn.is_empty()
            && (payload_type == "agent_message" || turns_with_context.contains(&current_turn))
        {
            let Some(timestamp_ms) = timestamp.as_deref().and_then(timestamp_millis) else {
                continue;
            };
            if payload_type == "agent_message" {
                let memory_citation = payload.get("memory_citation").unwrap_or(&Value::Null);
                if !memory_citation.is_null() {
                    let turn = turns.entry(current_turn.clone()).or_default();
                    turn.memory_citations = turn
                        .memory_citations
                        .saturating_add(value_item_count(memory_citation));
                }
            }
            let content = payload
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string);
            let phase = if payload_type == "user_message" {
                "user_prompt".to_string()
            } else {
                payload
                    .get("phase")
                    .and_then(Value::as_str)
                    .unwrap_or("assistant")
                    .to_string()
            };
            let turn = turns.entry(current_turn.clone()).or_default();
            turn.id = current_turn.clone();
            turn.model = current_model.clone();
            turn.phase_events.push(TracePhaseEvent {
                source_order,
                source_end_order: None,
                timestamp_ms,
                turn_id: current_turn.clone(),
                phase,
                source_type: format!("event_msg.{payload_type}"),
                role: Some(
                    if payload_type == "user_message" {
                        "user"
                    } else {
                        "assistant"
                    }
                    .to_string(),
                ),
                content_parts: usize::from(content.is_some()),
                content_bytes: content.as_ref().map_or(0, |value| value.len() as u64),
                content,
                summary_parts: 0,
                encrypted_bytes: 0,
            });
            continue;
        }
        let response_role = payload.get("role").and_then(Value::as_str);
        if entry_type == "response_item"
            && payload_type == "message"
            && response_role == Some("developer")
            && current_turn != "unassigned"
        {
            let content = payload.get("content").unwrap_or(&Value::Null);
            let content_bytes = serialized_value_bytes(content);
            let content_text = extract_content_text(content).unwrap_or_default();
            let memory_context = content_text.contains("<memories>")
                || content_text.contains("<memory_context>")
                || content_text.contains("memory_context");
            let turn = turns.entry(current_turn.clone()).or_default();
            turn.id = current_turn.clone();
            turn.model = current_model.clone();
            turn.developer_context_bytes =
                turn.developer_context_bytes.saturating_add(content_bytes);
            if memory_context {
                turn.memory_context_bytes = turn.memory_context_bytes.saturating_add(content_bytes);
            }
            continue;
        }
        let is_user_prompt = payload_type == "message"
            && response_role == Some("user")
            && turns_with_context.contains(&current_turn);
        if entry_type == "response_item"
            && (payload_type == "reasoning"
                || is_user_prompt
                || (payload_type == "message" && response_role == Some("assistant")))
        {
            let Some(timestamp_ms) = timestamp.as_deref().and_then(timestamp_millis) else {
                continue;
            };
            let content = payload.get("content").unwrap_or(&Value::Null);
            let summary = payload.get("summary").unwrap_or(&Value::Null);
            let phase = if payload_type == "reasoning" {
                "reasoning".to_string()
            } else if is_user_prompt {
                "user_prompt".to_string()
            } else {
                payload
                    .get("phase")
                    .and_then(Value::as_str)
                    .unwrap_or("assistant")
                    .to_string()
            };
            let turn = turns.entry(current_turn.clone()).or_default();
            turn.id = current_turn.clone();
            turn.model = current_model.clone();
            turn.phase_events.push(TracePhaseEvent {
                source_order,
                source_end_order: None,
                timestamp_ms,
                turn_id: current_turn.clone(),
                phase,
                source_type: format!("response_item.{payload_type}"),
                role: payload
                    .get("role")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                content: if payload_type == "reasoning" {
                    extract_content_text(summary)
                } else {
                    extract_content_text(content)
                },
                content_parts: value_item_count(content),
                content_bytes: serialized_value_bytes(content),
                summary_parts: value_item_count(summary),
                encrypted_bytes: payload
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .map_or(0, |value| value.len() as u64),
            });
            continue;
        }
        if entry_type == "response_item"
            && matches!(
                payload_type,
                "function_call" | "custom_tool_call" | "tool_search_call"
            )
        {
            tool_calls += 1;
            let namespace = payload
                .get("namespace")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty());
            let name = payload.get("name").and_then(Value::as_str).unwrap_or(
                if payload_type == "tool_search_call" {
                    "tool_search"
                } else {
                    "unknown"
                },
            );
            let tool_name = namespace
                .map(|namespace| format!("{namespace}.{name}"))
                .unwrap_or_else(|| name.to_string());
            *tool_counts.entry(tool_name.clone()).or_default() += 1;
            let input = payload
                .get("arguments")
                .or_else(|| payload.get("input"))
                .cloned()
                .unwrap_or(Value::Null);
            let presentation = describe_tool_call(&tool_name, namespace, &input);
            let arguments_json = trace_detail_json(&input);
            let signature = call_signature(&tool_name, &input);
            let previous_calls = *call_counts.get(&signature).unwrap_or(&0);
            *call_counts.entry(signature.clone()).or_default() += 1;
            let paths = read_paths(&tool_name, &input);
            for path in &paths {
                *read_counts.entry(path.clone()).or_default() += 1;
            }
            let subject = paths
                .into_iter()
                .next()
                .or_else(|| presentation.subject.clone());
            let timestamp_ms = timestamp
                .as_deref()
                .and_then(timestamp_millis)
                .unwrap_or_default();
            let turn = turns.entry(current_turn.clone()).or_default();
            turn.id = current_turn.clone();
            turn.model = current_model.clone();
            let event_index = turn.tool_events.len();
            turn.tool_events.push(TraceToolEvent {
                source_order,
                completed_source_order: None,
                execution_completed_source_order: None,
                timestamp_ms,
                completed_at_ms: None,
                execution_completed_at_ms: None,
                turn_id: current_turn.clone(),
                call_id: payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                source_type: payload_type.to_string(),
                execution_end_source_type: None,
                result_source_type: None,
                name: tool_name.clone(),
                category: presentation.category,
                server: presentation.server,
                subject,
                detail: presentation.detail,
                arguments: presentation.arguments,
                arguments_json,
                result_fields: Vec::new(),
                result_json: None,
                signature,
                repeated: previous_calls > 0,
                failed: false,
                output_bytes: 0,
                exit_code: None,
            });
            if let Some(call_id) = payload.get("call_id").and_then(Value::as_str) {
                pending_calls.insert(call_id.to_string(), (current_turn.clone(), event_index));
            }
            continue;
        }
        if entry_type == "response_item"
            && matches!(
                payload_type,
                "function_call_output" | "custom_tool_call_output" | "tool_search_output"
            )
        {
            let output = payload.get("output").unwrap_or(payload);
            let exit_code = extract_exit_code(output);
            let mut result_fields = extract_result_fields(output);
            let result_json = trace_detail_json(output);
            if let Some(exit_code) = exit_code
                && !result_fields.iter().any(|field| field.key == "exit_code")
            {
                result_fields.push(TraceEventField {
                    key: "exit_code".to_string(),
                    value: exit_code.to_string(),
                });
            }
            let failed = output_failed(&line) || exit_code.is_some_and(|code| code != 0);
            if failed {
                failed_tool_calls += 1;
            }
            if let Some(call_id) = payload.get("call_id").and_then(Value::as_str)
                && let Some((turn_id, event_index)) = pending_calls.remove(call_id)
                && let Some(event) = turns
                    .get_mut(&turn_id)
                    .and_then(|turn| turn.tool_events.get_mut(event_index))
            {
                event.completed_source_order = Some(source_order);
                event.completed_at_ms = timestamp.as_deref().and_then(timestamp_millis);
                event.result_source_type = Some(format!("response_item.{payload_type}"));
                event.failed = failed;
                event.output_bytes = line.len() as u64;
                event.exit_code = exit_code;
                event.result_fields = result_fields;
                event.result_json = result_json;
            }
        }
    }

    for turn in turns.values_mut() {
        merge_mirrored_phase_events(&mut turn.phase_events);
    }

    let repeated_tool_calls = call_counts
        .values()
        .map(|count| count.saturating_sub(1))
        .sum();
    let repeated_reads = read_counts
        .values()
        .map(|count| count.saturating_sub(1))
        .sum();
    let top_repeated_tool = tool_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 1)
        .map(|(subject, count)| RepeatedSubject { subject, count });
    let top_repeated_path = read_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 1)
        .map(|(subject, count)| RepeatedSubject { subject, count });

    Ok(IndexedTraceFile {
        length,
        modified_ms,
        session_id,
        parent_id,
        forked_at_ms,
        source,
        cwd,
        session_meta_source_order,
        session_meta_at_ms,
        conversation_name: None,
        official_status: None,
        started_at,
        updated_at,
        model,
        base_instructions_bytes,
        dynamic_tool_definitions,
        dynamic_tool_bytes,
        turns,
        open_turn_ids: open_turns.iter().cloned().collect(),
        usage_events,
        tool_calls,
        failed_tool_calls,
        repeated_tool_calls,
        repeated_reads,
        top_repeated_tool,
        top_repeated_path,
        context_compactions,
        large_tool_outputs,
        large_tool_output_bytes,
        malformed_lines,
        last_turn_open: !open_turns.is_empty(),
    })
}

#[derive(Default)]
struct ToolPresentation {
    category: String,
    server: Option<String>,
    subject: Option<String>,
    detail: Option<String>,
    arguments: Vec<TraceEventField>,
}

fn describe_tool_call(tool_name: &str, namespace: Option<&str>, input: &Value) -> ToolPresentation {
    let parsed = parse_tool_input(input);
    let raw = input
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| input.to_string());
    let normalized = tool_name.to_ascii_lowercase();
    let server = namespace
        .filter(|value| value.starts_with("mcp__"))
        .map(|value| value.trim_start_matches("mcp__").to_string())
        .or_else(|| {
            tool_name
                .strip_prefix("mcp__")
                .and_then(|value| value.split('.').next())
                .map(str::to_string)
        });
    let skill = extract_skill_reference(&raw);

    let mut category = if skill.is_some() {
        "skill"
    } else if server.is_some() {
        "mcp"
    } else if normalized == "exec_command"
        || normalized.ends_with(".exec_command")
        || normalized == "write_stdin"
        || normalized.ends_with(".write_stdin")
        || normalized == "wait"
        || normalized.ends_with(".wait")
    {
        "cli"
    } else if normalized == "exec" || normalized.ends_with(".exec") {
        if raw.contains("tools.exec_command") {
            "cli"
        } else {
            "automation"
        }
    } else if normalized.contains("apply_patch")
        || normalized.contains("read_file")
        || normalized.contains("read_mcp_resource")
        || normalized.contains("view_image")
        || normalized.contains("filesystem")
    {
        "file"
    } else if normalized.contains("browser")
        || normalized.contains("chrome")
        || normalized.contains("playwright")
        || normalized.contains("computer_use")
        || normalized.contains("computer-use")
        || normalized.contains("web__run")
        || normalized == "web.run"
        || normalized.starts_with("web.")
    {
        "browser"
    } else if normalized.contains("node_repl")
        || normalized.ends_with(".js")
        || normalized.contains("automation")
    {
        "automation"
    } else if normalized.contains("spawn_agent")
        || normalized.contains("collaboration")
        || normalized.contains("create_thread")
        || normalized.contains("send_message_to_thread")
    {
        "agent"
    } else {
        "tool"
    }
    .to_string();

    let mut subject = None;
    let mut detail = None;
    if let Some((skill_name, skill_path)) = skill {
        category = "skill".to_string();
        subject = Some(short_path(&skill_path));
        detail = Some(skill_name);
    } else if normalized.contains("apply_patch") {
        let files = extract_patch_files(&raw);
        if !files.is_empty() {
            subject = files.first().cloned();
            detail = Some(files.join(", "));
        }
    } else {
        for key in ["cmd", "code", "query", "q", "url", "jql_str", "title"] {
            if let Some(value) = parsed.get(key).and_then(Value::as_str) {
                detail = Some(preview_text(value, 640));
                break;
            }
        }
        if detail.is_none() && (normalized == "exec" || normalized.ends_with(".exec")) {
            detail = Some(preview_text(&raw, 640));
        }
    }

    let mut arguments = summarize_arguments(&parsed);
    let nested_tools = extract_nested_tools(&raw);
    if !nested_tools.is_empty() {
        arguments.push(TraceEventField {
            key: "nested_tools".to_string(),
            value: nested_tools.join(", "),
        });
    }

    ToolPresentation {
        category,
        server,
        subject,
        detail: detail.filter(|value| !value.is_empty()),
        arguments,
    }
}

fn parse_tool_input(input: &Value) -> Value {
    input
        .as_str()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| input.clone())
}

fn value_item_count(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Array(values) => values.len(),
        _ => 1,
    }
}

fn serialized_value_bytes(value: &Value) -> u64 {
    if value.is_null() {
        0
    } else {
        serde_json::to_vec(value).map_or(0, |bytes| bytes.len() as u64)
    }
}

fn trace_detail_json(value: &Value) -> Option<String> {
    let parsed = parse_tool_input(value);
    if parsed.is_null() {
        return None;
    }
    let rendered = if let Value::String(text) = &parsed {
        text.clone()
    } else {
        serde_json::to_string_pretty(&parsed).ok()?
    };
    Some(truncate_text(&rendered, MAX_TRACE_DETAIL_CHARS))
}

fn extract_content_text(value: &Value) -> Option<String> {
    let mut parts = Vec::new();
    match value {
        Value::String(text) => parts.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                } else if let Some(text) = item.get("input_text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                } else if let Some(text) = item.get("output_text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
        }
        Value::Object(item) => {
            if let Some(text) = item
                .get("text")
                .or_else(|| item.get("input_text"))
                .or_else(|| item.get("output_text"))
                .and_then(Value::as_str)
            {
                parts.push(text.to_string());
            }
        }
        _ => {}
    }
    let joined = parts.join("\n\n");
    (!joined.is_empty()).then(|| truncate_text(&joined, MAX_TRACE_DETAIL_CHARS))
}

fn extract_result_fields(output: &Value) -> Vec<TraceEventField> {
    let mut fields = Vec::new();
    fields.push(TraceEventField {
        key: "result_type".to_string(),
        value: match output {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "text",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
        .to_string(),
    });
    match output {
        Value::Array(values) => {
            fields.push(TraceEventField {
                key: "items".to_string(),
                value: values.len().to_string(),
            });
            let content_types = values
                .iter()
                .filter_map(|value| value.get("type").and_then(Value::as_str))
                .collect::<BTreeSet<_>>();
            if !content_types.is_empty() {
                fields.push(TraceEventField {
                    key: "content_types".to_string(),
                    value: content_types.into_iter().collect::<Vec<_>>().join(", "),
                });
            }
        }
        Value::Object(values) => fields.push(TraceEventField {
            key: "fields".to_string(),
            value: values.len().to_string(),
        }),
        Value::String(value) => fields.push(TraceEventField {
            key: "text_bytes".to_string(),
            value: value.len().to_string(),
        }),
        _ => {}
    }
    collect_safe_result_metadata(output, "", &mut fields, 0);
    let mut seen = HashSet::new();
    fields.retain(|field| seen.insert((field.key.clone(), field.value.clone())));
    fields.truncate(18);
    fields
}

fn collect_safe_result_metadata(
    value: &Value,
    path: &str,
    fields: &mut Vec<TraceEventField>,
    depth: usize,
) {
    if depth >= 5 || fields.len() >= 24 {
        return;
    }
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                let next_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if is_safe_result_key(key) {
                    let rendered = match value {
                        Value::Null => "null".to_string(),
                        Value::Bool(value) => value.to_string(),
                        Value::Number(value) => value.to_string(),
                        Value::String(value) => sanitize_text(value, 160),
                        Value::Array(values) => format!("[{} items]", values.len()),
                        Value::Object(values) => format!("{{{} fields}}", values.len()),
                    };
                    fields.push(TraceEventField {
                        key: next_path.clone(),
                        value: rendered,
                    });
                }
                collect_safe_result_metadata(value, &next_path, fields, depth + 1);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().take(12).enumerate() {
                let next_path = if path.is_empty() {
                    format!("[{index}]")
                } else {
                    format!("{path}[{index}]")
                };
                collect_safe_result_metadata(value, &next_path, fields, depth + 1);
            }
        }
        Value::String(value) => {
            let trimmed = value.trim();
            if (trimmed.starts_with('{') || trimmed.starts_with('['))
                && let Ok(parsed) = serde_json::from_str::<Value>(trimmed)
            {
                collect_safe_result_metadata(&parsed, path, fields, depth + 1);
            }
        }
        _ => {}
    }
}

fn is_safe_result_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "status"
            | "iserror"
            | "is_error"
            | "exit_code"
            | "wall_time_seconds"
            | "elapsed_ms"
            | "duration_ms"
            | "original_token_count"
            | "chunk_id"
            | "session_id"
            | "cell_id"
            | "content_count"
            | "tools_count"
            | "error_code"
    )
}

fn summarize_arguments(input: &Value) -> Vec<TraceEventField> {
    let Some(object) = input.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .take(12)
        .map(|(key, value)| TraceEventField {
            key: key.clone(),
            value: summarize_argument_value(key, value),
        })
        .collect()
}

fn summarize_argument_value(key: &str, value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => {
            if key.eq_ignore_ascii_case("patch") || key.eq_ignore_ascii_case("input") {
                let files = extract_patch_files(value);
                if !files.is_empty() {
                    return files.join(", ");
                }
            }
            preview_text(value, 240)
        }
        Value::Array(values) => format!("[{} items]", values.len()),
        Value::Object(values) => format!("{{{} fields}}", values.len()),
    }
}

fn preview_text(value: &str, limit: usize) -> String {
    truncate_text(
        &value.split_whitespace().collect::<Vec<_>>().join(" "),
        limit,
    )
}

fn sanitize_text(value: &str, limit: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut sanitized = collapsed
        .split(' ')
        .map(redact_token)
        .collect::<Vec<_>>()
        .join(" ");
    if sanitized.to_ascii_lowercase().contains("bearer ") {
        let words = sanitized.split_whitespace().collect::<Vec<_>>();
        let mut output = Vec::with_capacity(words.len());
        let mut redact_next = false;
        for word in words {
            if redact_next {
                output.push("[redacted]");
                redact_next = false;
            } else {
                redact_next = word.eq_ignore_ascii_case("bearer");
                output.push(word);
            }
        }
        sanitized = output.join(" ");
    }
    truncate_text(&sanitized, limit)
}

fn redact_token(token: &str) -> &str {
    let normalized = token
        .trim_matches(|character: char| {
            matches!(
                character,
                '"' | '\'' | '`' | ',' | ';' | '{' | '}' | '(' | ')'
            )
        })
        .to_ascii_lowercase();
    if normalized.starts_with("sk-")
        || [
            "token=",
            "secret=",
            "password=",
            "passwd=",
            "api_key=",
            "apikey=",
            "authorization=",
            "cookie=",
        ]
        .iter()
        .any(|needle| normalized.starts_with(needle))
    {
        "[redacted]"
    } else {
        token
    }
}

fn truncate_text(value: &str, limit: usize) -> String {
    let mut output = value.chars().take(limit).collect::<String>();
    if value.chars().count() > limit {
        output.push('…');
    }
    output
}

fn extract_patch_files(value: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    for line in value.lines() {
        for prefix in [
            "*** Add File: ",
            "*** Update File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ] {
            if let Some(path) = line.trim().strip_prefix(prefix) {
                files.insert(short_path(path.trim()));
            }
        }
    }
    files.into_iter().take(12).collect()
}

fn extract_skill_reference(value: &str) -> Option<(String, String)> {
    let marker = "SKILL.md";
    let marker_start = value.find(marker)?;
    let bytes = value.as_bytes();
    let mut start = marker_start;
    while start > 0 {
        let character = bytes[start - 1] as char;
        if character.is_whitespace()
            || matches!(character, '"' | '\'' | '`' | '(' | ')' | '{' | '}' | ',')
        {
            break;
        }
        start -= 1;
    }
    let path = value[start..marker_start + marker.len()]
        .trim_matches(|character: char| {
            matches!(
                character,
                '"' | '\'' | '`' | ',' | ';' | ':' | '(' | ')' | '{' | '}'
            )
        })
        .to_string();
    let skill_name = Path::new(&path)
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or("skill")
        .to_string();
    Some((skill_name, path))
}

fn extract_nested_tools(value: &str) -> Vec<String> {
    let mut tools = BTreeSet::new();
    let mut remaining = value;
    while let Some(index) = remaining.find("tools.") {
        let suffix = &remaining[index + "tools.".len()..];
        let name = suffix
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect::<String>();
        let name_length = name.len();
        if !name.is_empty() {
            tools.insert(name);
        }
        remaining = &suffix[name_length..];
    }
    tools.into_iter().take(12).collect()
}

fn extract_exit_code(value: &Value) -> Option<i64> {
    if let Some(code) = value.get("exit_code").and_then(Value::as_i64) {
        return Some(code);
    }
    if let Some(values) = value.as_array() {
        return values.iter().find_map(extract_exit_code);
    }
    if let Some(object) = value.as_object() {
        return object.values().find_map(extract_exit_code);
    }
    let raw = value.as_str()?;
    for marker in ["Process exited with code ", "exit code: ", "\"exit_code\":"] {
        if let Some(position) = raw.find(marker) {
            let suffix = raw[position + marker.len()..].trim_start();
            let number = suffix
                .chars()
                .take_while(|character| character.is_ascii_digit() || *character == '-')
                .collect::<String>();
            if let Ok(code) = number.parse::<i64>() {
                return Some(code);
            }
        }
    }
    serde_json::from_str::<Value>(raw)
        .ok()
        .as_ref()
        .and_then(extract_exit_code)
}

fn output_failed(line: &[u8]) -> bool {
    [
        b"Process exited with code 1".as_slice(),
        b"Process exited with code 2".as_slice(),
        b"exit code: 1".as_slice(),
        b"\\\"is_error\\\":true".as_slice(),
        b"\\\"isError\\\":true".as_slice(),
        b"Tool call failed".as_slice(),
        b"timed out".as_slice(),
    ]
    .iter()
    .any(|needle| memmem::find(line, needle).is_some())
}

fn call_signature(tool_name: &str, input: &Value) -> String {
    let mut hasher = DefaultHasher::new();
    tool_name.hash(&mut hasher);
    input.to_string().hash(&mut hasher);
    format!("{tool_name}:{:x}", hasher.finish())
}

fn read_paths(tool_name: &str, input: &Value) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    let parsed = if let Some(raw) = input.as_str() {
        serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
    } else {
        input.clone()
    };
    let workdir = parsed
        .get("workdir")
        .and_then(Value::as_str)
        .map(PathBuf::from);

    if tool_name.contains("view_image") || tool_name.contains("read") {
        for key in ["path", "file", "uri"] {
            if let Some(path) = parsed.get(key).and_then(Value::as_str) {
                output.insert(short_path(path));
            }
        }
    }
    if tool_name.ends_with("exec_command")
        && let Some(command) = parsed.get("cmd").and_then(Value::as_str)
        && command_reads_files(command)
    {
        for token in command.split_whitespace() {
            let token = token.trim_matches(|character: char| {
                matches!(
                    character,
                    '\'' | '"' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            });
            if looks_like_path(token) {
                let path = Path::new(token);
                let normalized = if path.is_relative() {
                    workdir
                        .as_ref()
                        .map(|workdir| workdir.join(path))
                        .unwrap_or_else(|| path.to_path_buf())
                } else {
                    path.to_path_buf()
                };
                output.insert(short_path(&normalized.to_string_lossy()));
            }
        }
    }
    output
}

fn command_reads_files(command: &str) -> bool {
    command.split_whitespace().any(|token| {
        matches!(
            token.trim_matches(|character: char| !character.is_alphanumeric() && character != '_'),
            "cat" | "sed" | "rg" | "grep" | "head" | "tail" | "find" | "ls" | "wc" | "stat" | "du"
        )
    })
}

fn looks_like_path(token: &str) -> bool {
    if token.is_empty()
        || token.starts_with('-')
        || token.contains("://")
        || token.contains('$')
        || token.contains('|')
        || token.contains('*')
    {
        return false;
    }
    token.contains('/')
        || [
            ".rs", ".ts", ".tsx", ".js", ".jsx", ".json", ".jsonl", ".toml", ".md", ".css",
            ".html", ".py", ".go", ".java", ".kt", ".swift", ".yaml", ".yml",
        ]
        .iter()
        .any(|extension| token.ends_with(extension))
}

fn short_path(value: &str) -> String {
    let path = Path::new(value);
    let components = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    if components.len() <= 2 {
        value.to_string()
    } else {
        components[components.len() - 2..].join("/")
    }
}

fn project_identity(cwd: Option<&str>) -> (String, String) {
    let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) else {
        return ("Codex".to_string(), String::new());
    };
    let project_path = Path::new(cwd);
    let label = project_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Codex")
        .to_string();
    (label, project_path.to_string_lossy().to_string())
}

fn parse_usage(value: &Value) -> Option<RawUsage> {
    if !value.is_object() {
        return None;
    }
    let read = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    let input_tokens = read("input_tokens");
    let output_tokens = read("output_tokens");
    let reported_total = read("total_tokens");
    Some(RawUsage {
        input_tokens,
        cached_input_tokens: read("cached_input_tokens").min(input_tokens),
        output_tokens,
        reasoning_output_tokens: read("reasoning_output_tokens").min(output_tokens),
        total_tokens: if reported_total == 0 {
            input_tokens.saturating_add(output_tokens)
        } else {
            reported_total
        },
    })
}

fn replayed_prefix_len(
    child: &IndexedTraceFile,
    records_by_session: &HashMap<&str, &IndexedTraceFile>,
) -> usize {
    let Some(parent_id) = child.parent_id.as_deref() else {
        return 0;
    };
    let parent_events = records_by_session
        .get(parent_id)
        .copied()
        .filter(|parent| !std::ptr::eq(*parent, child))
        .map(|parent| {
            parent
                .usage_events
                .iter()
                .take_while(|event| {
                    child
                        .forked_at_ms
                        .is_none_or(|forked_at| event.timestamp_ms <= forked_at)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut matched = 0;
    for (index, event) in child.usage_events.iter().enumerate() {
        if parent_events
            .get(matched)
            .is_some_and(|parent| same_usage(parent, event))
        {
            matched += 1;
            continue;
        }
        return if matched > 0 { index } else { 0 };
    }
    child.usage_events.len()
}

fn same_usage(left: &TraceUsageEvent, right: &TraceUsageEvent) -> bool {
    left.input_tokens == right.input_tokens
        && left.cached_input_tokens == right.cached_input_tokens
        && left.output_tokens == right.output_tokens
        && left.reasoning_output_tokens == right.reasoning_output_tokens
        && left.total_tokens == right.total_tokens
}

fn timestamp_millis(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn timestamp_value_millis(value: &Value) -> Option<i64> {
    if let Some(value) = value.as_str() {
        return timestamp_millis(value);
    }
    let numeric = value.as_f64()?;
    if !numeric.is_finite() {
        return None;
    }
    let millis = if numeric.abs() < 10_000_000_000.0 {
        numeric * 1_000.0
    } else {
        numeric
    };
    Some(millis.round() as i64)
}

fn load_index(path: &Path, warnings: &mut Vec<String>) -> TraceIndex {
    match read_index_entries(path, TRACE_INDEX_NAMESPACE, TRACE_INDEX_SCHEMA_VERSION) {
        Ok(entries) => {
            let files = entries
                .into_iter()
                .map(|(session_path, payload)| {
                    serde_json::from_slice::<IndexedTraceFile>(&payload)
                        .map(|indexed| (session_path, indexed))
                        .map_err(|error| format!("过程索引记录无法解析：{error}"))
                })
                .collect::<Result<BTreeMap<_, _>, _>>();
            match files {
                Ok(files) => {
                    let index = TraceIndex {
                        schema_version: TRACE_INDEX_SCHEMA_VERSION,
                        files,
                    };
                    if let Err(error) = write_trace_mirror(path, &trace_mirror_sessions(&index)) {
                        warnings.push(format!("Trace 关系索引无法同步：{error}"));
                    }
                    index
                }
                Err(error) => {
                    warnings.push(format!("{error}，正在重建"));
                    TraceIndex::default()
                }
            }
        }
        Err(error) => {
            warnings.push(format!("过程索引无法读取，正在重建：{error}"));
            TraceIndex::default()
        }
    }
}

fn write_index(path: &Path, index: &TraceIndex) -> Result<(), String> {
    let entries = index
        .files
        .iter()
        .map(|(session_path, indexed)| {
            serde_json::to_vec(indexed)
                .map(|payload| (session_path.clone(), payload))
                .map_err(|error| format!("过程索引无法序列化：{error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    write_index_entries(
        path,
        TRACE_INDEX_NAMESPACE,
        TRACE_INDEX_SCHEMA_VERSION,
        &entries,
    )?;
    write_trace_mirror(path, &trace_mirror_sessions(index))
}

fn trace_mirror_sessions(index: &TraceIndex) -> Vec<TraceMirrorSession> {
    index
        .files
        .iter()
        .map(|(session_path, indexed)| {
            let turns = indexed
                .turns
                .iter()
                .map(|(turn_id, turn)| TraceMirrorTurn {
                    turn_id: turn_id.clone(),
                    model: turn.model.clone(),
                    reasoning_effort: turn.reasoning_effort.clone(),
                    summary_mode: turn.summary_mode.clone(),
                    started_source_order: turn.started_source_order,
                    started_at_ms: turn.started_at_ms,
                    completed_source_order: turn.completed_source_order,
                    completed_at_ms: turn.completed_at_ms,
                    duration_ms: turn.duration_ms,
                    structured_failures: turn.structured_failures,
                    context_compactions: turn.compaction_events.len(),
                })
                .collect();
            let tool_events = indexed
                .turns
                .values()
                .flat_map(|turn| turn.tool_events.iter())
                .map(|event| TraceMirrorToolEvent {
                    turn_id: event.turn_id.clone(),
                    source_order: event.source_order,
                    completed_source_order: event.completed_source_order,
                    execution_completed_source_order: event.execution_completed_source_order,
                    timestamp_ms: event.timestamp_ms,
                    completed_at_ms: event.completed_at_ms,
                    execution_completed_at_ms: event.execution_completed_at_ms,
                    call_id: event.call_id.clone(),
                    source_type: event.source_type.clone(),
                    execution_end_source_type: event.execution_end_source_type.clone(),
                    result_source_type: event.result_source_type.clone(),
                    name: event.name.clone(),
                    category: event.category.clone(),
                    server: event.server.clone(),
                    subject: event.subject.clone(),
                    detail: event.detail.clone(),
                    arguments_json: event.arguments_json.clone().or_else(|| {
                        (!event.arguments.is_empty())
                            .then(|| serde_json::to_string(&event.arguments).ok())
                            .flatten()
                    }),
                    result_json: event.result_json.clone().or_else(|| {
                        (!event.result_fields.is_empty())
                            .then(|| serde_json::to_string(&event.result_fields).ok())
                            .flatten()
                    }),
                    repeated: event.repeated,
                    failed: event.failed,
                    output_bytes: event.output_bytes,
                    exit_code: event.exit_code,
                })
                .collect();
            let phase_events = indexed
                .turns
                .values()
                .flat_map(|turn| turn.phase_events.iter())
                .map(|event| TraceMirrorPhaseEvent {
                    turn_id: event.turn_id.clone(),
                    source_order: event.source_order,
                    source_end_order: event.source_end_order,
                    timestamp_ms: event.timestamp_ms,
                    phase: event.phase.clone(),
                    source_type: event.source_type.clone(),
                    role: event.role.clone(),
                    content_bytes: event.content_bytes,
                    encrypted_bytes: event.encrypted_bytes,
                })
                .collect();
            let usage_events = indexed
                .usage_events
                .iter()
                .map(|event| TraceMirrorUsageEvent {
                    turn_id: event.turn_id.clone(),
                    source_order: event.source_order,
                    timestamp_ms: event.timestamp_ms,
                    model: event.model.clone(),
                    input_tokens: event.input_tokens,
                    cached_input_tokens: event.cached_input_tokens,
                    output_tokens: event.output_tokens,
                    reasoning_output_tokens: event.reasoning_output_tokens,
                    total_tokens: event.total_tokens,
                    context_window: event.context_window,
                })
                .collect();
            TraceMirrorSession {
                session_path: session_path.clone(),
                length: indexed.length,
                modified_ms: indexed.modified_ms,
                session_id: indexed.session_id.clone(),
                parent_id: indexed.parent_id.clone(),
                cwd: indexed.cwd.clone(),
                conversation_name: indexed.conversation_name.clone(),
                official_status: indexed.official_status.clone(),
                started_at: indexed.started_at.clone(),
                updated_at: indexed.updated_at.clone(),
                model: indexed.model.clone(),
                malformed_lines: indexed.malformed_lines,
                turns,
                tool_events,
                phase_events,
                usage_events,
            }
        })
        .collect()
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

fn discover_trace_files() -> Result<Vec<PathBuf>, String> {
    let mut selected = HashMap::<PathBuf, PathBuf>::new();
    for home in codex_homes()? {
        let active = home.join("sessions");
        let archived = home.join("archived_sessions");
        collect_logs(&archived, &archived, &mut selected)?;
        collect_logs(&active, &active, &mut selected)?;
    }
    let mut files = selected.into_values().collect::<Vec<_>>();
    files.sort_by_key(|path| {
        std::cmp::Reverse(
            fs::metadata(path)
                .ok()
                .map(|metadata| modified_millis(&metadata))
                .unwrap_or(0),
        )
    });
    files.truncate(MAX_TRACE_FILES);
    files.sort();
    Ok(files)
}

fn collect_logs(
    root: &Path,
    directory: &Path,
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
            collect_logs(root, &path, selected)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            selected.insert(relative, path);
        }
    }
    Ok(())
}

fn modified_millis(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_repeated_read_paths_without_command_content() {
        let input = Value::String(
            r#"{"cmd":"sed -n '1,20p' src/App.tsx && rg usage src/App.tsx","workdir":"/tmp/codex-xray"}"#
                .to_string(),
        );
        let paths = read_paths("exec_command", &input);
        assert!(paths.contains("src/App.tsx"));
    }

    #[test]
    fn describes_cli_with_exact_command_preview() {
        let input = Value::String(
            r#"{"cmd":"curl -H 'Authorization: Bearer abc123' https://example.com token=private","workdir":"/tmp/codex-xray"}"#
                .to_string(),
        );
        let presentation = describe_tool_call("exec_command", None, &input);
        assert_eq!(presentation.category, "cli");
        let detail = presentation.detail.expect("command detail");
        assert!(detail.contains("curl"));
        assert!(detail.contains("abc123"));
        assert!(detail.contains("private"));
    }

    #[test]
    fn classifies_mcp_server_and_keeps_argument_values() {
        let input = Value::String(
            r#"{"issue_key":"SPARK-123","api_key":"private","max_results":20,"max_output_tokens":10000}"#
                .to_string(),
        );
        let presentation = describe_tool_call(
            "mcp__aiops_pilot.query_jira_tickets",
            Some("mcp__aiops_pilot"),
            &input,
        );
        assert_eq!(presentation.category, "mcp");
        assert_eq!(presentation.server.as_deref(), Some("aiops_pilot"));
        assert!(
            presentation
                .arguments
                .iter()
                .any(|field| { field.key == "api_key" && field.value == "private" })
        );
        assert!(
            presentation
                .arguments
                .iter()
                .any(|field| { field.key == "max_output_tokens" && field.value == "10000" })
        );
    }

    #[test]
    fn recognizes_skill_load_from_skill_markdown_path() {
        let input = Value::String(
            r#"{"cmd":"sed -n '1,220p' /Users/test/.codex/skills/ui-ux-pro-max/SKILL.md"}"#
                .to_string(),
        );
        let presentation = describe_tool_call("exec_command", None, &input);
        assert_eq!(presentation.category, "skill");
        assert_eq!(presentation.detail.as_deref(), Some("ui-ux-pro-max"));
        assert_eq!(
            presentation.subject.as_deref(),
            Some("ui-ux-pro-max/SKILL.md")
        );
    }

    #[test]
    fn extracts_nested_tool_names_and_exit_codes() {
        assert_eq!(
            extract_nested_tools("await tools.exec_command({}); await tools.web__run({});"),
            vec!["exec_command".to_string(), "web__run".to_string()]
        );
        assert_eq!(
            extract_exit_code(&Value::String(
                "Chunk ID: test\nProcess exited with code 7".to_string()
            )),
            Some(7)
        );
    }

    #[test]
    fn builds_full_argument_and_result_details() {
        let input = Value::String(
            r#"{"query":{"filters":[{"field":"status","value":"open"}]},"auth":{"api_key":"private-key"},"limit":25}"#
                .to_string(),
        );
        let tree = trace_detail_json(&input).expect("argument tree");
        assert!(tree.contains("\"filters\""));
        assert!(tree.contains("\"status\""));
        assert!(tree.contains("private-key"));

        let result = Value::String(
            r#"{"exit_code":0,"wall_time_seconds":0.25,"status":"ok","output":"private tool output"}"#
                .to_string(),
        );
        let fields = extract_result_fields(&result);
        assert!(
            fields
                .iter()
                .any(|field| field.key == "exit_code" && field.value == "0")
        );
        assert!(
            fields
                .iter()
                .any(|field| field.key == "wall_time_seconds" && field.value == "0.25")
        );
        assert!(
            !serde_json::to_string(&fields)
                .expect("serialize fields")
                .contains("private tool output")
        );
        assert!(
            trace_detail_json(&result)
                .expect("result detail")
                .contains("private tool output")
        );
    }

    #[test]
    fn keeps_patch_body_in_local_trace_detail() {
        let input = Value::String(
            "*** Begin Patch\n*** Update File: src/App.tsx\n@@\n-secret source\n+new source\n*** End Patch\n"
                .to_string(),
        );
        let tree = trace_detail_json(&input).expect("argument tree");
        assert!(tree.contains("src/App.tsx"));
        assert!(tree.contains("secret source"));
        assert!(tree.contains("new source"));
    }

    #[test]
    fn builds_structured_trace_with_exact_local_details() {
        let directory = std::env::temp_dir().join(format!(
            "codex-xray-trace-test-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create temp directory");
        let path = directory.join("session.jsonl");
        let mut file = File::create(&path).expect("create fixture");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"session-1\",\"cwd\":\"/tmp/project\",\"source\":\"cli\"}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"turn-1\",\"started_at\":\"2026-07-26T00:00:01Z\"}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:02Z\",\"type\":\"turn_context\",\"payload\":{{\"turn_id\":\"turn-1\",\"model\":\"gpt-5.4\",\"effort\":\"xhigh\",\"summary\":\"auto\"}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:02.500Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"private user prompt\"}}]}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:03Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"model_context_window\":400000,\"last_token_usage\":{{\"input_tokens\":200000,\"cached_input_tokens\":10000,\"output_tokens\":1000,\"total_tokens\":201000}}}}}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:03.500Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"model_context_window\":400000,\"last_token_usage\":{{\"input_tokens\":230000,\"cached_input_tokens\":20000,\"output_tokens\":1200,\"total_tokens\":231200}}}}}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:03.700Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"reasoning\",\"summary\":[{{\"type\":\"summary_text\",\"text\":\"private reasoning summary\"}}],\"encrypted_content\":\"encrypted-private-reasoning\"}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:03.800Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"commentary\",\"content\":[{{\"type\":\"output_text\",\"text\":\"private assistant message\"}}]}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:04.100Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call\",\"name\":\"exec_command\",\"call_id\":\"call-1\",\"arguments\":\"{{\\\"cmd\\\":\\\"sed -n 1,20p src/App.tsx\\\",\\\"options\\\":{{\\\"api_key\\\":\\\"private-key\\\",\\\"paths\\\":[\\\"src/App.tsx\\\",\\\"src/types.ts\\\"]}}}}\"}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:04.200Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"call_id\":\"call-1\",\"output\":\"{{\\\"exit_code\\\":0,\\\"wall_time_seconds\\\":0.1,\\\"output\\\":\\\"private tool result\\\"}}\"}}}}"
        )
        .expect("write");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:05Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"turn-1\",\"duration_ms\":4000}}}}"
        )
        .expect("write");
        drop(file);

        let metadata = fs::metadata(&path).expect("metadata");
        let indexed =
            parse_trace_file(&path, metadata.len(), modified_millis(&metadata)).expect("parse");
        assert_eq!(indexed.session_id.as_deref(), Some("session-1"));
        assert_eq!(indexed.usage_events.len(), 2);
        assert_eq!(indexed.usage_events[0].total_tokens, 201_000);
        assert_eq!(indexed.turns["turn-1"].phase_events.len(), 3);
        let detail = summarize_session_detail(&indexed, 0, SystemTime::now());
        assert_eq!(detail.turns.len(), 1);
        assert_eq!(detail.turns[0].tool_calls, 1);
        assert_eq!(detail.turns[0].reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(detail.turns[0].summary_mode.as_deref(), Some("auto"));
        assert!(
            detail.turns[0]
                .timeline
                .windows(2)
                .all(|events| events[0].source_order < events[1].source_order)
        );
        assert_eq!(
            detail.turns[0]
                .timeline
                .iter()
                .map(|event| (event.kind.as_str(), event.label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("started", "turn"),
                ("phase", "user_prompt"),
                ("tokens", "gpt-5.4"),
                ("tokens", "gpt-5.4"),
                ("phase", "reasoning"),
                ("phase", "commentary"),
                ("tool_request", "exec_command"),
                ("tool_result", "exec_command"),
                ("completed", "turn"),
            ]
        );
        assert!(
            detail.turns[0]
                .timeline
                .iter()
                .any(|event| event.kind == "tokens" && event.category == "usage")
        );
        assert!(detail.turns[0].timeline.iter().any(|event| {
            event.kind == "tokens"
                && event.sequence == Some(2)
                && event.context_delta_tokens == Some(30_000)
                && event.cache_hit_percent.is_some()
        }));
        assert!(detail.turns[0].timeline.iter().any(|event| {
            event.kind == "phase"
                && event.label == "user_prompt"
                && event.content.as_deref() == Some("private user prompt")
        }));
        assert!(detail.turns[0].timeline.iter().any(|event| {
            event.kind == "phase"
                && event.label == "reasoning"
                && event.summary_parts == 1
                && event.encrypted_bytes > 0
                && event.content.as_deref() == Some("private reasoning summary")
        }));
        assert!(detail.turns[0].timeline.iter().any(|event| {
            event.kind == "tool_request"
                && event.label == "exec_command"
                && event.category == "cli"
                && event.call_id.as_deref() == Some("call-1")
                && event.source_order == 9
                && event.source_end_order.is_none()
                && event.source_type.as_deref() == Some("response_item.function_call")
                && event
                    .arguments_json
                    .as_deref()
                    .is_some_and(|value| value.contains("private-key"))
        }));
        assert!(detail.turns[0].timeline.iter().any(|event| {
            event.kind == "tool_result"
                && event.label == "exec_command"
                && event.exit_code == Some(0)
                && event.call_id.as_deref() == Some("call-1")
                && event.source_order == 10
                && event.source_type.as_deref() == Some("response_item.function_call_output")
                && event
                    .result_json
                    .as_deref()
                    .is_some_and(|value| value.contains("private tool result"))
                && event
                    .result_fields
                    .iter()
                    .any(|field| field.key == "wall_time_seconds")
        }));
        let serialized = serde_json::to_string(&indexed).expect("serialize");
        assert!(serialized.contains("private assistant message"));
        assert!(serialized.contains("private reasoning summary"));
        assert!(serialized.contains("private tool result"));
        assert!(serialized.contains("private-key"));

        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn reconstructs_context_compaction_and_explicit_memory_evidence() {
        let directory = std::env::temp_dir().join(format!(
            "codex-xray-context-trace-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create temp directory");
        let path = directory.join("session.jsonl");
        let mut file = File::create(&path).expect("create fixture");
        for line in [
            r#"{"timestamp":"2026-07-30T10:00:00.000Z","type":"session_meta","payload":{"id":"context-session","cwd":"/tmp/context-project","source":"app","base_instructions":{"text":"base"},"dynamic_tools":[{"name":"exec_command"},{"name":"web.run"}]}}"#,
            r#"{"timestamp":"2026-07-30T10:00:00.100Z","type":"event_msg","payload":{"type":"task_started","turn_id":"turn-context","started_at":"2026-07-30T10:00:00.100Z"}}"#,
            r#"{"timestamp":"2026-07-30T10:00:00.200Z","type":"turn_context","payload":{"turn_id":"turn-context","model":"gpt-5.6-sol","effort":"xhigh","summary":"auto"}}"#,
            r#"{"timestamp":"2026-07-30T10:00:00.300Z","type":"world_state","payload":{"cwd":"/tmp/context-project","permissions":"read-write"}}"#,
            r#"{"timestamp":"2026-07-30T10:00:00.400Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<memories>durable fact</memories>"}]}}"#,
            r#"{"timestamp":"2026-07-30T10:00:01.000Z","type":"event_msg","payload":{"type":"token_count","info":{"model":"gpt-5.6-sol","model_context_window":258400,"last_token_usage":{"input_tokens":200000,"cached_input_tokens":150000,"output_tokens":500,"total_tokens":200500}}}}"#,
            r#"{"timestamp":"2026-07-30T10:00:02.000Z","type":"compacted","payload":{"window_number":2,"replacement_history":[{"type":"compaction","encrypted_content":"ciphertext"},{"type":"message","role":"user","content":[{"type":"input_text","text":"retained"}]}]}}"#,
            r#"{"timestamp":"2026-07-30T10:00:02.010Z","type":"event_msg","payload":{"type":"context_compacted"}}"#,
            r#"{"timestamp":"2026-07-30T10:00:02.100Z","type":"event_msg","payload":{"type":"token_count","info":{"model":"gpt-5.6-sol","model_context_window":258400,"last_token_usage":{"input_tokens":100000,"cached_input_tokens":90000,"output_tokens":250,"total_tokens":100250}}}}"#,
            r#"{"timestamp":"2026-07-30T10:00:02.200Z","type":"event_msg","payload":{"type":"agent_message","phase":"final_answer","message":"done","memory_citation":{"thread_id":"source-thread"}}}"#,
            r#"{"timestamp":"2026-07-30T10:00:02.300Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-context","duration_ms":2200}}"#,
        ] {
            writeln!(file, "{line}").expect("write fixture");
        }
        drop(file);

        let metadata = fs::metadata(&path).expect("metadata");
        let indexed =
            parse_trace_file(&path, metadata.len(), modified_millis(&metadata)).expect("parse");
        let detail = summarize_session_detail(&indexed, 0, SystemTime::now());
        let turn = detail.turns.first().expect("turn summary");
        assert_eq!(turn.first_input_tokens, 200_000);
        assert_eq!(turn.peak_input_tokens, 200_000);
        assert_eq!(turn.last_input_tokens, 100_000);
        assert_eq!(turn.model_passes, 2);
        assert_eq!(turn.context_compactions, 1);
        assert_eq!(turn.estimated_reclaimed_tokens, 100_000);
        assert!(turn.local_context_bytes > 0);
        assert!(turn.session_context_bytes > 0);
        assert!(turn.developer_context_bytes > 0);
        assert!(turn.world_state_bytes > 0);
        assert!(turn.turn_context_bytes > 0);
        assert!(turn.memory_context_bytes > 0);
        assert_eq!(turn.memory_citations, 1);

        let compaction = turn
            .timeline
            .iter()
            .find(|event| event.kind == "compaction")
            .expect("compaction event");
        assert_eq!(compaction.source_order, 7);
        assert_eq!(compaction.source_end_order, Some(8));
        assert_eq!(compaction.sequence, Some(2));
        assert_eq!(compaction.content_parts, 2);
        assert_eq!(compaction.encrypted_bytes, 10);
        assert_eq!(compaction.context_before_tokens, Some(200_000));
        assert_eq!(compaction.context_after_tokens, Some(100_000));
        assert_eq!(compaction.context_reclaimed_tokens, Some(100_000));
        assert_eq!(detail.model_passes, 2);
        assert_eq!(detail.estimated_reclaimed_tokens, 100_000);
        assert!(detail.memory_context_bytes > 0);
        assert_eq!(detail.memory_citations, 1);

        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn parses_compaction_lines_larger_than_the_general_json_limit() {
        let directory = std::env::temp_dir().join(format!(
            "codex-xray-large-compaction-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create temp directory");
        let path = directory.join("session.jsonl");
        let mut file = File::create(&path).expect("create fixture");
        for line in [
            r#"{"timestamp":"2026-07-30T10:00:00.000Z","type":"session_meta","payload":{"id":"large-compaction","cwd":"/tmp/project"}}"#,
            r#"{"timestamp":"2026-07-30T10:00:00.100Z","type":"event_msg","payload":{"type":"task_started","turn_id":"turn-large"}}"#,
            r#"{"timestamp":"2026-07-30T10:00:00.200Z","type":"turn_context","payload":{"turn_id":"turn-large","model":"gpt-5.6-sol"}}"#,
            r#"{"timestamp":"2026-07-30T10:00:01.000Z","type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":258400,"last_token_usage":{"input_tokens":220000,"output_tokens":100,"total_tokens":220100}}}}"#,
        ] {
            writeln!(file, "{line}").expect("write fixture");
        }
        let oversized_history = "x".repeat(MAX_PARSED_LINE_BYTES + 1024);
        let compacted = serde_json::json!({
            "timestamp": "2026-07-30T10:00:02.000Z",
            "type": "compacted",
            "payload": {
                "window_number": 9,
                "replacement_history": [
                    {"type": "compaction", "encrypted_content": "encrypted-summary"},
                    {"type": "message", "content": [{"type": "input_text", "text": oversized_history}]}
                ]
            }
        });
        let compacted_line = serde_json::to_string(&compacted).expect("serialize compaction");
        assert!(compacted_line.len() > MAX_PARSED_LINE_BYTES);
        writeln!(file, "{compacted_line}").expect("write large compaction");
        for line in [
            r#"{"timestamp":"2026-07-30T10:00:02.010Z","type":"event_msg","payload":{"type":"context_compacted"}}"#,
            r#"{"timestamp":"2026-07-30T10:00:02.100Z","type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":258400,"last_token_usage":{"input_tokens":90000,"output_tokens":100,"total_tokens":90100}}}}"#,
            r#"{"timestamp":"2026-07-30T10:00:02.200Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-large"}}"#,
        ] {
            writeln!(file, "{line}").expect("write fixture");
        }
        drop(file);

        let metadata = fs::metadata(&path).expect("metadata");
        let indexed =
            parse_trace_file(&path, metadata.len(), modified_millis(&metadata)).expect("parse");
        let detail = summarize_session_detail(&indexed, 0, SystemTime::now());
        let turn = detail.turns.first().expect("turn summary");
        assert_eq!(turn.context_compactions, 1);
        assert_eq!(turn.estimated_reclaimed_tokens, 130_000);
        let compaction = turn
            .timeline
            .iter()
            .find(|event| event.kind == "compaction")
            .expect("compaction event");
        assert_eq!(compaction.sequence, Some(9));
        assert_eq!(compaction.content_parts, 2);
        assert_eq!(compaction.encrypted_bytes, 17);
        assert_eq!(compaction.source_end_order, Some(6));

        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn tracks_web_request_execution_and_output_boundaries() {
        let directory = std::env::temp_dir().join(format!(
            "codex-xray-web-trace-test-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create temp directory");
        let path = directory.join("session.jsonl");
        let mut file = File::create(&path).expect("create fixture");
        for line in [
            r#"{"timestamp":"2026-07-28T10:59:46.000Z","type":"session_meta","payload":{"id":"web-session","cwd":"/tmp/project","source":"app"}}"#,
            r#"{"timestamp":"2026-07-28T10:59:46.100Z","type":"event_msg","payload":{"type":"task_started","turn_id":"turn-web","started_at":"2026-07-28T10:59:46.100Z"}}"#,
            r#"{"timestamp":"2026-07-28T10:59:46.200Z","type":"turn_context","payload":{"turn_id":"turn-web","model":"gpt-5.6-sol"}}"#,
            r#"{"timestamp":"2026-07-28T10:59:46.300Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"合肥天气"}]}}"#,
            r#"{"timestamp":"2026-07-28T10:59:46.301Z","type":"event_msg","payload":{"type":"user_message","message":"合肥天气"}}"#,
            r#"{"timestamp":"2026-07-28T10:59:46.500Z","type":"event_msg","payload":{"type":"agent_message","phase":"commentary","message":"我查一下天气"}}"#,
            r#"{"timestamp":"2026-07-28T10:59:46.500Z","type":"response_item","payload":{"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"我查一下天气"}]}}"#,
            r#"{"timestamp":"2026-07-28T10:59:47.194Z","type":"response_item","payload":{"type":"function_call","namespace":"web","name":"run","call_id":"call-web","arguments":"{\"weather\":[{\"location\":\"合肥\",\"duration\":1}],\"response_length\":\"short\"}"}}"#,
            r#"{"timestamp":"2026-07-28T10:59:50.691Z","type":"event_msg","payload":{"type":"web_search_end","call_id":"call-web","query":"","action":{"type":"other"},"results":[]}}"#,
            r#"{"timestamp":"2026-07-28T10:59:50.700Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call-web","output":"[{\"type\":\"input_text\",\"text\":\"Weather result\"}]"}}"#,
            r#"{"timestamp":"2026-07-28T10:59:50.700Z","type":"event_msg","payload":{"type":"token_count","info":{"model":"gpt-5.6-sol","model_context_window":258400,"last_token_usage":{"input_tokens":20854,"cached_input_tokens":0,"output_tokens":70,"reasoning_output_tokens":0,"total_tokens":20924}}}}"#,
            r#"{"timestamp":"2026-07-28T10:59:55.179Z","type":"response_item","payload":{"type":"reasoning","encrypted_content":"encrypted"}}"#,
            r#"{"timestamp":"2026-07-28T10:59:56.415Z","type":"event_msg","payload":{"type":"agent_message","phase":"final_answer","message":"天气结果"}}"#,
            r#"{"timestamp":"2026-07-28T10:59:56.419Z","type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"天气结果"}]}}"#,
            r#"{"timestamp":"2026-07-28T10:59:56.517Z","type":"event_msg","payload":{"type":"token_count","info":{"model":"gpt-5.6-sol","model_context_window":258400,"last_token_usage":{"input_tokens":21333,"cached_input_tokens":20224,"output_tokens":147,"reasoning_output_tokens":74,"total_tokens":21480}}}}"#,
            r#"{"timestamp":"2026-07-28T10:59:56.521Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-web","duration_ms":10421}}"#,
        ] {
            writeln!(file, "{line}").expect("write fixture");
        }
        drop(file);

        let metadata = fs::metadata(&path).expect("metadata");
        let indexed =
            parse_trace_file(&path, metadata.len(), modified_millis(&metadata)).expect("parse");
        let detail = summarize_session_detail(&indexed, 0, SystemTime::now());
        let timeline = &detail.turns[0].timeline;
        assert_eq!(
            timeline
                .iter()
                .map(|event| {
                    (
                        event.kind.as_str(),
                        event.source_order,
                        event.source_end_order,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("started", 1, Some(3)),
                ("phase", 4, Some(5)),
                ("phase", 6, Some(7)),
                ("tool_request", 8, None),
                ("tool_execution", 9, None),
                ("tool_result", 10, None),
                ("tokens", 11, None),
                ("phase", 12, None),
                ("phase", 13, Some(14)),
                ("tokens", 15, None),
                ("completed", 16, None),
            ]
        );

        let request = timeline
            .iter()
            .find(|event| event.kind == "tool_request")
            .expect("web request");
        assert_eq!(request.label, "web.run");
        assert_eq!(request.category, "browser");
        assert_eq!(request.call_id.as_deref(), Some("call-web"));
        assert_eq!(
            request.source_type.as_deref(),
            Some("response_item.function_call")
        );

        let execution = timeline
            .iter()
            .find(|event| event.kind == "tool_execution")
            .expect("web execution end");
        assert_eq!(
            execution.source_type.as_deref(),
            Some("event_msg.web_search_end")
        );
        assert_eq!(
            execution.timestamp.as_deref(),
            Some("2026-07-28T10:59:50.691+00:00")
        );
        assert_eq!(execution.duration_ms, Some(3_497));

        let result = timeline
            .iter()
            .find(|event| event.kind == "tool_result")
            .expect("web output");
        assert_eq!(
            result.source_type.as_deref(),
            Some("response_item.function_call_output")
        );
        assert_eq!(
            result.timestamp.as_deref(),
            Some("2026-07-28T10:59:50.700+00:00")
        );
        assert_eq!(result.duration_ms, Some(9));

        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn merges_official_thread_name_directory_and_status() {
        let mut index = TraceIndex::default();
        index.files.insert(
            "session.jsonl".to_string(),
            IndexedTraceFile {
                session_id: Some("thread-1".to_string()),
                cwd: Some("/tmp/old".to_string()),
                ..IndexedTraceFile::default()
            },
        );
        let metadata = [ThreadMetadata {
            id: "thread-1".to_string(),
            name: Some("Improve trace UX".to_string()),
            cwd: "/tmp/project".to_string(),
            status: Some("waiting_approval".to_string()),
            path: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_100,
            parent_thread_id: None,
        }];

        assert_eq!(merge_thread_metadata(&mut index, Some(&metadata)), 1);
        let record = index.files.get("session.jsonl").expect("record");
        assert_eq!(
            record.conversation_name.as_deref(),
            Some("Improve trace UX")
        );
        assert_eq!(record.cwd.as_deref(), Some("/tmp/project"));
        assert_eq!(record.official_status.as_deref(), Some("waiting_approval"));
    }

    #[test]
    fn catalog_lists_official_threads_without_parsing_sessions() {
        let directory = std::env::temp_dir().join(format!(
            "codex-xray-catalog-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create temp directory");
        let metadata = [ThreadMetadata {
            id: "thread-catalog-1".to_string(),
            name: Some("Catalog only".to_string()),
            cwd: "/tmp/example-project".to_string(),
            status: Some("completed".to_string()),
            path: Some("/tmp/missing-session.jsonl".to_string()),
            created_at: 1_700_000_000,
            updated_at: 1_700_000_100,
            parent_thread_id: None,
        }];

        let snapshot =
            build_trace_catalog(&directory.join("trace-index.json"), &metadata).expect("catalog");
        assert_eq!(snapshot.files_scanned, 0);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.sessions[0].project, "example-project");
        assert_eq!(snapshot.sessions[0].analysis_state, "not_analyzed");
        assert_eq!(snapshot.totals.sessions, 0);

        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn aggregates_persisted_extension_calls_without_token_attribution() {
        let directory = std::env::temp_dir().join(format!(
            "codex-xray-extension-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create temp directory");
        let session_path = directory.join("session-extension-1.jsonl");
        fs::write(&session_path, "{}\n").expect("write session fixture");
        let metadata = fs::metadata(&session_path).expect("metadata");

        let turn = TraceTurnRecord {
            id: "turn-1".to_string(),
            tool_events: vec![
                TraceToolEvent {
                    timestamp_ms: 1_700_000_000_000,
                    completed_at_ms: Some(1_700_000_000_250),
                    turn_id: "turn-1".to_string(),
                    name: "mcp__demo.search".to_string(),
                    category: "mcp".to_string(),
                    server: Some("demo".to_string()),
                    failed: true,
                    repeated: true,
                    output_bytes: 400,
                    ..TraceToolEvent::default()
                },
                TraceToolEvent {
                    timestamp_ms: 1_700_000_001_000,
                    completed_at_ms: None,
                    turn_id: "turn-1".to_string(),
                    name: "exec_command".to_string(),
                    category: "skill".to_string(),
                    detail: Some("ui-review".to_string()),
                    output_bytes: 50,
                    ..TraceToolEvent::default()
                },
            ],
            ..TraceTurnRecord::default()
        };
        let mut turns = BTreeMap::new();
        turns.insert(turn.id.clone(), turn);
        let record = IndexedTraceFile {
            length: metadata.len(),
            modified_ms: modified_millis(&metadata),
            session_id: Some("session-extension-1".to_string()),
            cwd: Some("/tmp/example-project".to_string()),
            turns,
            ..IndexedTraceFile::default()
        };
        let mut index = TraceIndex::default();
        index
            .files
            .insert(session_path.to_string_lossy().to_string(), record);
        let mut cache = TraceIndexCache { index: Some(index) };

        let snapshot =
            build_extension_usage_cached(&directory.join("trace-index.json"), &mut cache)
                .expect("extension usage");
        assert_eq!(snapshot.analyzed_sessions, 1);
        assert_eq!(snapshot.current_sessions, 1);
        assert_eq!(snapshot.projects, 1);
        assert_eq!(snapshot.calls, 2);
        assert_eq!(snapshot.failures, 1);
        assert_eq!(snapshot.repeated_calls, 1);
        assert_eq!(snapshot.timed_calls, 1);
        assert_eq!(snapshot.duration_ms, 250);
        assert_eq!(snapshot.output_bytes, 450);
        assert!(snapshot.items.iter().any(|item| item.category == "mcp"
            && item.server.as_deref() == Some("demo")
            && item.calls == 1
            && item.occurrences.first().is_some_and(|occurrence| {
                occurrence.session_id == "session-extension-1"
                    && occurrence.turn_id == "turn-1"
                    && occurrence.failed
            })));
        assert!(
            snapshot
                .items
                .iter()
                .any(|item| item.category == "skill" && item.name == "ui-review")
        );

        fs::remove_dir_all(directory).expect("cleanup");
    }
}
