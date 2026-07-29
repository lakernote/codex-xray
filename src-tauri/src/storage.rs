use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const DATABASE_SCHEMA_VERSION: u32 = 4;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageHealth {
    pub database_bytes: u64,
    pub wal_bytes: u64,
    pub journal_mode: String,
    pub schema_version: u32,
    pub integrity_ok: bool,
    pub integrity_message: String,
    pub foreign_key_violations: u64,
    pub malformed_session_lines: u64,
    pub usage_sessions: u64,
    pub usage_turns: u64,
    pub token_events: u64,
    pub trace_sessions: u64,
    pub trace_turns: u64,
    pub trace_tool_events: u64,
}

#[derive(Debug, Clone, Default)]
pub struct UsageIndexFile {
    pub path: String,
    pub length: u64,
    pub modified_ms: u128,
    pub malformed_lines: usize,
    pub session_id: Option<String>,
    pub parent_id: Option<String>,
    pub forked_at_ms: Option<i64>,
    pub cwd: Option<String>,
    pub started_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub project_metadata_indexed: bool,
    pub turns: Vec<UsageIndexTurn>,
    pub events: Vec<UsageIndexEvent>,
}

#[derive(Debug, Clone, Default)]
pub struct UsageIndexTurn {
    pub id: String,
    pub started_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct UsageIndexEvent {
    pub event_index: usize,
    pub timestamp_ms: i64,
    pub date: String,
    pub turn_id: String,
    pub model: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TraceMirrorSession {
    pub session_path: String,
    pub length: u64,
    pub modified_ms: u128,
    pub session_id: Option<String>,
    pub parent_id: Option<String>,
    pub cwd: Option<String>,
    pub conversation_name: Option<String>,
    pub official_status: Option<String>,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    pub model: Option<String>,
    pub malformed_lines: usize,
    pub turns: Vec<TraceMirrorTurn>,
    pub tool_events: Vec<TraceMirrorToolEvent>,
    pub phase_events: Vec<TraceMirrorPhaseEvent>,
    pub usage_events: Vec<TraceMirrorUsageEvent>,
}

#[derive(Debug, Clone, Default)]
pub struct TraceMirrorTurn {
    pub turn_id: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub summary_mode: Option<String>,
    pub started_source_order: Option<usize>,
    pub started_at_ms: Option<i64>,
    pub completed_source_order: Option<usize>,
    pub completed_at_ms: Option<i64>,
    pub duration_ms: Option<u64>,
    pub structured_failures: usize,
    pub context_compactions: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TraceMirrorToolEvent {
    pub turn_id: String,
    pub source_order: usize,
    pub completed_source_order: Option<usize>,
    pub execution_completed_source_order: Option<usize>,
    pub timestamp_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub execution_completed_at_ms: Option<i64>,
    pub call_id: Option<String>,
    pub source_type: String,
    pub execution_end_source_type: Option<String>,
    pub result_source_type: Option<String>,
    pub name: String,
    pub category: String,
    pub server: Option<String>,
    pub subject: Option<String>,
    pub detail: Option<String>,
    pub arguments_json: Option<String>,
    pub result_json: Option<String>,
    pub repeated: bool,
    pub failed: bool,
    pub output_bytes: u64,
    pub exit_code: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct TraceMirrorPhaseEvent {
    pub turn_id: String,
    pub source_order: usize,
    pub source_end_order: Option<usize>,
    pub timestamp_ms: i64,
    pub phase: String,
    pub source_type: String,
    pub role: Option<String>,
    pub content_bytes: u64,
    pub encrypted_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TraceMirrorUsageEvent {
    pub turn_id: String,
    pub source_order: usize,
    pub timestamp_ms: i64,
    pub model: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    pub context_window: Option<u64>,
}

fn open(path: &Path) -> Result<Connection, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Codex X-Ray 数据库路径没有父目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建数据目录：{error}"))?;
    let connection =
        Connection::open(path).map_err(|error| format!("无法打开分析数据库：{error}"))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|error| format!("无法设置数据库等待时间：{error}"))?;
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA temp_store = MEMORY;
            CREATE TABLE IF NOT EXISTS app_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )
        .map_err(|error| format!("无法准备分析数据库：{error}"))?;
    let previous_schema = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("无法读取数据库版本：{error}"))?
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_default();
    if previous_schema < 3 {
        connection
            .execute_batch(
                "
                DROP TABLE IF EXISTS usage_token_events;
                DROP TABLE IF EXISTS usage_session_turns;
                DROP TABLE IF EXISTS usage_session_files;
                DROP TABLE IF EXISTS trace_usage_events;
                DROP TABLE IF EXISTS trace_phase_events;
                DROP TABLE IF EXISTS trace_tool_events;
                DROP TABLE IF EXISTS trace_turns;
                DROP TABLE IF EXISTS trace_sessions;
                ",
            )
            .map_err(|error| format!("无法升级关系索引：{error}"))?;
    }
    if previous_schema < 4 {
        connection
            .execute_batch(
                "
                DROP TABLE IF EXISTS project_turn_usage;
                DROP TABLE IF EXISTS project_turn_sessions;
                ",
            )
            .map_err(|error| format!("无法清理重复的项目 Turn 表：{error}"))?;
    }
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA temp_store = MEMORY;

            CREATE TABLE IF NOT EXISTS app_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS cache_entries (
                cache_key TEXT PRIMARY KEY,
                payload BLOB NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS index_meta (
                namespace TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS index_entries (
                namespace TEXT NOT NULL,
                entry_key TEXT NOT NULL,
                payload BLOB NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (namespace, entry_key),
                FOREIGN KEY (namespace) REFERENCES index_meta(namespace) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_index_entries_namespace
            ON index_entries(namespace);

            CREATE TABLE IF NOT EXISTS usage_session_files (
                id INTEGER PRIMARY KEY,
                session_path TEXT NOT NULL UNIQUE,
                length INTEGER NOT NULL,
                modified_ms TEXT NOT NULL,
                malformed_lines INTEGER NOT NULL,
                session_id TEXT,
                parent_id TEXT,
                forked_at_ms INTEGER,
                cwd TEXT,
                started_at_ms INTEGER,
                updated_at_ms INTEGER,
                project_metadata_indexed INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_usage_session_files_session
            ON usage_session_files(session_id);
            CREATE INDEX IF NOT EXISTS idx_usage_session_files_project
            ON usage_session_files(cwd, updated_at_ms DESC);

            CREATE TABLE IF NOT EXISTS usage_session_turns (
                session_file_id INTEGER NOT NULL,
                turn_id TEXT NOT NULL,
                started_at_ms INTEGER,
                updated_at_ms INTEGER,
                PRIMARY KEY(session_file_id, turn_id),
                FOREIGN KEY(session_file_id) REFERENCES usage_session_files(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_usage_session_turns_time
            ON usage_session_turns(started_at_ms);

            CREATE TABLE IF NOT EXISTS usage_token_events (
                session_file_id INTEGER NOT NULL,
                event_index INTEGER NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                date TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                cache_write_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                PRIMARY KEY(session_file_id, event_index),
                FOREIGN KEY(session_file_id) REFERENCES usage_session_files(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_usage_token_events_date_model
            ON usage_token_events(date, model);
            CREATE INDEX IF NOT EXISTS idx_usage_token_events_turn
            ON usage_token_events(session_file_id, turn_id, timestamp_ms);

            CREATE TABLE IF NOT EXISTS trace_sessions (
                session_path TEXT PRIMARY KEY,
                length INTEGER NOT NULL,
                modified_ms TEXT NOT NULL,
                session_id TEXT,
                parent_id TEXT,
                cwd TEXT,
                conversation_name TEXT,
                official_status TEXT,
                started_at TEXT,
                updated_at TEXT,
                model TEXT,
                malformed_lines INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_trace_sessions_project
            ON trace_sessions(cwd, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_trace_sessions_id
            ON trace_sessions(session_id);

            CREATE TABLE IF NOT EXISTS trace_turns (
                session_path TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                model TEXT NOT NULL,
                reasoning_effort TEXT,
                summary_mode TEXT,
                started_source_order INTEGER,
                started_at_ms INTEGER,
                completed_source_order INTEGER,
                completed_at_ms INTEGER,
                duration_ms INTEGER,
                structured_failures INTEGER NOT NULL,
                context_compactions INTEGER NOT NULL,
                PRIMARY KEY(session_path, turn_id),
                FOREIGN KEY(session_path) REFERENCES trace_sessions(session_path) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_trace_turns_time
            ON trace_turns(started_at_ms);

            CREATE TABLE IF NOT EXISTS trace_tool_events (
                session_path TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                source_order INTEGER NOT NULL,
                completed_source_order INTEGER,
                execution_completed_source_order INTEGER,
                timestamp_ms INTEGER NOT NULL,
                completed_at_ms INTEGER,
                execution_completed_at_ms INTEGER,
                call_id TEXT,
                source_type TEXT NOT NULL,
                execution_end_source_type TEXT,
                result_source_type TEXT,
                name TEXT NOT NULL,
                category TEXT NOT NULL,
                server TEXT,
                subject TEXT,
                detail TEXT,
                arguments_json TEXT,
                result_json TEXT,
                repeated INTEGER NOT NULL,
                failed INTEGER NOT NULL,
                output_bytes INTEGER NOT NULL,
                exit_code INTEGER,
                PRIMARY KEY(session_path, source_order),
                FOREIGN KEY(session_path) REFERENCES trace_sessions(session_path) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_trace_tool_events_name
            ON trace_tool_events(name, timestamp_ms);
            CREATE INDEX IF NOT EXISTS idx_trace_tool_events_turn
            ON trace_tool_events(turn_id, timestamp_ms);

            CREATE TABLE IF NOT EXISTS trace_phase_events (
                session_path TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                source_order INTEGER NOT NULL,
                source_end_order INTEGER,
                timestamp_ms INTEGER NOT NULL,
                phase TEXT NOT NULL,
                source_type TEXT NOT NULL,
                role TEXT,
                content_bytes INTEGER NOT NULL,
                encrypted_bytes INTEGER NOT NULL,
                PRIMARY KEY(session_path, source_order),
                FOREIGN KEY(session_path) REFERENCES trace_sessions(session_path) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_trace_phase_events_turn
            ON trace_phase_events(turn_id, timestamp_ms);

            CREATE TABLE IF NOT EXISTS trace_usage_events (
                session_path TEXT NOT NULL,
                source_order INTEGER NOT NULL,
                turn_id TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                reasoning_output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                context_window INTEGER,
                PRIMARY KEY(session_path, source_order),
                FOREIGN KEY(session_path) REFERENCES trace_sessions(session_path) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_trace_usage_events_turn
            ON trace_usage_events(turn_id, timestamp_ms);
            ",
        )
        .map_err(|error| format!("无法初始化分析数据库：{error}"))?;
    if previous_schema < 2 {
        connection
            .execute_batch(
                "
                DELETE FROM index_entries
                WHERE namespace IN ('cost-files', 'project-turns');
                DELETE FROM index_meta
                WHERE namespace IN ('cost-files', 'project-turns');
                ",
            )
            .map_err(|error| format!("无法清理旧用量索引：{error}"))?;
    }
    connection
        .execute(
            "INSERT INTO app_meta(key, value) VALUES('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [DATABASE_SCHEMA_VERSION.to_string()],
        )
        .map_err(|error| format!("无法记录数据库版本：{error}"))?;
    Ok(connection)
}

pub fn initialize(path: &Path) -> Result<(), String> {
    open(path).map(|_| ())
}

pub fn health(path: &Path) -> Result<StorageHealth, String> {
    let connection = open(path)?;
    let journal_mode = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .map_err(|error| format!("无法读取数据库日志模式：{error}"))?;
    let counts = connection
        .query_row(
            "SELECT
                (SELECT count(*) FROM usage_session_files),
                (SELECT count(*) FROM usage_session_turns),
                (SELECT count(*) FROM usage_token_events),
                (SELECT count(*) FROM trace_sessions),
                (SELECT count(*) FROM trace_turns),
                (SELECT count(*) FROM trace_tool_events),
                (SELECT coalesce(sum(malformed_lines), 0) FROM usage_session_files)",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|error| format!("无法统计分析数据库：{error}"))?;
    let schema_version = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_default();
    let integrity_message = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0))
        .unwrap_or_else(|error| format!("quick_check 执行失败：{error}"));
    let integrity_ok = integrity_message.eq_ignore_ascii_case("ok");
    let foreign_key_violations = connection
        .prepare("PRAGMA foreign_key_check")
        .and_then(|mut statement| {
            let rows = statement.query_map([], |_| Ok(()))?;
            Ok(rows.filter_map(Result::ok).count() as u64)
        })
        .unwrap_or(u64::MAX);
    let wal_path = Path::new(&format!("{}-wal", path.to_string_lossy())).to_path_buf();
    Ok(StorageHealth {
        database_bytes: fs::metadata(path)
            .map(|value| value.len())
            .unwrap_or_default(),
        wal_bytes: fs::metadata(wal_path)
            .map(|value| value.len())
            .unwrap_or_default(),
        journal_mode,
        schema_version,
        integrity_ok,
        integrity_message,
        foreign_key_violations,
        malformed_session_lines: unsigned(counts.6),
        usage_sessions: unsigned(counts.0),
        usage_turns: unsigned(counts.1),
        token_events: unsigned(counts.2),
        trace_sessions: unsigned(counts.3),
        trace_turns: unsigned(counts.4),
        trace_tool_events: unsigned(counts.5),
    })
}

pub fn read_cache<T: DeserializeOwned>(path: &Path, key: &str) -> Result<Option<T>, String> {
    let connection = open(path)?;
    let payload = connection
        .query_row(
            "SELECT payload FROM cache_entries WHERE cache_key = ?1",
            [key],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(|error| format!("无法读取缓存 {key}：{error}"))?;
    payload
        .map(|payload| {
            serde_json::from_slice(&payload)
                .map_err(|error| format!("缓存 {key} 无法解析：{error}"))
        })
        .transpose()
}

pub fn write_cache<T: Serialize>(path: &Path, key: &str, value: &T) -> Result<(), String> {
    let payload =
        serde_json::to_vec(value).map_err(|error| format!("缓存 {key} 无法序列化：{error}"))?;
    let connection = open(path)?;
    connection
        .execute(
            "INSERT INTO cache_entries(cache_key, payload, updated_at)
             VALUES(?1, ?2, unixepoch())
             ON CONFLICT(cache_key) DO UPDATE SET
                 payload = excluded.payload,
                 updated_at = excluded.updated_at",
            params![key, payload],
        )
        .map_err(|error| format!("无法保存缓存 {key}：{error}"))?;
    Ok(())
}

pub fn read_index_entries(
    path: &Path,
    namespace: &str,
    schema_version: u32,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut connection = open(path)?;
    let stored_version = connection
        .query_row(
            "SELECT schema_version FROM index_meta WHERE namespace = ?1",
            [namespace],
            |row| row.get::<_, u32>(0),
        )
        .optional()
        .map_err(|error| format!("无法读取 {namespace} 索引版本：{error}"))?;
    if stored_version != Some(schema_version) {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("无法重建 {namespace} 索引：{error}"))?;
        transaction
            .execute(
                "DELETE FROM index_entries WHERE namespace = ?1",
                [namespace],
            )
            .map_err(|error| format!("无法清空 {namespace} 索引：{error}"))?;
        transaction
            .execute(
                "INSERT INTO index_meta(namespace, schema_version) VALUES(?1, ?2)
                 ON CONFLICT(namespace) DO UPDATE SET schema_version = excluded.schema_version",
                params![namespace, schema_version],
            )
            .map_err(|error| format!("无法更新 {namespace} 索引版本：{error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("无法提交 {namespace} 索引重建：{error}"))?;
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(
            "SELECT entry_key, payload
             FROM index_entries
             WHERE namespace = ?1
             ORDER BY entry_key",
        )
        .map_err(|error| format!("无法准备 {namespace} 索引查询：{error}"))?;
    let rows = statement
        .query_map([namespace], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|error| format!("无法查询 {namespace} 索引：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法读取 {namespace} 索引记录：{error}"))
}

pub fn write_index_entries(
    path: &Path,
    namespace: &str,
    schema_version: u32,
    entries: &[(String, Vec<u8>)],
) -> Result<(), String> {
    let mut connection = open(path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("无法开始保存 {namespace} 索引：{error}"))?;
    transaction
        .execute(
            "INSERT INTO index_meta(namespace, schema_version) VALUES(?1, ?2)
             ON CONFLICT(namespace) DO UPDATE SET schema_version = excluded.schema_version",
            params![namespace, schema_version],
        )
        .map_err(|error| format!("无法保存 {namespace} 索引版本：{error}"))?;

    let current_keys = entries
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<HashSet<_>>();
    {
        let mut statement = transaction
            .prepare("SELECT entry_key FROM index_entries WHERE namespace = ?1")
            .map_err(|error| format!("无法读取 {namespace} 旧索引键：{error}"))?;
        let old_keys = statement
            .query_map([namespace], |row| row.get::<_, String>(0))
            .map_err(|error| format!("无法查询 {namespace} 旧索引键：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("无法读取 {namespace} 旧索引键：{error}"))?;
        for key in old_keys {
            if !current_keys.contains(key.as_str()) {
                transaction
                    .execute(
                        "DELETE FROM index_entries
                         WHERE namespace = ?1 AND entry_key = ?2",
                        params![namespace, key],
                    )
                    .map_err(|error| format!("无法删除 {namespace} 过期索引：{error}"))?;
            }
        }
    }

    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO index_entries(namespace, entry_key, payload, updated_at)
                 VALUES(?1, ?2, ?3, unixepoch())
                 ON CONFLICT(namespace, entry_key) DO UPDATE SET
                     payload = excluded.payload,
                     updated_at = excluded.updated_at
                 WHERE index_entries.payload <> excluded.payload",
            )
            .map_err(|error| format!("无法准备 {namespace} 索引写入：{error}"))?;
        for (key, payload) in entries {
            statement
                .execute(params![namespace, key, payload])
                .map_err(|error| format!("无法保存 {namespace} 索引记录 {key}：{error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交 {namespace} 索引：{error}"))
}

fn integer(value: u64, field: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{field} 超出 SQLite INTEGER 范围"))
}

fn count(value: usize, field: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{field} 超出 SQLite INTEGER 范围"))
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn position(value: i64) -> usize {
    usize::try_from(value).unwrap_or_default()
}

pub fn read_usage_index(path: &Path) -> Result<Vec<UsageIndexFile>, String> {
    let connection = open(path)?;
    let mut files = BTreeMap::<String, UsageIndexFile>::new();
    let mut paths_by_id = BTreeMap::<i64, String>::new();
    {
        let mut statement = connection
            .prepare(
                "SELECT id, session_path, length, modified_ms, malformed_lines, session_id,
                        parent_id, forked_at_ms, cwd, started_at_ms, updated_at_ms,
                        project_metadata_indexed
                 FROM usage_session_files
                 ORDER BY session_path",
            )
            .map_err(|error| format!("无法准备 Session 索引查询：{error}"))?;
        let rows = statement
            .query_map([], |row| {
                let modified_ms = row.get::<_, String>(3)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    UsageIndexFile {
                        path: row.get(1)?,
                        length: unsigned(row.get(2)?),
                        modified_ms: modified_ms.parse().unwrap_or_default(),
                        malformed_lines: position(row.get(4)?),
                        session_id: row.get(5)?,
                        parent_id: row.get(6)?,
                        forked_at_ms: row.get(7)?,
                        cwd: row.get(8)?,
                        started_at_ms: row.get(9)?,
                        updated_at_ms: row.get(10)?,
                        project_metadata_indexed: row.get::<_, i64>(11)? != 0,
                        turns: Vec::new(),
                        events: Vec::new(),
                    },
                ))
            })
            .map_err(|error| format!("无法查询 Session 索引：{error}"))?;
        for row in rows {
            let (file_id, file) = row.map_err(|error| format!("无法读取 Session 索引：{error}"))?;
            paths_by_id.insert(file_id, file.path.clone());
            files.insert(file.path.clone(), file);
        }
    }
    {
        let mut statement = connection
            .prepare(
                "SELECT session_file_id, turn_id, started_at_ms, updated_at_ms
                 FROM usage_session_turns
                 ORDER BY session_file_id, started_at_ms, turn_id",
            )
            .map_err(|error| format!("无法准备 Turn 索引查询：{error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    UsageIndexTurn {
                        id: row.get(1)?,
                        started_at_ms: row.get(2)?,
                        updated_at_ms: row.get(3)?,
                    },
                ))
            })
            .map_err(|error| format!("无法查询 Turn 索引：{error}"))?;
        for row in rows {
            let (session_file_id, turn) =
                row.map_err(|error| format!("无法读取 Turn 索引：{error}"))?;
            if let Some(session_path) = paths_by_id.get(&session_file_id)
                && let Some(file) = files.get_mut(session_path)
            {
                file.turns.push(turn);
            }
        }
    }
    {
        let mut statement = connection
            .prepare(
                "SELECT session_file_id, event_index, timestamp_ms, date, turn_id, model,
                        input_tokens, cached_input_tokens, cache_write_input_tokens,
                        output_tokens, total_tokens
                 FROM usage_token_events
                 ORDER BY session_file_id, event_index",
            )
            .map_err(|error| format!("无法准备 Token 事件查询：{error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    UsageIndexEvent {
                        event_index: position(row.get(1)?),
                        timestamp_ms: row.get(2)?,
                        date: row.get(3)?,
                        turn_id: row.get(4)?,
                        model: row.get(5)?,
                        input_tokens: unsigned(row.get(6)?),
                        cached_input_tokens: unsigned(row.get(7)?),
                        cache_write_input_tokens: unsigned(row.get(8)?),
                        output_tokens: unsigned(row.get(9)?),
                        total_tokens: unsigned(row.get(10)?),
                    },
                ))
            })
            .map_err(|error| format!("无法查询 Token 事件：{error}"))?;
        for row in rows {
            let (session_file_id, event) =
                row.map_err(|error| format!("无法读取 Token 事件：{error}"))?;
            if let Some(session_path) = paths_by_id.get(&session_file_id)
                && let Some(file) = files.get_mut(session_path)
            {
                file.events.push(event);
            }
        }
    }
    Ok(files.into_values().collect())
}

pub fn write_usage_index(path: &Path, files: &[UsageIndexFile]) -> Result<(), String> {
    let mut connection = open(path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("无法开始保存 Session 索引：{error}"))?;
    let current_paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();
    {
        let mut statement = transaction
            .prepare("SELECT session_path FROM usage_session_files")
            .map_err(|error| format!("无法读取旧 Session 索引：{error}"))?;
        let old_paths = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("无法查询旧 Session 索引：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("无法读取旧 Session 路径：{error}"))?;
        for old_path in old_paths {
            if !current_paths.contains(old_path.as_str()) {
                transaction
                    .execute(
                        "DELETE FROM usage_session_files WHERE session_path = ?1",
                        [old_path],
                    )
                    .map_err(|error| format!("无法删除过期 Session：{error}"))?;
            }
        }
    }
    for file in files {
        let stored = transaction
            .query_row(
                "SELECT id, length, modified_ms FROM usage_session_files WHERE session_path = ?1",
                [&file.path],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("无法检查 Session {}：{error}", file.path))?;
        let length = integer(file.length, "Session 文件长度")?;
        let modified_ms = file.modified_ms.to_string();
        if stored
            .as_ref()
            .is_some_and(|(_, stored_length, stored_modified)| {
                *stored_length == length && *stored_modified == modified_ms
            })
        {
            continue;
        }
        transaction
            .execute(
                "INSERT INTO usage_session_files(
                    session_path, length, modified_ms, malformed_lines, session_id,
                    parent_id, forked_at_ms, cwd, started_at_ms, updated_at_ms,
                    project_metadata_indexed
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(session_path) DO UPDATE SET
                    length = excluded.length,
                    modified_ms = excluded.modified_ms,
                    malformed_lines = excluded.malformed_lines,
                    session_id = excluded.session_id,
                    parent_id = excluded.parent_id,
                    forked_at_ms = excluded.forked_at_ms,
                    cwd = excluded.cwd,
                    started_at_ms = excluded.started_at_ms,
                    updated_at_ms = excluded.updated_at_ms,
                    project_metadata_indexed = excluded.project_metadata_indexed",
                params![
                    file.path,
                    length,
                    modified_ms,
                    count(file.malformed_lines, "损坏行数")?,
                    file.session_id,
                    file.parent_id,
                    file.forked_at_ms,
                    file.cwd,
                    file.started_at_ms,
                    file.updated_at_ms,
                    i64::from(file.project_metadata_indexed),
                ],
            )
            .map_err(|error| format!("无法保存 Session {}：{error}", file.path))?;
        let session_file_id = transaction
            .query_row(
                "SELECT id FROM usage_session_files WHERE session_path = ?1",
                [&file.path],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("无法读取 Session 主键 {}：{error}", file.path))?;
        transaction
            .execute(
                "DELETE FROM usage_session_turns WHERE session_file_id = ?1",
                [session_file_id],
            )
            .map_err(|error| format!("无法更新 Session Turn：{error}"))?;
        transaction
            .execute(
                "DELETE FROM usage_token_events WHERE session_file_id = ?1",
                [session_file_id],
            )
            .map_err(|error| format!("无法更新 Session Token：{error}"))?;
        for turn in &file.turns {
            transaction
                .execute(
                    "INSERT INTO usage_session_turns(
                        session_file_id, turn_id, started_at_ms, updated_at_ms
                     ) VALUES(?1, ?2, ?3, ?4)",
                    params![
                        session_file_id,
                        turn.id,
                        turn.started_at_ms,
                        turn.updated_at_ms
                    ],
                )
                .map_err(|error| format!("无法保存 Turn {}：{error}", turn.id))?;
        }
        for event in &file.events {
            transaction
                .execute(
                    "INSERT INTO usage_token_events(
                        session_file_id, event_index, timestamp_ms, date, turn_id, model,
                        input_tokens, cached_input_tokens, cache_write_input_tokens,
                        output_tokens, total_tokens
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        session_file_id,
                        count(event.event_index, "Token 事件序号")?,
                        event.timestamp_ms,
                        event.date,
                        event.turn_id,
                        event.model,
                        integer(event.input_tokens, "输入 Token")?,
                        integer(event.cached_input_tokens, "缓存 Token")?,
                        integer(event.cache_write_input_tokens, "缓存写入 Token")?,
                        integer(event.output_tokens, "输出 Token")?,
                        integer(event.total_tokens, "总 Token")?,
                    ],
                )
                .map_err(|error| format!("无法保存 Token 事件：{error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交 Session 索引：{error}"))
}

pub fn write_trace_mirror(path: &Path, sessions: &[TraceMirrorSession]) -> Result<(), String> {
    let mut connection = open(path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("无法开始保存 Trace 关系索引：{error}"))?;
    let current_paths = sessions
        .iter()
        .map(|session| session.session_path.as_str())
        .collect::<HashSet<_>>();
    {
        let mut statement = transaction
            .prepare("SELECT session_path FROM trace_sessions")
            .map_err(|error| format!("无法读取旧 Trace Session：{error}"))?;
        let old_paths = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("无法查询旧 Trace Session：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("无法读取旧 Trace 路径：{error}"))?;
        for old_path in old_paths {
            if !current_paths.contains(old_path.as_str()) {
                transaction
                    .execute(
                        "DELETE FROM trace_sessions WHERE session_path = ?1",
                        [old_path],
                    )
                    .map_err(|error| format!("无法删除过期 Trace Session：{error}"))?;
            }
        }
    }
    for session in sessions {
        let stored = transaction
            .query_row(
                "SELECT length, modified_ms FROM trace_sessions WHERE session_path = ?1",
                [&session.session_path],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| format!("无法检查 Trace Session：{error}"))?;
        let length = integer(session.length, "Trace 文件长度")?;
        let modified_ms = session.modified_ms.to_string();
        let content_unchanged = stored.as_ref() == Some(&(length, modified_ms.clone()));
        transaction
            .execute(
                "INSERT INTO trace_sessions(
                    session_path, length, modified_ms, session_id, parent_id, cwd,
                    conversation_name, official_status, started_at, updated_at, model,
                    malformed_lines
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(session_path) DO UPDATE SET
                    length = excluded.length,
                    modified_ms = excluded.modified_ms,
                    session_id = excluded.session_id,
                    parent_id = excluded.parent_id,
                    cwd = excluded.cwd,
                    conversation_name = excluded.conversation_name,
                    official_status = excluded.official_status,
                    started_at = excluded.started_at,
                    updated_at = excluded.updated_at,
                    model = excluded.model,
                    malformed_lines = excluded.malformed_lines",
                params![
                    session.session_path,
                    length,
                    modified_ms,
                    session.session_id,
                    session.parent_id,
                    session.cwd,
                    session.conversation_name,
                    session.official_status,
                    session.started_at,
                    session.updated_at,
                    session.model,
                    count(session.malformed_lines, "Trace 损坏行数")?,
                ],
            )
            .map_err(|error| format!("无法保存 Trace Session：{error}"))?;
        if content_unchanged {
            continue;
        }
        for table in [
            "trace_turns",
            "trace_tool_events",
            "trace_phase_events",
            "trace_usage_events",
        ] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE session_path = ?1"),
                    [&session.session_path],
                )
                .map_err(|error| format!("无法更新 {table}：{error}"))?;
        }
        for turn in &session.turns {
            transaction
                .execute(
                    "INSERT INTO trace_turns(
                        session_path, turn_id, model, reasoning_effort, summary_mode,
                        started_source_order, started_at_ms, completed_source_order,
                        completed_at_ms, duration_ms, structured_failures, context_compactions
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        session.session_path,
                        turn.turn_id,
                        turn.model,
                        turn.reasoning_effort,
                        turn.summary_mode,
                        turn.started_source_order
                            .map(|value| count(value, "起始行"))
                            .transpose()?,
                        turn.started_at_ms,
                        turn.completed_source_order
                            .map(|value| count(value, "结束行"))
                            .transpose()?,
                        turn.completed_at_ms,
                        turn.duration_ms
                            .map(|value| integer(value, "Turn 耗时"))
                            .transpose()?,
                        count(turn.structured_failures, "失败数")?,
                        count(turn.context_compactions, "压缩数")?,
                    ],
                )
                .map_err(|error| format!("无法保存 Trace Turn {}：{error}", turn.turn_id))?;
        }
        for event in &session.tool_events {
            transaction
                .execute(
                    "INSERT INTO trace_tool_events(
                        session_path, turn_id, source_order, completed_source_order,
                        execution_completed_source_order, timestamp_ms, completed_at_ms,
                        execution_completed_at_ms, call_id, source_type,
                        execution_end_source_type, result_source_type, name, category,
                        server, subject, detail, arguments_json, result_json, repeated,
                        failed, output_bytes, exit_code
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                              ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                              ?21, ?22, ?23)",
                    params![
                        session.session_path,
                        event.turn_id,
                        count(event.source_order, "工具事件行")?,
                        event
                            .completed_source_order
                            .map(|value| count(value, "工具结束行"))
                            .transpose()?,
                        event
                            .execution_completed_source_order
                            .map(|value| count(value, "执行结束行"))
                            .transpose()?,
                        event.timestamp_ms,
                        event.completed_at_ms,
                        event.execution_completed_at_ms,
                        event.call_id,
                        event.source_type,
                        event.execution_end_source_type,
                        event.result_source_type,
                        event.name,
                        event.category,
                        event.server,
                        event.subject,
                        event.detail,
                        event.arguments_json,
                        event.result_json,
                        i64::from(event.repeated),
                        i64::from(event.failed),
                        integer(event.output_bytes, "工具输出字节")?,
                        event.exit_code,
                    ],
                )
                .map_err(|error| format!("无法保存 Trace 工具事件：{error}"))?;
        }
        for event in &session.phase_events {
            transaction
                .execute(
                    "INSERT INTO trace_phase_events(
                        session_path, turn_id, source_order, source_end_order, timestamp_ms,
                        phase, source_type, role, content_bytes, encrypted_bytes
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        session.session_path,
                        event.turn_id,
                        count(event.source_order, "阶段事件行")?,
                        event
                            .source_end_order
                            .map(|value| count(value, "阶段结束行"))
                            .transpose()?,
                        event.timestamp_ms,
                        event.phase,
                        event.source_type,
                        event.role,
                        integer(event.content_bytes, "内容字节")?,
                        integer(event.encrypted_bytes, "加密内容字节")?,
                    ],
                )
                .map_err(|error| format!("无法保存 Trace 阶段事件：{error}"))?;
        }
        for event in &session.usage_events {
            transaction
                .execute(
                    "INSERT INTO trace_usage_events(
                        session_path, source_order, turn_id, timestamp_ms, model,
                        input_tokens, cached_input_tokens, output_tokens,
                        reasoning_output_tokens, total_tokens, context_window
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        session.session_path,
                        count(event.source_order, "用量事件行")?,
                        event.turn_id,
                        event.timestamp_ms,
                        event.model,
                        integer(event.input_tokens, "输入 Token")?,
                        integer(event.cached_input_tokens, "缓存 Token")?,
                        integer(event.output_tokens, "输出 Token")?,
                        integer(event.reasoning_output_tokens, "推理 Token")?,
                        integer(event.total_tokens, "总 Token")?,
                        event
                            .context_window
                            .map(|value| integer(value, "上下文窗口"))
                            .transpose()?,
                    ],
                )
                .map_err(|error| format!("无法保存 Trace 用量事件：{error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交 Trace 关系索引：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Fixture {
        value: u64,
    }

    fn database_path(name: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("codex-xray-{name}-{suffix}.sqlite"))
    }

    #[test]
    fn cache_round_trip() {
        let path = database_path("cache");
        write_cache(&path, "usage", &Fixture { value: 42 }).expect("write");
        let value = read_cache::<Fixture>(&path, "usage").expect("read");
        assert_eq!(value, Some(Fixture { value: 42 }));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn index_replaces_stale_rows_and_resets_on_schema_change() {
        let path = database_path("index");
        write_index_entries(
            &path,
            "cost",
            1,
            &[
                ("a".to_string(), b"one".to_vec()),
                ("b".to_string(), b"two".to_vec()),
            ],
        )
        .expect("first write");
        write_index_entries(&path, "cost", 1, &[("b".to_string(), b"changed".to_vec())])
            .expect("second write");
        assert_eq!(
            read_index_entries(&path, "cost", 1).expect("read"),
            vec![("b".to_string(), b"changed".to_vec())]
        );
        assert!(
            read_index_entries(&path, "cost", 2)
                .expect("schema reset")
                .is_empty()
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn usage_relations_round_trip_and_remove_stale_sessions() {
        let path = database_path("usage-relations");
        let fixture = UsageIndexFile {
            path: "/tmp/session.jsonl".to_string(),
            length: 512,
            modified_ms: 1_234,
            session_id: Some("session-1".to_string()),
            cwd: Some("/tmp/project".to_string()),
            project_metadata_indexed: true,
            turns: vec![UsageIndexTurn {
                id: "turn-1".to_string(),
                started_at_ms: Some(10),
                updated_at_ms: Some(20),
            }],
            events: vec![UsageIndexEvent {
                event_index: 0,
                timestamp_ms: 20,
                date: "2026-07-29".to_string(),
                turn_id: "turn-1".to_string(),
                model: "gpt-test".to_string(),
                input_tokens: 100,
                cached_input_tokens: 80,
                output_tokens: 10,
                total_tokens: 110,
                ..UsageIndexEvent::default()
            }],
            ..UsageIndexFile::default()
        };
        write_usage_index(&path, std::slice::from_ref(&fixture)).expect("write");
        let restored = read_usage_index(&path).expect("read");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].turns[0].id, "turn-1");
        assert_eq!(restored[0].events[0].cached_input_tokens, 80);
        write_usage_index(&path, &[]).expect("remove stale");
        assert!(read_usage_index(&path).expect("empty").is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn trace_mirror_persists_queryable_flow_rows() {
        let path = database_path("trace-relations");
        write_trace_mirror(
            &path,
            &[TraceMirrorSession {
                session_path: "/tmp/session.jsonl".to_string(),
                length: 42,
                modified_ms: 99,
                session_id: Some("session-1".to_string()),
                turns: vec![TraceMirrorTurn {
                    turn_id: "turn-1".to_string(),
                    model: "gpt-test".to_string(),
                    ..TraceMirrorTurn::default()
                }],
                tool_events: vec![TraceMirrorToolEvent {
                    turn_id: "turn-1".to_string(),
                    source_order: 12,
                    timestamp_ms: 100,
                    name: "web.run".to_string(),
                    category: "web".to_string(),
                    source_type: "response_item.function_call".to_string(),
                    ..TraceMirrorToolEvent::default()
                }],
                phase_events: vec![TraceMirrorPhaseEvent {
                    turn_id: "turn-1".to_string(),
                    source_order: 9,
                    timestamp_ms: 90,
                    phase: "user_prompt".to_string(),
                    source_type: "event_msg.user_message".to_string(),
                    ..TraceMirrorPhaseEvent::default()
                }],
                usage_events: vec![TraceMirrorUsageEvent {
                    turn_id: "turn-1".to_string(),
                    source_order: 16,
                    timestamp_ms: 110,
                    model: "gpt-test".to_string(),
                    total_tokens: 123,
                    ..TraceMirrorUsageEvent::default()
                }],
                ..TraceMirrorSession::default()
            }],
        )
        .expect("write trace mirror");
        let connection = open(&path).expect("open");
        let counts = connection
            .query_row(
                "SELECT
                    (SELECT count(*) FROM trace_sessions),
                    (SELECT count(*) FROM trace_turns),
                    (SELECT count(*) FROM trace_tool_events),
                    (SELECT count(*) FROM trace_phase_events),
                    (SELECT count(*) FROM trace_usage_events)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .expect("counts");
        assert_eq!(counts, (1, 1, 1, 1, 1));
        let _ = fs::remove_file(path);
    }
}
