# Codex X-Ray Data Sources and Metric Definitions

Version: 0.1.0
Rule: official values are shown as returned; local values are explicitly labeled as derived or estimated. Missing data is never turned into a fabricated precise number.

## Data paths

Codex X-Ray keeps four kinds of data separate:

1. **Official account path:** it starts the installed `codex app-server`, communicates over local stdio JSONL with JSON-RPC, and calls read-only account methods. App Server is the official Codex interface for rich clients.
2. **Official thread metadata:** read-only `thread/list`; Codex X-Ray retains only thread ID, user-facing name, `cwd`, and structured activity status. It never writes `preview` or message text into its index.
3. **Local session path:** it read-only scans structured events in `$CODEX_HOME/sessions/**/*.jsonl`. The default `CODEX_HOME` is `~/.codex`. Codex X-Ray never modifies these files.
4. **Codex configuration path:** the official App Server `config/read` method supplies the merged configuration and user-layer version, while `model/list` supplies the current official model catalog. Codex X-Ray calls `config/batchWrite` only after the user previews and confirms selected user-setting changes.

Codex X-Ray assigns its App Server an independent `sqlite_home`, so it does not contend with the Codex App runtime database. `thread/list` can repair Codex X-Ray's own metadata copy from shared session records without writing the Codex App database or starting model turns.

## Environment diagnosis sources

The Environment page combines three read-only fact sets:

- The CLI/App Server launched by Codex X-Ray supplies the executable path, version, and merged configuration returned by official `config/read`.
- `$CODEX_HOME`, `config.toml`, `sessions`, Skills, and the plugin cache are checked only for path existence and top-level directory counts; extension contents are not read.
- MCP entries expose only their configured name, enabled state, transport, and a sanitized target summary. Headers, environment-variable values, and credentials are never displayed.

The “analysis database” shown by Environment is Codex X-Ray's own `codex-xray.sqlite`, not the Codex App SQLite state. X-Ray's App Server uses a separate `sqlite_home` for its runtime state. Environment diagnosis never reads `auth.json` and does not claim to attach to another Codex App process.

## Provider configuration and Chat compatibility

Current Codex custom Providers use `wire_api = "responses"`. “OpenAI compatible” is not treated as sufficient: a route is marked native or hosted Responses only when the vendor documents `/responses`, streaming events, and function tools.

| Preset | Route | Base URL | Environment variable | Note |
|---|---|---|---|---|
| OpenAI | Built into Codex | Built in | Codex sign-in | Does not add a custom Provider |
| Alibaba Model Studio / Qwen | Native Responses | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` | The existing public endpoint remains available; workspace endpoints are recommended |
| Volcengine Ark / Doubao | Native Responses | `https://ark.cn-beijing.volces.com/api/v3` | `ARK_API_KEY` | The model ID must match Ark |
| Zhipu GLM | Local Chat bridge | `https://open.bigmodel.cn/api/paas/v4` | `ZHIPUAI_API_KEY` | X-Ray translates Codex Responses traffic to the documented `/chat/completions` endpoint |
| DeepSeek | Local Chat bridge | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` | X-Ray translates streaming text and function-tool traffic locally |
| Kimi | Local Chat bridge | `https://api.moonshot.cn/v1` | `MOONSHOT_API_KEY` | X-Ray translates the official Chat API's streaming text and function-tool traffic locally |
| MiniMax | Native Responses | `https://api.minimaxi.com/v1` | `MINIMAX_API_KEY` | MiniMax publishes a Codex desktop setup guide |
| Xiaomi MiMo | Native Responses | `https://api.xiaomimimo.com/v1` | `MIMO_API_KEY` | Xiaomi publishes Codex and Responses API setup guidance |
| Custom API | Native Responses or local Chat bridge | User supplied | User supplied | Other providers are available through a custom profile |

The installed Codex currently accepts `wire_api = "responses"` for custom Providers. For a Chat preset, X-Ray therefore writes a loopback Responses endpoint into Codex configuration and keeps the real vendor URL in X-Ray's non-secret bridge registry. Codex also requests a model catalog from that endpoint; X-Ray returns the configured model ID and context-window metadata so Codex can schedule compaction correctly.

The bridge translates message text, streaming output, function definitions, tool requests, tool results, and usage counters. It intentionally drops Responses-only built-in tools and metadata that Chat Completions cannot represent, including native Web Search, encrypted reasoning, and server-side compaction. The bridge is available only while X-Ray is running; closing X-Ray stops it.

### Making an environment variable visible to Codex desktop

The Provider page writes the environment-variable name only. A macOS app launched from Finder does not automatically inherit a shell `export`. To keep the actual key out of shell history, run the following in zsh (replace the variable name with the one shown by the preset), then fully quit and reopen Codex and Codex X-Ray:

```zsh
read -s "provider_key?API Key: "
launchctl setenv DASHSCOPE_API_KEY "$provider_key"
unset provider_key
```

Do not put the real key in project files or screenshots. Use `launchctl unsetenv DASHSCOPE_API_KEY` when it is no longer needed. A Codex CLI process launched from the same terminal can use a regular environment variable.

### Write and recovery boundary

1. The page initially calls `config/read` only and never reads `auth.json`. Credential fields in the configuration result are not extracted, returned to the frontend, or stored.
2. Direct API-key entry stores the secret in a user-only file under `~/.codex/codex-xray/credentials/`; `config.toml` receives only a command-backed credential helper. Environment-variable authentication remains available. **Test connection** reads the selected credential transiently in the Rust backend and sends one minimal request using the selected protocol. The key is not returned to the frontend, logged, written to SQLite, or placed in process arguments.
3. Preview shows the current and target Provider, model, and endpoint without writing.
4. Confirm calls `config/batchWrite` with the user-layer version returned by `config/read` to prevent silent concurrent overwrites.
5. The previous Provider, model, and non-sensitive endpoint definition are stored in the Codex X-Ray app-data directory as a reversible restore point.
6. Chat bridge mappings store only provider ID, upstream URL, authentication mode, environment-variable name, model ID, and context-window size. They contain no key. Provider changes do not start model turns or consume Codex allowance. A third-party connection test sends one minimal request and may incur a very small API charge. New tasks or a Codex restart use the new configuration.

## Visual Codex control center

The control center is not a raw `config.toml` key/value editor. It groups common settings into Model Behavior, Permissions & Approvals, Context & Memory, and Tools & Capabilities. Every setting includes a plain-language purpose, recommended value, scope, and risk note.

- **Model behavior:** reasoning effort, plan reasoning, reasoning summaries, answer verbosity, personality, and hidden/raw reasoning display.
- **Permissions and approvals:** sandbox mode, approval policy, approval reviewer, and workspace network access. `danger-full-access` and `never` receive explicit risk treatment.
- **Context and Memory:** history persistence, auto-compaction threshold and token-count scope, the Memory feature, memory generation/use, and disabling Memory under external context.
- **Tools and capabilities:** Web Search, Apps, subagents, Goals, Hooks, unified execution, and Fast mode.

Unset settings remain “follow Codex default.” Saving `null` clears a user-layer override instead of writing a guessed default. Only changed keys are submitted, with the version returned by `config/read` to prevent silent concurrent overwrites. The complete diff is shown before save, and the previous non-sensitive values of those keys become a one-level restore point. Restore also uses official `config/batchWrite`.

Some settings affect only new tasks or the next turn. The UI follows Codex configuration semantics and does not promise universal hot switching.

## Usage metrics

| UI metric | Type | Source / field | Definition | Freshness and limitation |
|---|---|---|---|---|
| Plan | Official direct | `account/read` → `planType` | Name formatting only; no inference | Depends on App Server response |
| 5-hour quota | Official direct | `account/rateLimits/read` → `primary.usedPercent`, `resetsAt`, `windowDurationMins` | Percentage is shown as returned; remaining = `100 - usedPercent`; countdown = `resetsAt - now` | Missing windows remain missing; unlimited is never invented |
| Weekly quota | Official direct | `account/rateLimits/read` → `secondary.*` | Same as the 5-hour window | Some buckets have no secondary window |
| Credits | Official direct | `account/rateLimits/read` → `credits` | Shows availability, `unlimited`, and `balance` | Credits are neither tokens nor an API bill |
| Tokens today | Local derived, official fallback | Session `token_count`; official daily bucket when no local data exists | `Total = input + output` | Local logs are near-real-time; official daily buckets may lag |
| Input tokens | Local derived | `token_count.input_tokens` | All request input, including cache hits | Only sessions with structured token events are counted |
| Cached input | Local derived | `token_count.cached_input_tokens` | A subset of input; never added to total again | Hit rate = cached input / input |
| Cache write | Local direct | `token_count.cache_write_input_tokens` | Shown only when an event explicitly returns it; never added to total by Codex X-Ray | Most Codex sessions currently omit it, so the column hides automatically |
| Uncached input | Local derived | Input and cached input | `max(input - cached input, 0)` | Highlights context that was processed again |
| Output tokens | Local derived | `token_count.output_tokens` | Added to input for total tokens | Reasoning tokens are a subset, not an extra term |
| Reasoning tokens | Local derived | `reasoning_output_tokens` | Displayed as an output detail | May be absent for some models or versions |
| Lifetime tokens | Official direct | `account/usage/read` → `summary.lifetimeTokens` | Shown as returned | Historical usage, not remaining quota |
| Daily peak | Official direct | `summary.peakDailyTokens` | Shown as returned | Official account statistic |
| Usage streak | Official direct | `currentStreakDays`, `longestStreakDays` | Shown as returned | Calculated by the official service on calendar days |
| Longest task | Official direct | `summary.longestRunningTurnSec` | Formatted as a duration | Not app uptime |
| Daily ledger | Local derived | Local session aggregates by day | Lists fresh input, cache reads, cache writes, output, local total, models, and estimated cost | Only dates with local structured token events are shown |
| Monthly ledger | Local derived | The local daily ledger rolled up by calendar month | Sums local sessions in each month; click a month to expand its per-model detail | The first and current month may be partial |
| Project / conversation / turn ledger | Local derived + official names | Session `session_meta.cwd`, `task_started.turn_id`, `turn_context.model`, and `token_count`; `thread/list.name` | Groups the same token events and prices by full workspace path, Session, and Turn; clicking a conversation or Turn opens its execution trace | Only local sessions with structured token events are listed; the Session ID is used when an official name is unavailable |
| Account-view comparison | Official direct | `account/usage/read` → `dailyUsageBuckets` | Displayed beside the matching local day/month with a delta; never added together | Official buckets and local logs can differ in sync time, coverage, or accounting scope |

Usage is split into five focused reports: Overview, Daily, Monthly, Projects, and Model cost. Overview prioritizes today's exact breakdown, official quota, and the most recent seven days. Daily and monthly reports default to an auditable local-session table and can switch to a trend chart, so exact values never depend on hover alone. The primary columns are **fresh input / cache read / cache write (when present) / output / local total / API-equivalent cost**. Click any day or month to inspect its per-model composition.

Projects does not depend on prior execution inspection. Its first paint reuses Session IDs, models, and numeric usage from the cost index and enriches them with `cwd` and user-visible names from read-only `thread/list`, so listing projects does not rescan the full history. The first time an older conversation is expanded, Codex X-Ray reads only that Session's `task_started`, `turn_context`, and `token_count` records and persists the Turn figures in its own `codex-xray.sqlite`; later opens reuse it. When a conversation or turn opens Trace and the target has not been analyzed, the app analyzes that Session once and then focuses the requested turn. User message text is written to neither index. Project, conversation, and loaded Turn totals use the same branch-replay deduplication and custom prices as the daily and monthly ledgers.

The Usage boot cache, per-file cost index, project Turn data, and Trace file index live in one WAL-backed `codex-xray.sqlite` database. Usage is normalized into `usage_session_files`, `usage_session_turns`, and `usage_token_events`; project, conversation, and Turn reports query the same relations instead of copying a second project-detail dataset. Dates, projects, Sessions, Turns, models, and token classes are directly queryable with SQL. Trace is mirrored into `trace_sessions`, `trace_turns`, `trace_phase_events`, `trace_tool_events`, and `trace_usage_events`; tool request, Codex execution completion, and result write-back keep their own source-line and timestamp boundaries. A per-Session Trace snapshot remains for lossless, fast Timeline reconstruction. Refreshes update only changed Sessions, and original Codex Session JSONL files remain read-only.

### Ledger and account-view rules

1. The daily/monthly primary table uses structured token events from local sessions only, keeping input, cache, output, model, and cost on the same coverage boundary.
2. `Fresh input = max(input - cache read, 0)`.
3. `Local total` uses the total reported by session events. Under the current event shape it normally equals `input + output`; cache reads are already inside input and reasoning tokens are already inside output, so neither is added again.
4. The monthly ledger aggregates only local daily rows in that calendar month. Empty dates and months do not become fabricated zero rows.
5. Official daily buckets remain a separate Account view. They highlight differences in sync time, coverage, or accounting scope; they never replace or add to the local detail.
6. API-equivalent cost is calculated from that same local model detail. Unknown models are explicitly marked Unpriced.

## API-equivalent cost

API-equivalent cost answers: “What would the same model-token mix roughly be worth at published Standard API prices?” It is **not** a ChatGPT/Codex subscription bill and does not represent an actual charge.

Cost is calculated only from local session token details that include model identity, independently of whichever token source the daily/monthly table selected. A period can therefore show official token usage next to a local session cost estimate. If their coverage differs, the pair must not be interpreted as an actual average price per token.

1. Read model, input, cached input, and output tokens from local sessions.
2. `Uncached input = input - cached input`.
3. Apply either Codex X-Ray's bundled public pricing snapshot or the per-model override saved under **Model cost → Pricing**:
   `cost = uncached input × input price + cached input × cached price + output × output price`.
4. Tokens with an unknown model or missing price remain “unpriced”; no generic price is forced onto them.
5. Estimated cache savings = regular input value of cached tokens - cached-input value.

All prices use **USD per 1 million tokens**. The built-in snapshot date is displayed, and default-price updates ship with app updates. Custom rates are stored as effective-dated versions in `pricing-config.json` inside Codex X-Ray's own app-data directory. Each event uses the latest version effective on that event date, so changing today's prices does not rewrite historical months. The file never writes to `~/.codex` or changes real Codex, account, or Provider billing configuration. Saving recalculates daily, monthly, lifetime, model, Session, and Turn cost totals while reusing the incremental token index.

A custom model price is a flat input, cached-input, and output rate. It replaces both the bundled regular and long-context tier for that model. Restoring a known model re-enables its bundled tier; clearing an unknown model returns it to Unpriced. Every amount remains an estimate rather than a claim about an actual bill.

## Execution inspection

Execution inspection reads and displays user messages, assistant messages, and readable summaries on demand from the original Session selected by the user. SQLite stores only the structured aggregates needed to reconstruct real calls; it does not store those message bodies, complete patches, file contents returned by reads, or complete tool output. To support execution learning, it persists bounded and redacted command, script, and argument summaries.

### Session detail hierarchy

The execution workbench separates catalog loading from inspection. Opening the view fully paginates official `thread/list` to build the project/conversation catalog and does not scan every session. A session is parsed only after the user explicitly clicks Inspect:

1. **Project:** exact `thread/list.cwd` is the stable grouping key; the last directory component is the project label.
2. **Conversation:** uses `thread/list.name` when available. Otherwise it shows an explicit time-based fallback instead of reading the first message to manufacture a title.
3. **Inspection state:** Not inspected means no parse has run; Inspected means the persisted index matches the session file metadata; Stale means the file changed after the last inspection.
4. **Session summary:** total tokens, turns, tool calls, input/cache, output/reasoning, context peak/window, compactions, and API-equivalent cost.
5. **Per-turn data:** input, fresh input, cache reads, output, reasoning output, context peak, context-window utilization, context change, compactions, duration, and cost.
6. **Factual structured timeline:** displays every recorded `task_started`, assistant `reasoning/message` phase, `token_count`, `tool_search_call`, structured tool call/result, `context_compacted`, and `task_complete` event in session order. The previous 80-event cap has been removed.
7. **LLM events:** each `token_count` is numbered and shown as a model-response usage settlement with model, fresh input, cached input, output, reasoning output, total, context window, context delta from the previous call, cache-hit rate, and per-call API-equivalent cost. User messages, assistant `commentary/final_answer`, and readable reasoning summaries present in the Session can be displayed on demand from the original file but are not written to SQLite. Encrypted reasoning can expose only that a record exists and its size; its text cannot be reconstructed.
8. **Event taxonomy:** structured tool names, MCP namespaces, and safe arguments classify events as LLM, MCP, CLI, Skill, file, browser/automation, agent, context/lifecycle, or other tool. Classification is only a filter; the original tool name remains visible.
9. **MCP events:** show the exact tool name, server derived from the `mcp__*` namespace, and a redacted top-level argument summary.
10. **Call audit details:** each tool event can expand to show `call_id`, raw `response_item` type, exact start/end time, a recursively redacted input tree, and allowlisted result metadata such as result shape, item count, status, exit code, duration, and chunk/session/cell IDs.
11. **CLI / automation events:** show a bounded command or script summary, safe fields such as work directory, duration, a parsed exit code when available, and the JSONL result-record size. Result-record bytes are not terminal-output character counts.
12. **Skill events:** a structured call that explicitly reads `SKILL.md` is labeled with the skill name and short path. It proves the instruction file was read; the session format has no independent “skill completed” event.
13. **File and patch events:** read operations retain only safely extracted short paths; `apply_patch` retains affected filenames rather than patch content.

Collapsed argument summaries retain at most 12 top-level fields. Expanded trees recurse to at most five levels, keep at most 20 items per collection, and cap each preview at 6,000 characters. Values are replaced with `[redacted]` when field names imply tokens, secrets, passwords, authorization, credentials, cookies, API keys, private keys, or access keys. Bearer values, `sk-*`, and common secret assignments inside commands are redacted as well. Results expose only allowlisted metadata; complete tool outputs are never written to the index.

Current Codex session events do not expose an independent “memory usage” field. Codex X-Ray does not invent one; it shows the directly observable related values instead: peak input, model context window, utilization percentage, cache reads, and context compactions.

A conversation inspection parses only the selected session. A project inspection processes that project's uninspected or stale sessions one by one, exposes progress and failures, and can be stopped. Both write structured aggregates to Codex X-Ray's own incremental index, and later opens reuse the persisted result. Launching the app or refreshing the catalog never starts a project inspection, invokes a model, or consumes Codex quota.

## Extension usage

**Environment → Extension usage** is built entirely from the persisted execution-inspection index above. It does not scan all sessions to produce the report:

1. Calls, failures, repeats, result-record bytes, and start/end times come from structured tool-call and result events.
2. Project, session, and turn coverage is deduplicated from each call's `cwd`, session ID, and turn ID.
3. Total duration includes only calls with both start and completion events. The timed-call denominator is shown explicitly, so unfinished calls are not treated as zero milliseconds.
4. MCP entries are separated by server and exact tool name. A Skill is counted only when `SKILL.md` is explicitly read. CLI, browser, automation, file, and agent usage use the same structured event taxonomy.
5. A session changed after inspection is marked stale. Its persisted evidence remains visible, with a warning that current activity may be undercounted.
6. Codex sessions do not currently expose causal per-tool token usage. The extension report never presents the enclosing turn's tokens as that tool's tokens and does not calculate a fabricated per-tool cost.
7. Clicking a call identity opens the session and turn containing its most recent persisted occurrence and targets the matching tool request when a Call ID exists. Aggregate counts still use all inspected evidence.

## Session status

The current release reads official `thread/list.status` and uses it when an activity state is actually returned:

- `active`: running.
- `active + waitingOnApproval`: waiting for approval.
- `active + waitingOnUserInput`: waiting for input.
- `systemError`: failed.

Because Codex X-Ray's App Server is isolated from the Codex App process, another process's `activeFlags` are not guaranteed to be visible; the list will often report `idle` or `notLoaded`. Codex X-Ray then fills running/completed/interrupted/likely-failed gaps with `task_started`, `task_complete`, structured failures, and file modification time. Approval and input waits appear only when the official flag is actually returned; Codex X-Ray never guesses them. The independent App Server neither attaches to nor controls Codex App's active turns.

## Cache and startup

- WebView `localStorage` holds the latest Usage, cost, and Trace aggregate for immediate paint.
- A Rust-side snapshot in Codex X-Ray app data is the fallback.
- The cost index uses file metadata, so unchanged sessions are not parsed again.
- The Trace landing view refreshes only the official catalog; opening it never triggers a full session scan.
- Each analysis parses one user-selected session and atomically persists the aggregate in Codex X-Ray app data, with an in-memory copy for the running process.
- Background refresh starts no Codex task and consumes no model quota.

## Privacy and read-only boundary

Codex X-Ray:

- does not read `auth.json`;
- can display prompts, response text, and readable summaries on demand from the selected original Session, but does not write those message bodies to the SQLite index;
- does not store email, complete patches, read file contents, or full tool output;
- stores only bounded, redacted tool-argument, command, and script summaries for timelines the user explicitly analyzes;
- does not modify sessions or Codex databases;
- updates only user-selected configuration keys through official `config/batchWrite` after a diff preview and explicit confirmation;
- does not read or store Provider API keys, only the selected environment-variable name;
- stores only user-entered model rates in its own `pricing-config.json` and never changes Codex billing or account data;
- does not proxy or intercept Codex requests;
- does not upload local analysis;
- writes only its own caches and indexes in Codex X-Ray app data.

## Official references

- [Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [Codex configuration reference (`sqlite_home` and `$CODEX_HOME`)](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Alibaba Model Studio Responses API](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses)
- [Volcengine Ark Responses API](https://www.volcengine.com/docs/82379/1795150)
- [Baidu Qianfan Responses API](https://cloud.baidu.com/doc/qianfan-docs/s/4mi400l1m)
- [Kimi K2.6](https://platform.kimi.com/docs/guide/kimi-k2-6-quickstart)
- [MiniMax Codex setup](https://platform.minimaxi.com/docs/token-plan/codex)
- [Xiaomi MiMo Codex setup](https://mimo.mi.com/docs/en-US/tokenplan/integration/codex-configuration)

Account methods such as `account/usage/read` are compatibility-checked against the schema generated by the currently installed Codex App Server. App Server surfaces can evolve, so Codex X-Ray treats absent fields as unavailable rather than guessing.
