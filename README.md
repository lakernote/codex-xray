<p align="center">
  <img src="src-tauri/icons/icon.svg" width="96" height="96" alt="Codex X-Ray icon" />
</p>

<h1 align="center">Codex X-Ray</h1>

<p align="center">See Codex usage, cost, context, and every execution step—locally.</p>

<p align="center">
  <strong>English</strong> · <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/lakernote/codex-xray/actions/workflows/ci.yml"><img src="https://github.com/lakernote/codex-xray/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/lakernote/codex-xray/releases"><img src="https://img.shields.io/github/v/release/lakernote/codex-xray?include_prereleases&amp;sort=semver&amp;label=release" alt="Release" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-2f4f20.svg" alt="MIT License" /></a>
</p>

Codex X-Ray is a local desktop workbench for understanding and controlling Codex. It combines official account data with read-only analysis of local Codex Sessions.

> [!IMPORTANT]
> Codex X-Ray is an unofficial open-source project. It is not affiliated with or endorsed by OpenAI.

## Features

### Usage and cost

- View official quota and reset times separately from local Session usage.
- Break down tokens and API-equivalent cost by day, month, model, project, conversation, and turn.
- Configure model prices by effective date without changing real provider billing.

[![Usage overview](docs/assets/usage-overview.en.png)](docs/assets/usage-overview.en.png)

### Execution trace

- Reconstruct a turn from prompt and model response through CLI, MCP, Skill, file, browser, and subagent activity.
- Inspect token settlements, context growth, cache use, compaction boundaries, tool duration, failures, and repeated work.
- Open the original Session records on demand to verify the analysis line by line.

[![Execution trace](docs/assets/execution-trace.en.png)](docs/assets/execution-trace.en.png)

### WeChat remote control

- Pair the owner account by QR code and select a Codex task in X-Ray.
- Continue the same task running in Codex Desktop through user-local IPC, or explicitly create a new X-Ray task.
- Send normal WeChat messages, receive progress updates during long work, stop a turn, and approve or deny requests remotely.
- Use `/list` to switch tasks temporarily and `/status` to check the current target.
- Never copy, fork, or create a conversation from an accidental message.

The channel runs only while Codex X-Ray is open. Closing the app stops WeChat and all X-Ray-owned local services; no background daemon or login item is installed.

### Model access and configuration

- Save and switch multiple provider profiles with independent endpoints, models, protocols, and credentials.
- Use native Responses providers directly or connect compatible Chat Completions providers through X-Ray's loopback bridge.
- Edit common Codex settings through a reviewed diff with a recoverable previous state.

[![Provider configuration](docs/assets/provider-console.en.png)](docs/assets/provider-console.en.png)

### Desktop conveniences

- Chinese and English UI, light and dark themes, version checks, and signed in-app stable updates.
- Quick access to Codex configuration, Sessions, Skills, Plugins, and the X-Ray analysis database.

All screenshots use fictional projects and simulated data. They contain no real path, conversation, account, or credential.

## Download

Preview installers are available on [GitHub Releases](https://github.com/lakernote/codex-xray/releases):

| Platform | Package |
| --- | --- |
| macOS · Apple silicon | `_aarch64.dmg` |
| macOS · Intel | `_x64.dmg` |
| Windows · 64-bit | `_x64-setup.exe` |
| Ubuntu / Debian · 64-bit | `_amd64.deb` |
| Fedora / RHEL · 64-bit | `.x86_64.rpm` |

Codex must already be installed and authenticated. Preview builds are not yet signed or notarized by a trusted publisher, so the operating system may show an unknown-developer warning.

## Privacy and security

- Analysis is local and has no telemetry. Original Codex Sessions and databases remain read-only.
- Conversation text is read from the original Session only when selected. The SQLite index stores structural metadata, bounded/redacted argument summaries, and source references—not message bodies, full patches, file contents, or complete tool output.
- Provider keys use a user-only credential file under `~/.codex/codex-xray/credentials/` or an environment variable. They are not written to `config.toml`, SQLite, logs, the webview, or process arguments.
- Native Responses traffic goes directly from Codex to the provider. Selecting a Chat provider sends that task's requests through the loopback bridge to the configured provider.
- Enabling WeChat necessarily sends the owner's prompts, Codex replies, progress, and approval summaries through WeChat. X-Ray shortens local paths and redacts common credentials in system-generated messages, but user-requested/model-generated content can still contain private information.
- Remote control accepts only the paired owner's direct messages and does not expose a public App Server port.

See [Data sources and metric definitions](docs/data-sources.en.md) and [Security policy](SECURITY.md) for the complete boundary.

## Run from source

Requires Node.js 22 (18 minimum), Rust stable, and an installed/authenticated Codex.

```bash
git clone https://github.com/lakernote/codex-xray.git
cd codex-xray
npm ci
npm run tauri dev
```

Useful checks:

```bash
npm run check
npm run build
npm run test:rust
```

## Current limits

- Cost is an API-equivalent estimate, not a subscription bill or actual charge.
- Same-task Desktop control relies on a private, user-local IPC protocol and is version-sensitive. X-Ray fails closed instead of creating a different conversation when control cannot be established.
- The WeChat channel accepts direct text and available voice transcripts; group messages and media do not enter Codex.
- The Chat bridge cannot translate Responses-only features such as native Web Search, encrypted reasoning, or server-side compaction.
- The project is validated primarily on macOS; release builds also cover Windows and Linux.

## License

[MIT](LICENSE) · [Third-party notices](THIRD_PARTY_NOTICES.md)
