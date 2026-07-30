export type AccountInfo = {
  account_type: string;
  plan_type: string | null;
};

export type RateLimitWindow = {
  used_percent: number;
  window_duration_mins: number | null;
  resets_at: number | null;
};

export type CreditsInfo = {
  has_credits: boolean;
  unlimited: boolean;
  balance: string | null;
};

export type RateLimit = {
  limit_id: string;
  limit_name: string | null;
  plan_type: string | null;
  primary: RateLimitWindow | null;
  secondary: RateLimitWindow | null;
  credits: CreditsInfo | null;
  spend_control_reached: boolean | null;
  rate_limit_reached_type: string | null;
};

export type UsageSummary = {
  lifetime_tokens: number | null;
  peak_daily_tokens: number | null;
  longest_running_turn_sec: number | null;
  current_streak_days: number | null;
  longest_streak_days: number | null;
};

export type DailyUsage = {
  start_date: string;
  tokens: number;
};

export type LocalTodayUsage = {
  date: string;
  input_tokens: number;
  cached_input_tokens: number;
  cache_write_input_tokens: number;
  output_tokens: number;
  reasoning_output_tokens: number;
  total_tokens: number;
  uncached_input_tokens: number;
  cache_hit_percent: number;
  files_scanned: number;
  token_events: number;
  duplicate_events_skipped: number;
  malformed_lines_skipped: number;
  latest_event_at: string | null;
  estimated_cost_usd: number;
  uncached_input_cost_usd: number;
  cached_input_cost_usd: number;
  output_cost_usd: number;
  cache_savings_usd: number;
  priced_tokens: number;
  unpriced_tokens: number;
  models: string[];
};

export type DailyCostEstimate = {
  date: string;
  input_tokens: number;
  cached_input_tokens: number;
  cache_write_input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  priced_tokens: number;
  unpriced_tokens: number;
  cost_usd: number;
  models: ModelCostEstimate[];
};

export type ModelCostEstimate = {
  model: string;
  input_tokens: number;
  cached_input_tokens: number;
  cache_write_input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  cost_usd: number;
  cache_savings_usd: number;
  priced: boolean;
};

export type PricingOverride = {
  model: string;
  input_per_million: number;
  cached_input_per_million: number;
  output_per_million: number;
};

export type PricingRateDefinition = PricingOverride & {
  has_long_context_tier: boolean;
};

export type PricingVersion = {
  effective_from: string;
  created_at: string;
  overrides: PricingOverride[];
};

export type PricingConfigSnapshot = {
  config_path: string;
  updated_at: string | null;
  defaults_updated_at: string;
  overrides: PricingOverride[];
  versions: PricingVersion[];
  defaults: PricingRateDefinition[];
};

export type CostEstimateSnapshot = {
  report_schema_version: number;
  generated_at: string;
  pricing_basis: string;
  pricing_updated_at: string;
  total_cost_usd: number;
  uncached_input_cost_usd: number;
  cached_input_cost_usd: number;
  output_cost_usd: number;
  cache_savings_usd: number;
  total_tokens: number;
  priced_tokens: number;
  unpriced_tokens: number;
  coverage_start: string | null;
  coverage_end: string | null;
  files_indexed: number;
  files_scanned: number;
  files_reused: number;
  token_events: number;
  duplicate_events_skipped: number;
  elapsed_ms: number;
  daily: DailyCostEstimate[];
  models: ModelCostEstimate[];
  warnings: string[];
};

export type ProjectUsageTurn = {
  id: string;
  sequence: number;
  started_at: string | null;
  updated_at: string | null;
  models: string[];
  input_tokens: number;
  cached_input_tokens: number;
  cache_write_input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  priced_tokens: number;
  unpriced_tokens: number;
  cost_usd: number;
};

export type ProjectUsageConversation = {
  id: string;
  title: string | null;
  session_path: string;
  updated_at: string | null;
  is_subagent: boolean;
  parent_id: string | null;
  models: string[];
  turns: number;
  turns_indexed: boolean;
  input_tokens: number;
  cached_input_tokens: number;
  cache_write_input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  priced_tokens: number;
  unpriced_tokens: number;
  cost_usd: number;
  turn_rows: ProjectUsageTurn[];
};

export type ProjectUsageProject = {
  name: string;
  path: string;
  updated_at: string | null;
  sessions: number;
  turns: number;
  turn_sessions_indexed: number;
  input_tokens: number;
  cached_input_tokens: number;
  cache_write_input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  priced_tokens: number;
  unpriced_tokens: number;
  cost_usd: number;
  conversations: ProjectUsageConversation[];
};

export type ProjectUsageSnapshot = {
  report_schema_version: number;
  generated_at: string;
  pricing_updated_at: string;
  files_indexed: number;
  sessions: number;
  turns: number;
  turn_sessions_indexed: number;
  input_tokens: number;
  cached_input_tokens: number;
  cache_write_input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  priced_tokens: number;
  unpriced_tokens: number;
  cost_usd: number;
  projects: ProjectUsageProject[];
  warnings: string[];
};

export type ProjectTurnUsageDetail = {
  session_id: string;
  generated_at: string;
  turns: ProjectUsageTurn[];
};

export type UsageSnapshot = {
  fetched_at: string;
  codex_version: string;
  account: AccountInfo | null;
  current_limit_id: string | null;
  rate_limits: RateLimit[];
  summary: UsageSummary | null;
  daily_usage: DailyUsage[];
  local_today: LocalTodayUsage | null;
  warnings: string[];
};

export type ProviderDefinition = {
  id: string;
  name: string;
  base_url: string | null;
  env_key: string | null;
  env_available: boolean;
  credential_source:
    | "builtin"
    | "keychain"
    | "environment"
    | "command"
    | "none";
  credential_available: boolean;
  auth_command: string | null;
  auth_args: string[];
  wire_api: string;
  protocol: "responses" | "chat_completions";
  context_window: number | null;
  builtin: boolean;
  compatibility:
    | "responses"
    | "chat_bridge"
    | "chat_only"
    | "unsupported_wire_api";
};

export type ProviderSnapshot = {
  fetched_at: string;
  config_path: string;
  version: string | null;
  active_provider: string;
  active_model: string | null;
  models: ModelOption[];
  providers: ProviderDefinition[];
  restore_available: boolean;
  warnings: string[];
};

export type ModelOption = {
  id: string;
  display_name: string;
  default_reasoning_effort: string | null;
  supported_reasoning_efforts: string[];
  supports_personality: boolean;
  is_default: boolean;
};

export type ProviderApplyRequest = {
  provider_id: string;
  model: string;
  name: string | null;
  base_url: string | null;
  env_key: string | null;
  credential_mode: "keychain" | "environment" | "none" | null;
  api_key: string | null;
  protocol: "responses" | "chat_completions";
  context_window: number;
  expected_version: string | null;
};

export type ProviderTestResult = {
  success: boolean;
  check_kind:
    | "responses_request"
    | "chat_completions_request"
    | "codex_model_catalog";
  provider_id: string;
  model: string;
  endpoint: string | null;
  latency_ms: number;
  http_status: number | null;
  message: string;
};

export type ChatBridgeStatus = {
  running: boolean;
  base_url: string;
  configured_providers: number;
  last_error: string | null;
};

export type CodexSettings = {
  model_reasoning_effort: string | null;
  plan_mode_reasoning_effort: string | null;
  model_reasoning_summary: string | null;
  model_verbosity: string | null;
  personality: string | null;
  approval_policy: string;
  approvals_reviewer: string;
  sandbox_mode: string;
  workspace_network_access: boolean;
  web_search: string;
  history_persistence: string;
  auto_compact_token_limit: number | null;
  auto_compact_scope: string;
  memories_enabled: boolean;
  memories_use: boolean;
  memories_generate: boolean;
  memories_disable_on_external_context: boolean;
  multi_agent_enabled: boolean;
  goals_enabled: boolean;
  hooks_enabled: boolean;
  unified_exec_enabled: boolean;
  fast_mode_enabled: boolean;
  apps_enabled: boolean;
  hide_agent_reasoning: boolean;
  show_raw_agent_reasoning: boolean;
};

export type SettingsSnapshot = {
  fetched_at: string;
  config_path: string;
  version: string | null;
  settings: CodexSettings;
  restore_available: boolean;
  warnings: string[];
};

export type SettingsApplyRequest = {
  settings: CodexSettings;
  expected_version: string | null;
};

export type EnvironmentPath = {
  label: string;
  path: string;
  exists: boolean;
  item_count: number | null;
};

export type EnvironmentMcpServer = {
  name: string;
  enabled: boolean;
  transport: string;
  target: string | null;
};

export type EnvironmentProvider = {
  id: string;
  name: string;
  model: string | null;
  wire_api: string;
  endpoint: string | null;
  credential_variable: string | null;
  credential_source: string;
  credential_available: boolean;
  compatibility: string;
};

export type EnvironmentSnapshot = {
  fetched_at: string;
  codex_version: string;
  codex_binary: string;
  codex_home: string;
  config_path: string;
  sessions_path: string;
  xray_data_path: string;
  xray_sqlite_path: string;
  storage: {
    database_bytes: number;
    wal_bytes: number;
    journal_mode: string;
    schema_version: number;
    integrity_ok: boolean;
    integrity_message: string;
    foreign_key_violations: number;
    malformed_session_lines: number;
    usage_sessions: number;
    usage_turns: number;
    token_events: number;
    trace_sessions: number;
    trace_turns: number;
    trace_tool_events: number;
  };
  config_version: string | null;
  provider: EnvironmentProvider;
  settings: CodexSettings;
  mcp_servers: EnvironmentMcpServer[];
  extension_paths: EnvironmentPath[];
  warnings: string[];
};

export type ExtensionCategoryUsage = {
  category: string;
  calls: number;
  failures: number;
  repeated_calls: number;
  timed_calls: number;
  duration_ms: number;
  output_bytes: number;
  unique_items: number;
};

export type ExtensionUsageItem = {
  category: string;
  name: string;
  server: string | null;
  calls: number;
  failures: number;
  repeated_calls: number;
  timed_calls: number;
  duration_ms: number;
  average_duration_ms: number | null;
  output_bytes: number;
  projects: number;
  sessions: number;
  turns: number;
  last_used_at: string | null;
  occurrences: ExtensionUsageOccurrence[];
};

export type ExtensionUsageOccurrence = {
  project: string;
  session_id: string;
  turn_id: string;
  call_id: string | null;
  used_at: string | null;
  failed: boolean;
};

export type ExtensionUsageSnapshot = {
  generated_at: string;
  analyzed_sessions: number;
  current_sessions: number;
  stale_sessions: number;
  projects: number;
  turns: number;
  calls: number;
  failures: number;
  repeated_calls: number;
  timed_calls: number;
  duration_ms: number;
  output_bytes: number;
  categories: ExtensionCategoryUsage[];
  items: ExtensionUsageItem[];
  warnings: string[];
};

export type TraceInsight = {
  kind:
    | "context_growth"
    | "low_cache_hit"
    | "repeated_file_read"
    | "tool_failure"
    | "repeated_tool_call"
    | "large_tool_output"
    | "context_compaction"
    | "high_cost_turn"
    | "subagent_spend";
  severity: "high" | "medium" | "low" | "info";
  value: number;
  subject: string | null;
};

export type TraceSessionSummary = {
  id: string;
  conversation_name: string | null;
  project: string;
  project_path: string;
  session_path: string | null;
  analysis_state: "ready" | "stale" | "not_analyzed";
  source: string;
  model: string;
  started_at: string | null;
  updated_at: string | null;
  status:
    | "running"
    | "waiting_approval"
    | "waiting_input"
    | "completed"
    | "failed"
    | "interrupted"
    | "unknown";
  status_source: "app_server" | "local_events" | "unknown";
  is_subagent: boolean;
  parent_id: string | null;
  duration_ms: number | null;
  turns: number;
  tool_calls: number;
  failed_tool_calls: number;
  repeated_tool_calls: number;
  repeated_reads: number;
  context_compactions: number;
  large_tool_outputs: number;
  input_tokens: number;
  cached_input_tokens: number;
  uncached_input_tokens: number;
  output_tokens: number;
  reasoning_output_tokens: number;
  total_tokens: number;
  cache_hit_percent: number;
  context_growth_tokens: number;
  estimated_cost_usd: number;
  high_cost_turns: number;
  issue_score: number;
  insights: TraceInsight[];
};

export type TraceTotals = {
  sessions: number;
  running_sessions: number;
  subagent_sessions: number;
  turns: number;
  tool_calls: number;
  failed_tool_calls: number;
  repeated_tool_calls: number;
  repeated_reads: number;
  context_compactions: number;
  large_tool_outputs: number;
  input_tokens: number;
  cached_input_tokens: number;
  uncached_input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  subagent_tokens: number;
  estimated_cost_usd: number;
  subagent_cost_usd: number;
  cache_hit_percent: number;
};

export type TraceSnapshot = {
  generated_at: string;
  files_indexed: number;
  files_scanned: number;
  files_reused: number;
  official_threads_matched: number;
  elapsed_ms: number;
  coverage_start: string | null;
  coverage_end: string | null;
  totals: TraceTotals;
  sessions: TraceSessionSummary[];
  warnings: string[];
};

export type TraceTimelineEvent = {
  source_order: number;
  source_end_order: number | null;
  timestamp: string | null;
  completed_at: string | null;
  execution_completed_at: string | null;
  kind:
    | "started"
    | "completed"
    | "tokens"
    | "phase"
    | "tool_request"
    | "tool_execution"
    | "tool_result"
    | "compaction";
  category:
    | "model"
    | "usage"
    | "input"
    | "mcp"
    | "cli"
    | "skill"
    | "file"
    | "browser"
    | "automation"
    | "agent"
    | "context"
    | "lifecycle"
    | "tool";
  label: string;
  sequence: number | null;
  call_id: string | null;
  source_type: string | null;
  execution_end_source_type: string | null;
  result_source_type: string | null;
  server: string | null;
  subject: string | null;
  detail: string | null;
  arguments: TraceEventField[];
  arguments_json: string | null;
  result_fields: TraceEventField[];
  result_json: string | null;
  content: string | null;
  status: "info" | "success" | "failed" | "pending" | "repeated" | "warning";
  input_tokens: number;
  cached_input_tokens: number;
  output_tokens: number;
  reasoning_output_tokens: number;
  total_tokens: number;
  context_window: number | null;
  context_delta_tokens: number | null;
  cache_hit_percent: number | null;
  estimated_cost_usd: number | null;
  content_parts: number;
  content_bytes: number;
  summary_parts: number;
  encrypted_bytes: number;
  duration_ms: number | null;
  output_bytes: number;
  exit_code: number | null;
  repeated: boolean;
};

export type TraceEventField = {
  key: string;
  value: string;
};

export type TraceTurnSummary = {
  id: string;
  sequence: number;
  model: string;
  reasoning_effort: string | null;
  summary_mode: string | null;
  status: "running" | "completed" | "failed" | "interrupted" | "unknown";
  started_at: string | null;
  completed_at: string | null;
  duration_ms: number | null;
  input_tokens: number;
  cached_input_tokens: number;
  uncached_input_tokens: number;
  output_tokens: number;
  reasoning_output_tokens: number;
  total_tokens: number;
  cache_hit_percent: number;
  peak_input_tokens: number;
  context_window: number | null;
  context_utilization_percent: number | null;
  context_growth_tokens: number;
  estimated_cost_usd: number;
  tool_calls: number;
  failed_tool_calls: number;
  repeated_tool_calls: number;
  repeated_reads: number;
  context_compactions: number;
  large_tool_outputs: number;
  large_tool_output_bytes: number;
  issue_score: number;
  insights: TraceInsight[];
  timeline: TraceTimelineEvent[];
  timeline_events_omitted: number;
};

export type TraceToolAggregate = {
  name: string;
  calls: number;
  failures: number;
  repeats: number;
  large_outputs: number;
  output_bytes: number;
  subjects: string[];
};

export type TraceSessionDetail = {
  session: TraceSessionSummary;
  flagged_turns: number;
  flagged_tokens: number;
  flagged_cost_usd: number;
  turns: TraceTurnSummary[];
  tools: TraceToolAggregate[];
};
