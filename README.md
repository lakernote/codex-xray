<p align="center">
  <img src="src-tauri/icons/icon.svg" width="96" height="96" alt="Codex X-Ray icon" />
</p>

<h1 align="center">Codex X-Ray</h1>

<p align="center">
  Understand Codex usage, cost, context, and every execution step.
</p>

<p align="center">
  <strong>English</strong> · <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/lakernote/codex-xray/actions/workflows/ci.yml"><img src="https://github.com/lakernote/codex-xray/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/lakernote/codex-xray/releases"><img src="https://img.shields.io/github/v/release/lakernote/codex-xray?include_prereleases&amp;sort=semver&amp;label=release" alt="GitHub Release" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-2f4f20.svg" alt="MIT License" /></a>
</p>

Codex X-Ray is a usage and execution analysis tool for Codex. It shows quota and tokens, tracks cost by project, conversation, and turn, and reconstructs the execution timeline of LLM and tool calls.

> [!IMPORTANT]
> Codex X-Ray is an unofficial open-source project and is not affiliated with or endorsed by OpenAI.

## Download

Preview installers are available on the [GitHub Releases page](https://github.com/lakernote/codex-xray/releases). Choose the asset by its filename suffix:

| System | Installer |
| --- | --- |
| macOS · Apple Silicon | `_aarch64.dmg` |
| macOS · Intel | `_x64.dmg` |
| Windows · 64-bit | `_x64-setup.exe` |
| Ubuntu / Debian · 64-bit | `_amd64.deb` |
| Fedora / RHEL · 64-bit | `.x86_64.rpm` |

These preview builds are not yet signed with a trusted publisher certificate or notarized, so the operating system may show an unidentified-developer warning. Codex must already be installed and authenticated. The “Source code” archives are source snapshots, not installers.

The app checks GitHub Releases at most once per day. When a newer version is available, you can ignore that version or open its download page. Codex X-Ray never downloads or installs an update automatically.

## Screenshots

### Usage overview

Official quota and local Session usage remain separate, with input, cache, output, yearly activity, and API-equivalent cost shown together.

![Codex X-Ray usage overview](docs/assets/usage-overview.en.png)

### Execution timeline

Follow local preparation, user input, LLM output, tool request, Codex execution, result write-back, token accounting, and turn completion in Session order.

![Codex X-Ray execution timeline](docs/assets/execution-trace.en.png)

Every screenshot is generated from a fictional project and simulated Session data. No real user path, conversation, account, or key is included.

## What it does

### Usage

- Displays official account information, quota windows, reset times, Credits, lifetime usage, peak usage, and activity streaks when Codex provides them.
- Builds a local ledger for today, day, month, model, and project. Each view separates uncached input, cache reads, output, reasoning, total tokens, and API-equivalent cost.
- Drills down from project to conversation and turn, so token usage and estimated cost can be traced to individual work.
- Includes a yearly activity heatmap and effective-dated custom model pricing. Local aggregates are incrementally indexed in SQLite.

### Execution trace

- Organizes Codex Sessions by working directory, conversation, and turn.
- Reconstructs the real event order: local preparation, user input, LLM output, tool request, Codex execution, result write-back, token accounting, compaction, and turn completion.
- Identifies CLI commands, MCP calls, Skills, browser operations, automation, and sub-agent activity, with available arguments, results, duration, tokens, context use, cache hit rate, and source line references.
- Analyzes conversations only when requested and keeps the analysis result in X-Ray's SQLite index; original Session files remain unchanged.

### Console

- Switches between native Responses providers and OpenAI-compatible Chat Completions providers. Presets include OpenAI and common providers, with a custom-provider editor for base URL, model, context window, and protocol.
- Runs Chat Completions providers through X-Ray's local compatibility bridge, translating text, streaming responses, function calls, tool results, and usage back to the Responses shape expected by Codex.
- Stores provider keys in the operating system credential store, or references an environment variable. Keys are not written to `config.toml`.
- Exposes model, reasoning, verbosity, personality, approval, sandbox, network, history, Memory, compaction, tool, app, sub-agent, goal, and hook settings with explanations.
- Shows the exact configuration diff before applying it through the Codex App Server and keeps a recoverable previous state.
- Reports the detected Codex CLI and App Server versions, and can open or copy the paths for config, Sessions, Skills, Plugins, and X-Ray's SQLite index.

### Desktop experience

- Chinese and English interfaces with light and dark themes.
- Closing the main window keeps the app and an active Chat bridge available from the system tray.
- Checks GitHub Releases at most once per day. A new-version prompt can be dismissed, ignored for that version, or opened for manual download; updates are never installed automatically.

## Data flow

```text
Codex App Server ──account, quota, catalog, configuration──┐
                                                          ├─ local Rust analysis ─ SQLite index ─ React UI
$CODEX_HOME/sessions ──read-only JSONL events─────────────┘

Optional Chat provider: Codex Responses request ─ X-Ray bridge ─ vendor /chat/completions
```

- Official values keep their original semantics; local derivations and cost estimates are labeled.
- Original Codex Sessions, databases, and task content remain read-only.
- Conversations are analyzed only after selection; opening the catalog does not parse all history.
- The index lives in Codex X-Ray's own application data directory and is incrementally updated with SQLite WAL.

## Privacy and security

- Codex X-Ray does not read `auth.json` or upload local analysis. Native Responses providers connect directly; only a Chat Completions provider explicitly selected in the Console is routed through the local X-Ray bridge.
- Execution details can display user messages, assistant messages, and readable summaries from the original Session on demand; message bodies are not written to the SQLite index.
- SQLite stores usage, structured phases, source line references, and bounded/redacted command, argument, and result metadata. It does not store complete tool output, full patches, or files read by Codex.
- Provider keys can be stored in macOS Keychain, Windows Credential Manager, or Linux Secret Service. Codex reads them on demand through its official command-backed provider authentication; keys are never written to `config.toml`, SQLite, logs, or process arguments. The local Chat bridge forwards only the selected provider key and ignores unrelated inbound authorization. Environment-variable authentication remains available.
- Configuration changes require a visible diff and explicit confirmation, with a recoverable previous state.

See the [English data source guide](docs/data-sources.en.md) for field sources, formulas, and limitations. See [SECURITY.md](SECURITY.md) for vulnerability reporting.

## Run from source

Development requires:

- Node.js 22 (18 minimum)
- Rust stable
- An installed and authenticated Codex

```bash
git clone https://github.com/lakernote/codex-xray.git
cd codex-xray
npm ci
npm run tauri dev
```

Build and verify:

```bash
npm run version:check
npm run check
npm run build
npm run test:rust
npm run tauri build
```

If `codex` is not on `PATH`, Codex X-Ray attempts to detect the CLI bundled with the Codex/ChatGPT app. You can also set `CODEX_BIN` explicitly.

## Current limitations

- API-equivalent cost estimates token value; it is not a ChatGPT/Codex subscription bill or an actual charge.
- A separate App Server cannot always observe every transient state inside another Codex App process. The UI distinguishes official states from local-event inference.
- Codex App Server and Session formats may evolve; compatibility follows the locally installed version.
- The Chat bridge translates text, streaming output, function tools, tool results, and token usage. Responses-only features such as native Web Search, encrypted reasoning, and server-side compaction are not translated.
- Chat providers require Codex X-Ray to remain running; closing the window keeps it in the system tray.
- The project is currently validated primarily on macOS. Release builds cover Windows, Linux, and macOS.

## License

[MIT](LICENSE)
