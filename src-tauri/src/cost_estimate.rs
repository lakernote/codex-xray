use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use chrono::{DateTime, Local, Utc};
use memchr::memmem;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codex::ThreadMetadata;
use crate::pricing::{EstimatedCost, estimate_standard_api_cost_at, pricing_metadata};
use crate::storage::{
    UsageIndexEvent, UsageIndexFile, UsageIndexTurn, read_usage_index, write_usage_index,
};

const INDEX_SCHEMA_VERSION: u32 = 4;
const REPORT_SCHEMA_VERSION: u32 = 2;
const PROJECT_REPORT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimateSnapshot {
    pub report_schema_version: u32,
    pub generated_at: String,
    pub pricing_basis: String,
    pub pricing_updated_at: String,
    pub total_cost_usd: f64,
    pub uncached_input_cost_usd: f64,
    pub cached_input_cost_usd: f64,
    pub output_cost_usd: f64,
    pub cache_savings_usd: f64,
    pub total_tokens: u64,
    pub priced_tokens: u64,
    pub unpriced_tokens: u64,
    pub coverage_start: Option<String>,
    pub coverage_end: Option<String>,
    pub files_indexed: usize,
    pub files_scanned: usize,
    pub files_reused: usize,
    pub token_events: usize,
    pub duplicate_events_skipped: usize,
    pub elapsed_ms: u128,
    pub daily: Vec<DailyCostEstimate>,
    pub models: Vec<ModelCostEstimate>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyCostEstimate {
    pub date: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub priced_tokens: u64,
    pub unpriced_tokens: u64,
    pub cost_usd: f64,
    pub models: Vec<ModelCostEstimate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCostEstimate {
    pub model: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub cache_savings_usd: f64,
    pub priced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectUsageSnapshot {
    pub report_schema_version: u32,
    pub generated_at: String,
    pub pricing_updated_at: String,
    pub files_indexed: usize,
    pub sessions: usize,
    pub turns: usize,
    pub turn_sessions_indexed: usize,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub priced_tokens: u64,
    pub unpriced_tokens: u64,
    pub cost_usd: f64,
    pub projects: Vec<ProjectUsageProject>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectUsageProject {
    pub name: String,
    pub path: String,
    pub updated_at: Option<String>,
    pub sessions: usize,
    pub turns: usize,
    pub turn_sessions_indexed: usize,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub priced_tokens: u64,
    pub unpriced_tokens: u64,
    pub cost_usd: f64,
    pub conversations: Vec<ProjectUsageConversation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectUsageConversation {
    pub id: String,
    pub title: Option<String>,
    pub session_path: String,
    pub updated_at: Option<String>,
    pub is_subagent: bool,
    pub parent_id: Option<String>,
    pub models: Vec<String>,
    pub turns: usize,
    pub turns_indexed: bool,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub priced_tokens: u64,
    pub unpriced_tokens: u64,
    pub cost_usd: f64,
    pub turn_rows: Vec<ProjectUsageTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectUsageTurn {
    pub id: String,
    pub sequence: usize,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    pub models: Vec<String>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub priced_tokens: u64,
    pub unpriced_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTurnUsageDetail {
    pub session_id: String,
    pub generated_at: String,
    pub turns: Vec<ProjectUsageTurn>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CostIndex {
    schema_version: u32,
    files: BTreeMap<String, IndexedFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct IndexedFile {
    length: u64,
    modified_ms: u128,
    malformed_lines: usize,
    session_id: Option<String>,
    parent_id: Option<String>,
    forked_at_ms: Option<i64>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    started_at_ms: Option<i64>,
    #[serde(default)]
    updated_at_ms: Option<i64>,
    #[serde(default)]
    turns: BTreeMap<String, IndexedTurn>,
    #[serde(default)]
    project_metadata_indexed: bool,
    events: Vec<CostUsageEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct IndexedTurn {
    started_at_ms: Option<i64>,
    updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CostUsageEvent {
    timestamp_ms: i64,
    date: String,
    #[serde(default = "default_turn_id")]
    turn_id: String,
    model: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
}

fn default_turn_id() -> String {
    "unassigned".to_string()
}

#[derive(Debug, Clone, Default)]
struct RawUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
}

impl RawUsage {
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
            total_tokens: difference(self.total_tokens, previous.total_tokens),
        }
    }
}

#[derive(Default)]
struct Aggregate {
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    priced_tokens: u64,
    unpriced_tokens: u64,
    cost: EstimatedCost,
}

impl Aggregate {
    fn add_event(&mut self, event: &CostUsageEvent, cost: Option<EstimatedCost>) {
        self.input_tokens = self.input_tokens.saturating_add(event.input_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(event.cached_input_tokens);
        self.cache_write_input_tokens = self
            .cache_write_input_tokens
            .saturating_add(event.cache_write_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(event.output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(event.total_tokens);
        if let Some(cost) = cost {
            self.priced_tokens = self.priced_tokens.saturating_add(event.total_tokens);
            self.cost.uncached_input_usd += cost.uncached_input_usd;
            self.cost.cached_input_usd += cost.cached_input_usd;
            self.cost.output_usd += cost.output_usd;
            self.cost.total_usd += cost.total_usd;
            self.cost.cache_savings_usd += cost.cache_savings_usd;
        } else {
            self.unpriced_tokens = self.unpriced_tokens.saturating_add(event.total_tokens);
        }
    }

    fn add_aggregate(&mut self, other: &Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(other.cached_input_tokens);
        self.cache_write_input_tokens = self
            .cache_write_input_tokens
            .saturating_add(other.cache_write_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
        self.priced_tokens = self.priced_tokens.saturating_add(other.priced_tokens);
        self.unpriced_tokens = self.unpriced_tokens.saturating_add(other.unpriced_tokens);
        self.cost.uncached_input_usd += other.cost.uncached_input_usd;
        self.cost.cached_input_usd += other.cost.cached_input_usd;
        self.cost.output_usd += other.cost.output_usd;
        self.cost.total_usd += other.cost.total_usd;
        self.cost.cache_savings_usd += other.cost.cache_savings_usd;
    }
}

#[derive(Default)]
struct TurnAggregate {
    usage: Aggregate,
    models: BTreeSet<String>,
    started_at_ms: Option<i64>,
    updated_at_ms: Option<i64>,
}

#[derive(Default)]
struct ConversationAggregate {
    id: String,
    title: Option<String>,
    session_path: String,
    updated_at_ms: Option<i64>,
    is_subagent: bool,
    parent_id: Option<String>,
    usage: Aggregate,
    models: BTreeSet<String>,
    turns: BTreeMap<String, TurnAggregate>,
    turn_rows_override: Option<Vec<ProjectUsageTurn>>,
    turns_indexed: bool,
}

#[derive(Default)]
struct ProjectAggregate {
    name: String,
    path: String,
    updated_at_ms: Option<i64>,
    usage: Aggregate,
    conversations: Vec<ConversationAggregate>,
}

pub fn build_cost_estimate(index_path: &Path) -> Result<CostEstimateSnapshot, String> {
    let started = Instant::now();
    let files = discover_usage_files()?;
    let mut warnings = Vec::new();
    let previous_index = load_index(index_path, &mut warnings);
    let mut next_index = CostIndex {
        schema_version: INDEX_SCHEMA_VERSION,
        files: BTreeMap::new(),
    };
    let mut files_scanned = 0;
    let mut files_reused = 0;
    let mut malformed_lines = 0;

    for path in &files {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("无法读取 {} 的文件信息：{error}", path.display()))?;
        let length = metadata.len();
        let modified_ms = modified_millis(&metadata);
        let key = path.to_string_lossy().into_owned();

        let indexed = match previous_index.files.get(&key) {
            Some(previous) if previous.length == length && previous.modified_ms == modified_ms => {
                files_reused += 1;
                previous.clone()
            }
            _ => {
                files_scanned += 1;
                parse_cost_file(path, length, modified_ms)?
            }
        };
        malformed_lines += indexed.malformed_lines;
        next_index.files.insert(key, indexed);
    }

    let mut total = Aggregate::default();
    let mut daily = BTreeMap::<String, Aggregate>::new();
    let mut daily_models = BTreeMap::<String, BTreeMap<String, Aggregate>>::new();
    let mut models = BTreeMap::<String, Aggregate>::new();
    let mut seen = HashSet::new();
    let mut duplicate_events_skipped = 0;
    let mut token_events = 0;

    let files_by_session_id = next_index
        .files
        .values()
        .filter_map(|indexed| {
            indexed
                .session_id
                .as_deref()
                .map(|session_id| (session_id, indexed))
        })
        .collect::<HashMap<_, _>>();
    let mut replayed_events_skipped = 0;

    for indexed in next_index.files.values() {
        let skip_replayed = replayed_prefix_len(indexed, &files_by_session_id);
        replayed_events_skipped += skip_replayed;
        for event in indexed.events.iter().skip(skip_replayed) {
            let key = (
                event.timestamp_ms,
                event.model.clone(),
                event.input_tokens,
                event.cached_input_tokens,
                event.cache_write_input_tokens,
                event.output_tokens,
                event.total_tokens,
            );
            if !seen.insert(key) {
                duplicate_events_skipped += 1;
                continue;
            }

            token_events += 1;
            let cost = estimate_standard_api_cost_at(
                &event.model,
                event.input_tokens,
                event.cached_input_tokens,
                event.output_tokens,
                event.timestamp_ms,
            );
            total.add_event(event, cost);
            daily
                .entry(event.date.clone())
                .or_default()
                .add_event(event, cost);
            daily_models
                .entry(event.date.clone())
                .or_default()
                .entry(event.model.clone())
                .or_default()
                .add_event(event, cost);
            models
                .entry(event.model.clone())
                .or_default()
                .add_event(event, cost);
        }
    }

    if malformed_lines > 0 {
        warnings.push(format!(
            "{malformed_lines} 行本地日志无法解析，已跳过；不会影响 Codex 本身"
        ));
    }
    let unpriced_models = models
        .iter()
        .filter_map(|(model, aggregate)| (aggregate.unpriced_tokens > 0).then_some(model.clone()))
        .collect::<BTreeSet<_>>();
    if !unpriced_models.is_empty() {
        warnings.push(format!(
            "{} 暂无公开 API 单价，相关 Token 未计入金额",
            unpriced_models.into_iter().collect::<Vec<_>>().join("、")
        ));
    }

    if let Err(error) = write_index(index_path, &next_index) {
        warnings.push(format!("成本增量索引未保存：{error}"));
    }

    let daily_rows = daily
        .into_iter()
        .map(|(date, aggregate)| DailyCostEstimate {
            models: model_rows(daily_models.remove(&date).unwrap_or_default().into_iter()),
            date,
            input_tokens: aggregate.input_tokens,
            cached_input_tokens: aggregate.cached_input_tokens,
            cache_write_input_tokens: aggregate.cache_write_input_tokens,
            output_tokens: aggregate.output_tokens,
            total_tokens: aggregate.total_tokens,
            priced_tokens: aggregate.priced_tokens,
            unpriced_tokens: aggregate.unpriced_tokens,
            cost_usd: aggregate.cost.total_usd,
        })
        .collect::<Vec<_>>();
    let coverage_start = daily_rows.first().map(|row| row.date.clone());
    let coverage_end = daily_rows.last().map(|row| row.date.clone());
    let model_rows = model_rows(models.into_iter());
    let (pricing_basis, pricing_updated_at) = pricing_metadata();

    Ok(CostEstimateSnapshot {
        report_schema_version: REPORT_SCHEMA_VERSION,
        generated_at: Utc::now().to_rfc3339(),
        pricing_basis,
        pricing_updated_at,
        total_cost_usd: total.cost.total_usd,
        uncached_input_cost_usd: total.cost.uncached_input_usd,
        cached_input_cost_usd: total.cost.cached_input_usd,
        output_cost_usd: total.cost.output_usd,
        cache_savings_usd: total.cost.cache_savings_usd,
        total_tokens: total.total_tokens,
        priced_tokens: total.priced_tokens,
        unpriced_tokens: total.unpriced_tokens,
        coverage_start,
        coverage_end,
        files_indexed: next_index.files.len(),
        files_scanned,
        files_reused,
        token_events,
        duplicate_events_skipped: duplicate_events_skipped + replayed_events_skipped,
        elapsed_ms: started.elapsed().as_millis(),
        daily: daily_rows,
        models: model_rows,
        warnings,
    })
}

pub fn build_project_usage(
    index_path: &Path,
    thread_metadata: Option<&[ThreadMetadata]>,
) -> Result<ProjectUsageSnapshot, String> {
    let index = read_cost_index(index_path)?;

    let files_by_session_id = index
        .files
        .values()
        .filter_map(|indexed| {
            indexed
                .session_id
                .as_deref()
                .map(|session_id| (session_id, indexed))
        })
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut projects = BTreeMap::<String, ProjectAggregate>::new();
    let mut canonical_projects = HashMap::<String, String>::new();
    let mut total = Aggregate::default();
    let warnings = Vec::new();
    let metadata_by_id = thread_metadata
        .unwrap_or_default()
        .iter()
        .map(|thread| (thread.id.as_str(), thread))
        .collect::<HashMap<_, _>>();

    for (session_path, indexed) in &index.files {
        let skip_replayed = replayed_prefix_len(indexed, &files_by_session_id);
        let session_id = indexed
            .session_id
            .clone()
            .unwrap_or_else(|| session_path.clone());
        let thread = metadata_by_id.get(session_id.as_str()).copied();
        let raw_project_path = thread
            .map(|thread| thread.cwd.clone())
            .filter(|path| !path.is_empty())
            .or_else(|| indexed.cwd.clone())
            .unwrap_or_default();
        let project_path = canonical_projects
            .entry(raw_project_path.clone())
            .or_insert_with(|| canonical_project_path(&raw_project_path))
            .clone();
        let project_key = if project_path.is_empty() {
            "__unassigned__".to_string()
        } else {
            project_path.clone()
        };
        let mut conversation = ConversationAggregate {
            id: session_id.clone(),
            title: thread.and_then(|thread| thread.name.clone()),
            session_path: thread
                .and_then(|thread| thread.path.clone())
                .unwrap_or_else(|| session_path.clone()),
            updated_at_ms: latest_timestamp(
                indexed.updated_at_ms,
                thread.map(|thread| thread.updated_at.saturating_mul(1_000)),
            ),
            is_subagent: thread
                .and_then(|thread| thread.parent_thread_id.as_ref())
                .or(indexed.parent_id.as_ref())
                .is_some(),
            parent_id: thread
                .and_then(|thread| thread.parent_thread_id.clone())
                .or_else(|| indexed.parent_id.clone()),
            turn_rows_override: None,
            turns_indexed: indexed.project_metadata_indexed,
            ..ConversationAggregate::default()
        };

        for event in indexed.events.iter().skip(skip_replayed) {
            let key = (
                event.timestamp_ms,
                event.model.clone(),
                event.input_tokens,
                event.cached_input_tokens,
                event.cache_write_input_tokens,
                event.output_tokens,
                event.total_tokens,
            );
            if !seen.insert(key) {
                continue;
            }

            let cost = estimate_standard_api_cost_at(
                &event.model,
                event.input_tokens,
                event.cached_input_tokens,
                event.output_tokens,
                event.timestamp_ms,
            );
            total.add_event(event, cost);
            conversation.usage.add_event(event, cost);
            conversation.models.insert(event.model.clone());
            conversation.updated_at_ms =
                latest_timestamp(conversation.updated_at_ms, Some(event.timestamp_ms));

            let turn = conversation
                .turns
                .entry(event.turn_id.clone())
                .or_insert_with(|| {
                    let metadata = indexed.turns.get(&event.turn_id);
                    TurnAggregate {
                        started_at_ms: metadata
                            .and_then(|turn| turn.started_at_ms)
                            .or(Some(event.timestamp_ms)),
                        updated_at_ms: metadata
                            .and_then(|turn| turn.updated_at_ms)
                            .or(Some(event.timestamp_ms)),
                        ..TurnAggregate::default()
                    }
                });
            turn.usage.add_event(event, cost);
            turn.models.insert(event.model.clone());
            turn.updated_at_ms = latest_timestamp(turn.updated_at_ms, Some(event.timestamp_ms));
        }

        if conversation.usage.total_tokens == 0 {
            continue;
        }
        let project = projects
            .entry(project_key)
            .or_insert_with(|| ProjectAggregate {
                name: project_name(&project_path),
                path: project_path,
                ..ProjectAggregate::default()
            });
        project.usage.add_aggregate(&conversation.usage);
        project.updated_at_ms = latest_timestamp(project.updated_at_ms, conversation.updated_at_ms);
        project.conversations.push(conversation);
    }

    let mut project_rows = projects
        .into_values()
        .map(project_usage_row)
        .collect::<Vec<_>>();
    project_rows.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.total_tokens.cmp(&left.total_tokens))
            .then_with(|| left.name.cmp(&right.name))
    });

    let session_count = project_rows.iter().map(|project| project.sessions).sum();
    let turn_count = project_rows.iter().map(|project| project.turns).sum();
    let turn_sessions_indexed = project_rows
        .iter()
        .map(|project| project.turn_sessions_indexed)
        .sum();
    let (_, pricing_updated_at) = pricing_metadata();
    Ok(ProjectUsageSnapshot {
        report_schema_version: PROJECT_REPORT_SCHEMA_VERSION,
        generated_at: Utc::now().to_rfc3339(),
        pricing_updated_at,
        files_indexed: index.files.len(),
        sessions: session_count,
        turns: turn_count,
        turn_sessions_indexed,
        input_tokens: total.input_tokens,
        cached_input_tokens: total.cached_input_tokens,
        cache_write_input_tokens: total.cache_write_input_tokens,
        output_tokens: total.output_tokens,
        total_tokens: total.total_tokens,
        priced_tokens: total.priced_tokens,
        unpriced_tokens: total.unpriced_tokens,
        cost_usd: total.cost.total_usd,
        projects: project_rows,
        warnings,
    })
}

pub fn build_project_turn_usage(
    cost_index_path: &Path,
    session_id: &str,
) -> Result<ProjectTurnUsageDetail, String> {
    if session_id.len() < 8
        || !session_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err("Session ID 不合法".to_string());
    }
    let index = read_cost_index(cost_index_path)?;
    let (session_path, indexed) = index
        .files
        .iter()
        .find(|(_, indexed)| indexed.session_id.as_deref() == Some(session_id))
        .ok_or_else(|| format!("没有找到 {session_id} 对应的本地 Session"))?;
    let files_by_session_id = index
        .files
        .values()
        .filter_map(|file| {
            file.session_id
                .as_deref()
                .map(|file_session_id| (file_session_id, file))
        })
        .collect::<HashMap<_, _>>();
    let skip_replayed = replayed_prefix_len(indexed, &files_by_session_id);
    let mut turns = BTreeMap::<String, TurnAggregate>::new();
    for event in indexed.events.iter().skip(skip_replayed) {
        let cost = estimate_standard_api_cost_at(
            &event.model,
            event.input_tokens,
            event.cached_input_tokens,
            event.output_tokens,
            event.timestamp_ms,
        );
        let turn = turns.entry(event.turn_id.clone()).or_insert_with(|| {
            let metadata = indexed.turns.get(&event.turn_id);
            TurnAggregate {
                started_at_ms: metadata
                    .and_then(|turn| turn.started_at_ms)
                    .or(Some(event.timestamp_ms)),
                updated_at_ms: metadata
                    .and_then(|turn| turn.updated_at_ms)
                    .or(Some(event.timestamp_ms)),
                ..TurnAggregate::default()
            }
        });
        turn.usage.add_event(event, cost);
        turn.models.insert(event.model.clone());
        turn.updated_at_ms = latest_timestamp(turn.updated_at_ms, Some(event.timestamp_ms));
    }
    let turn_rows = turn_usage_rows(turns);
    let _ = session_path;
    Ok(ProjectTurnUsageDetail {
        session_id: session_id.to_string(),
        generated_at: Utc::now().to_rfc3339(),
        turns: turn_rows,
    })
}

fn canonical_project_path(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    fs::canonicalize(path)
        .map(|resolved| resolved.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

fn project_usage_row(mut project: ProjectAggregate) -> ProjectUsageProject {
    project.conversations.sort_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| right.usage.total_tokens.cmp(&left.usage.total_tokens))
    });
    let conversations = project
        .conversations
        .into_iter()
        .map(conversation_usage_row)
        .collect::<Vec<_>>();
    let turns = conversations
        .iter()
        .map(|conversation| conversation.turns)
        .sum();
    let turn_sessions_indexed = conversations
        .iter()
        .filter(|conversation| conversation.turns_indexed)
        .count();
    ProjectUsageProject {
        name: project.name,
        path: project.path,
        updated_at: timestamp_string(project.updated_at_ms),
        sessions: conversations.len(),
        turns,
        turn_sessions_indexed,
        input_tokens: project.usage.input_tokens,
        cached_input_tokens: project.usage.cached_input_tokens,
        cache_write_input_tokens: project.usage.cache_write_input_tokens,
        output_tokens: project.usage.output_tokens,
        total_tokens: project.usage.total_tokens,
        priced_tokens: project.usage.priced_tokens,
        unpriced_tokens: project.usage.unpriced_tokens,
        cost_usd: project.usage.cost.total_usd,
        conversations,
    }
}

fn conversation_usage_row(conversation: ConversationAggregate) -> ProjectUsageConversation {
    let turn_rows = conversation
        .turn_rows_override
        .unwrap_or_else(|| turn_usage_rows(conversation.turns));

    ProjectUsageConversation {
        id: conversation.id,
        title: conversation.title,
        session_path: conversation.session_path,
        updated_at: timestamp_string(conversation.updated_at_ms),
        is_subagent: conversation.is_subagent,
        parent_id: conversation.parent_id,
        models: conversation.models.into_iter().collect(),
        turns: turn_rows.len(),
        turns_indexed: conversation.turns_indexed,
        input_tokens: conversation.usage.input_tokens,
        cached_input_tokens: conversation.usage.cached_input_tokens,
        cache_write_input_tokens: conversation.usage.cache_write_input_tokens,
        output_tokens: conversation.usage.output_tokens,
        total_tokens: conversation.usage.total_tokens,
        priced_tokens: conversation.usage.priced_tokens,
        unpriced_tokens: conversation.usage.unpriced_tokens,
        cost_usd: conversation.usage.cost.total_usd,
        turn_rows,
    }
}

fn turn_usage_rows(turns: BTreeMap<String, TurnAggregate>) -> Vec<ProjectUsageTurn> {
    let mut rows = turns.into_iter().collect::<Vec<_>>();
    rows.sort_by(|(left_id, left), (right_id, right)| {
        left.started_at_ms
            .cmp(&right.started_at_ms)
            .then_with(|| left_id.cmp(right_id))
    });
    rows.into_iter()
        .enumerate()
        .map(|(index, (id, turn))| ProjectUsageTurn {
            id,
            sequence: index + 1,
            started_at: timestamp_string(turn.started_at_ms),
            updated_at: timestamp_string(turn.updated_at_ms),
            models: turn.models.into_iter().collect(),
            input_tokens: turn.usage.input_tokens,
            cached_input_tokens: turn.usage.cached_input_tokens,
            cache_write_input_tokens: turn.usage.cache_write_input_tokens,
            output_tokens: turn.usage.output_tokens,
            total_tokens: turn.usage.total_tokens,
            priced_tokens: turn.usage.priced_tokens,
            unpriced_tokens: turn.usage.unpriced_tokens,
            cost_usd: turn.usage.cost.total_usd,
        })
        .collect()
}

fn latest_timestamp(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn timestamp_string(timestamp_ms: Option<i64>) -> Option<String> {
    timestamp_ms
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|value| value.to_rfc3339())
}

fn project_name(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn model_rows(models: impl Iterator<Item = (String, Aggregate)>) -> Vec<ModelCostEstimate> {
    let mut rows = models
        .map(|(model, aggregate)| ModelCostEstimate {
            model,
            input_tokens: aggregate.input_tokens,
            cached_input_tokens: aggregate.cached_input_tokens,
            cache_write_input_tokens: aggregate.cache_write_input_tokens,
            output_tokens: aggregate.output_tokens,
            total_tokens: aggregate.total_tokens,
            cost_usd: aggregate.cost.total_usd,
            cache_savings_usd: aggregate.cost.cache_savings_usd,
            priced: aggregate.priced_tokens > 0,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .cost_usd
            .total_cmp(&left.cost_usd)
            .then_with(|| right.total_tokens.cmp(&left.total_tokens))
    });
    rows
}

fn load_index(path: &Path, warnings: &mut Vec<String>) -> CostIndex {
    match read_cost_index(path) {
        Ok(index) => index,
        Err(error) => {
            warnings.push(format!("成本索引无法读取，正在重建：{error}"));
            CostIndex {
                schema_version: INDEX_SCHEMA_VERSION,
                ..CostIndex::default()
            }
        }
    }
}

fn write_index(path: &Path, index: &CostIndex) -> Result<(), String> {
    let files = index
        .files
        .iter()
        .map(|(session_path, indexed)| UsageIndexFile {
            path: session_path.clone(),
            length: indexed.length,
            modified_ms: indexed.modified_ms,
            malformed_lines: indexed.malformed_lines,
            session_id: indexed.session_id.clone(),
            parent_id: indexed.parent_id.clone(),
            forked_at_ms: indexed.forked_at_ms,
            cwd: indexed.cwd.clone(),
            started_at_ms: indexed.started_at_ms,
            updated_at_ms: indexed.updated_at_ms,
            project_metadata_indexed: indexed.project_metadata_indexed,
            turns: indexed
                .turns
                .iter()
                .map(|(turn_id, turn)| UsageIndexTurn {
                    id: turn_id.clone(),
                    started_at_ms: turn.started_at_ms,
                    updated_at_ms: turn.updated_at_ms,
                })
                .collect(),
            events: indexed
                .events
                .iter()
                .enumerate()
                .map(|(event_index, event)| UsageIndexEvent {
                    event_index,
                    timestamp_ms: event.timestamp_ms,
                    date: event.date.clone(),
                    turn_id: event.turn_id.clone(),
                    model: event.model.clone(),
                    input_tokens: event.input_tokens,
                    cached_input_tokens: event.cached_input_tokens,
                    cache_write_input_tokens: event.cache_write_input_tokens,
                    output_tokens: event.output_tokens,
                    total_tokens: event.total_tokens,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    write_usage_index(path, &files)
}

fn read_cost_index(path: &Path) -> Result<CostIndex, String> {
    let files = read_usage_index(path)?
        .into_iter()
        .map(|file| {
            (
                file.path,
                IndexedFile {
                    length: file.length,
                    modified_ms: file.modified_ms,
                    malformed_lines: file.malformed_lines,
                    session_id: file.session_id,
                    parent_id: file.parent_id,
                    forked_at_ms: file.forked_at_ms,
                    cwd: file.cwd,
                    started_at_ms: file.started_at_ms,
                    updated_at_ms: file.updated_at_ms,
                    turns: file
                        .turns
                        .into_iter()
                        .map(|turn| {
                            (
                                turn.id,
                                IndexedTurn {
                                    started_at_ms: turn.started_at_ms,
                                    updated_at_ms: turn.updated_at_ms,
                                },
                            )
                        })
                        .collect(),
                    project_metadata_indexed: file.project_metadata_indexed,
                    events: file
                        .events
                        .into_iter()
                        .map(|event| CostUsageEvent {
                            timestamp_ms: event.timestamp_ms,
                            date: event.date,
                            turn_id: event.turn_id,
                            model: event.model,
                            input_tokens: event.input_tokens,
                            cached_input_tokens: event.cached_input_tokens,
                            cache_write_input_tokens: event.cache_write_input_tokens,
                            output_tokens: event.output_tokens,
                            total_tokens: event.total_tokens,
                        })
                        .collect(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(CostIndex {
        schema_version: INDEX_SCHEMA_VERSION,
        files,
    })
}

fn parse_cost_file(path: &Path, length: u64, modified_ms: u128) -> Result<IndexedFile, String> {
    let file = File::open(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut line = Vec::new();
    let mut malformed_lines = 0;
    let mut events = Vec::new();
    let mut current_model = "unknown".to_string();
    let mut previous_total: Option<RawUsage> = None;
    let mut session_id = None;
    let mut parent_id = None;
    let mut forked_at_ms = None;
    let mut cwd = None;
    let mut started_at_ms: Option<i64> = None;
    let mut updated_at_ms: Option<i64> = None;
    let mut current_turn = "unassigned".to_string();
    let mut turns = BTreeMap::<String, IndexedTurn>::new();

    loop {
        line.clear();
        let bytes_read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
        if bytes_read == 0 {
            break;
        }

        let is_session_meta = memmem::find(&line, b"session_meta").is_some();
        let is_turn_context = memmem::find(&line, b"turn_context").is_some();
        let is_token_count = memmem::find(&line, b"token_count").is_some();
        let is_task_started = memmem::find(&line, b"task_started").is_some();
        let is_task_complete = memmem::find(&line, b"task_complete").is_some();
        if !is_session_meta
            && !is_turn_context
            && !is_token_count
            && !is_task_started
            && !is_task_complete
        {
            continue;
        }
        let Ok(entry) = serde_json::from_slice::<Value>(&line) else {
            malformed_lines += 1;
            continue;
        };
        let payload = entry.get("payload").unwrap_or(&Value::Null);
        let entry_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
        let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
        let timestamp_ms = entry
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
            .map(|timestamp| timestamp.timestamp_millis());
        started_at_ms = match (started_at_ms, timestamp_ms) {
            (Some(current), Some(timestamp)) => Some(current.min(timestamp)),
            (None, Some(timestamp)) => Some(timestamp),
            (current, None) => current,
        };
        updated_at_ms = latest_timestamp(updated_at_ms, timestamp_ms);

        if entry_type == "session_meta" {
            if session_id.is_none() {
                session_id = payload
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                parent_id = payload
                    .get("forked_from_id")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        payload
                            .pointer("/source/subagent/thread_spawn/parent_thread_id")
                            .and_then(Value::as_str)
                    })
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                forked_at_ms = entry
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
                    .map(|timestamp| timestamp.timestamp_millis());
            }
            cwd = cwd.or_else(|| {
                payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            continue;
        }
        if entry_type == "turn_context" {
            if let Some(model) = payload.get("model").and_then(Value::as_str) {
                current_model = model.to_string();
            }
            if let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) {
                current_turn = turn_id.to_string();
                let turn = turns.entry(current_turn.clone()).or_default();
                turn.started_at_ms = turn.started_at_ms.or(timestamp_ms);
                turn.updated_at_ms = latest_timestamp(turn.updated_at_ms, timestamp_ms);
            }
            continue;
        }
        if entry_type == "event_msg" && payload_type == "task_started" {
            if let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) {
                current_turn = turn_id.to_string();
                let turn = turns.entry(current_turn.clone()).or_default();
                turn.started_at_ms = payload
                    .get("started_at")
                    .and_then(Value::as_str)
                    .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
                    .map(|timestamp| timestamp.timestamp_millis())
                    .or(turn.started_at_ms)
                    .or(timestamp_ms);
                turn.updated_at_ms = latest_timestamp(turn.updated_at_ms, timestamp_ms);
            }
            continue;
        }
        if entry_type == "event_msg" && payload_type == "task_complete" {
            if let Some(turn_id) = payload.get("turn_id").and_then(Value::as_str) {
                let turn = turns.entry(turn_id.to_string()).or_default();
                turn.updated_at_ms = payload
                    .get("completed_at")
                    .and_then(Value::as_str)
                    .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
                    .map(|timestamp| timestamp.timestamp_millis())
                    .or(timestamp_ms)
                    .or(turn.updated_at_ms);
            }
            continue;
        }
        if entry_type != "event_msg" || payload_type != "token_count" {
            continue;
        }
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
        if usage.input_tokens == 0 && usage.cached_input_tokens == 0 && usage.output_tokens == 0 {
            continue;
        }

        let Some(timestamp) = entry.get("timestamp").and_then(Value::as_str) else {
            continue;
        };
        let Ok(parsed_timestamp) = DateTime::parse_from_rfc3339(timestamp) else {
            malformed_lines += 1;
            continue;
        };
        let model = info
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(&current_model)
            .to_string();
        events.push(CostUsageEvent {
            timestamp_ms: parsed_timestamp.timestamp_millis(),
            date: parsed_timestamp
                .with_timezone(&Local)
                .date_naive()
                .to_string(),
            turn_id: current_turn.clone(),
            model,
            input_tokens: usage.input_tokens,
            cached_input_tokens: usage.cached_input_tokens.min(usage.input_tokens),
            cache_write_input_tokens: usage.cache_write_input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
        });
    }

    Ok(IndexedFile {
        length,
        modified_ms,
        malformed_lines,
        session_id,
        parent_id,
        forked_at_ms,
        cwd,
        started_at_ms,
        updated_at_ms,
        turns,
        project_metadata_indexed: true,
        events,
    })
}

fn replayed_prefix_len(
    child: &IndexedFile,
    files_by_session_id: &HashMap<&str, &IndexedFile>,
) -> usize {
    let Some(parent_id) = child.parent_id.as_deref() else {
        return 0;
    };
    let parent = files_by_session_id
        .get(parent_id)
        .copied()
        .filter(|parent| !std::ptr::eq(*parent, child));
    let parent_events = parent
        .map(|parent| {
            parent
                .events
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
    for (child_index, event) in child.events.iter().enumerate() {
        if parent_events
            .get(matched)
            .is_some_and(|parent| same_usage(parent, event))
        {
            matched += 1;
            continue;
        }
        if matched > 0 {
            return child_index;
        }
        return rewritten_first_second_len(&child.events);
    }
    child.events.len()
}

fn same_usage(left: &CostUsageEvent, right: &CostUsageEvent) -> bool {
    left.input_tokens == right.input_tokens
        && left.cached_input_tokens == right.cached_input_tokens
        && left.output_tokens == right.output_tokens
        && left.total_tokens == right.total_tokens
}

fn rewritten_first_second_len(events: &[CostUsageEvent]) -> usize {
    let Some(first) = events.first() else {
        return 0;
    };
    let Some(second) = events.get(1) else {
        return 0;
    };
    let first_second = first.timestamp_ms.div_euclid(1_000);
    if second.timestamp_ms.div_euclid(1_000) != first_second {
        return 0;
    }
    events
        .iter()
        .take_while(|event| event.timestamp_ms.div_euclid(1_000) == first_second)
        .count()
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
        cached_input_tokens: read("cached_input_tokens"),
        cache_write_input_tokens: read("cache_write_input_tokens"),
        output_tokens,
        total_tokens: if reported_total == 0 {
            input_tokens.saturating_add(output_tokens)
        } else {
            reported_total
        },
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

fn discover_usage_files() -> Result<Vec<PathBuf>, String> {
    let mut output = Vec::new();
    for home in codex_homes()? {
        let active_dir = home.join("sessions");
        let archived_dir = home.join("archived_sessions");
        let mut selected = HashMap::<PathBuf, PathBuf>::new();
        collect_logs(&archived_dir, &archived_dir, &mut selected)?;
        collect_logs(&active_dir, &active_dir, &mut selected)?;
        output.extend(selected.into_values());
    }
    output.sort();
    output.dedup();
    Ok(output)
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
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        selected.insert(relative, path);
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
    fn parses_project_turn_and_token_deltas_without_message_or_tool_outputs() {
        let directory = std::env::temp_dir().join(format!(
            "codex-xray-cost-test-{}",
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
            "{{\"timestamp\":\"2026-07-26T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"session-1\",\"cwd\":\"/tmp/demo\"}}}}"
        )
        .expect("write fixture");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:00.100Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"turn-1\"}}}}"
        )
        .expect("write fixture");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:00.200Z\",\"type\":\"turn_context\",\"payload\":{{\"turn_id\":\"turn-1\",\"model\":\"gpt-5.4\"}}}}"
        )
        .expect("write fixture");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":100,\"cached_input_tokens\":40,\"output_tokens\":10,\"total_tokens\":110}},\"last_token_usage\":{{\"input_tokens\":100,\"cached_input_tokens\":40,\"output_tokens\":10,\"total_tokens\":110}}}}}}}}"
        )
        .expect("write fixture");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-26T00:00:02Z\",\"type\":\"response_item\",\"payload\":{{\"content\":\"private source text\"}}}}"
        )
        .expect("write fixture");
        drop(file);

        let metadata = fs::metadata(&path).expect("metadata");
        let indexed =
            parse_cost_file(&path, metadata.len(), modified_millis(&metadata)).expect("parse");
        assert_eq!(indexed.events.len(), 1);
        assert_eq!(indexed.events[0].model, "gpt-5.4");
        assert_eq!(indexed.events[0].cached_input_tokens, 40);
        assert_eq!(indexed.events[0].turn_id, "turn-1");
        assert_eq!(indexed.cwd.as_deref(), Some("/tmp/demo"));
        assert!(indexed.turns.contains_key("turn-1"));

        fs::remove_dir_all(directory).expect("remove temp directory");
    }

    #[test]
    fn builds_project_conversation_and_turn_rows_from_the_cost_index() {
        let directory = std::env::temp_dir().join(format!(
            "codex-xray-project-usage-test-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create temp directory");
        let index_path = directory.join("cost-index.json");
        let indexed = IndexedFile {
            session_id: Some("session-1".to_string()),
            cwd: Some("/tmp/demo".to_string()),
            updated_at_ms: Some(2_000),
            project_metadata_indexed: true,
            turns: BTreeMap::from([(
                "turn-1".to_string(),
                IndexedTurn {
                    started_at_ms: Some(1_000),
                    updated_at_ms: Some(2_000),
                },
            )]),
            events: vec![event(1_500, 100)],
            ..IndexedFile::default()
        };
        let index = CostIndex {
            schema_version: INDEX_SCHEMA_VERSION,
            files: BTreeMap::from([("/tmp/session-1.jsonl".to_string(), indexed)]),
        };
        write_index(&index_path, &index).expect("write index");
        let metadata = [ThreadMetadata {
            id: "session-1".to_string(),
            name: Some("Inspect the local index".to_string()),
            cwd: "/tmp/demo".to_string(),
            status: Some("idle".to_string()),
            path: Some("/tmp/session-1.jsonl".to_string()),
            created_at: 1,
            updated_at: 2,
            parent_thread_id: None,
        }];

        let snapshot =
            build_project_usage(&index_path, Some(&metadata)).expect("build project usage");
        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.sessions, 1);
        assert_eq!(snapshot.turns, 1);
        assert_eq!(snapshot.total_tokens, 100);
        assert_eq!(snapshot.projects[0].name, "demo");
        assert_eq!(
            snapshot.projects[0].conversations[0].title.as_deref(),
            Some("Inspect the local index")
        );
        assert_eq!(
            snapshot.projects[0].conversations[0].turn_rows[0].id,
            "turn-1"
        );

        fs::remove_dir_all(directory).expect("remove temp directory");
    }

    fn event(timestamp_ms: i64, input_tokens: u64) -> CostUsageEvent {
        CostUsageEvent {
            timestamp_ms,
            date: "2026-07-26".to_string(),
            turn_id: "turn-1".to_string(),
            model: "gpt-5.4".to_string(),
            input_tokens,
            cached_input_tokens: 0,
            cache_write_input_tokens: 0,
            output_tokens: 0,
            total_tokens: input_tokens,
        }
    }

    #[test]
    fn skips_parent_usage_replayed_into_forked_session() {
        let parent = IndexedFile {
            length: 0,
            modified_ms: 0,
            malformed_lines: 0,
            session_id: Some("parent".to_string()),
            parent_id: None,
            forked_at_ms: None,
            events: vec![event(1_000, 100), event(2_000, 200)],
            ..IndexedFile::default()
        };
        let child = IndexedFile {
            length: 0,
            modified_ms: 0,
            malformed_lines: 0,
            session_id: Some("child".to_string()),
            parent_id: Some("parent".to_string()),
            forked_at_ms: Some(2_500),
            events: vec![event(3_000, 100), event(3_001, 200), event(4_000, 50)],
            ..IndexedFile::default()
        };
        let parents = HashMap::from([("parent", &parent)]);
        assert_eq!(replayed_prefix_len(&child, &parents), 2);
    }

    #[test]
    fn skips_rewritten_second_when_parent_log_is_missing() {
        let child = IndexedFile {
            length: 0,
            modified_ms: 0,
            malformed_lines: 0,
            session_id: Some("child".to_string()),
            parent_id: Some("missing".to_string()),
            forked_at_ms: Some(2_500),
            events: vec![event(3_000, 100), event(3_050, 200), event(4_000, 50)],
            ..IndexedFile::default()
        };
        assert_eq!(replayed_prefix_len(&child, &HashMap::new()), 2);
    }

    #[test]
    fn canonicalizes_existing_project_paths_and_preserves_missing_ones() {
        let current = std::env::current_dir().expect("current directory");
        assert_eq!(
            canonical_project_path("."),
            current.to_string_lossy().into_owned()
        );
        assert_eq!(
            canonical_project_path("/definitely/missing/codex-xray-project"),
            "/definitely/missing/codex-xray-project"
        );
    }
}
