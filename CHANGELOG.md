# Changelog

All notable user-visible changes are recorded here.

## [Unreleased]

- Added an in-process WeChat remote channel with QR pairing, owner-only direct messages, Codex task listing and attachment, progress monitoring, turn steering/interruption, and remote approvals. Closing X-Ray stops the channel; no background service is installed.
- Added follower-mode Codex Desktop IPC control so WeChat can operate the exact Desktop-owned task, including live state, steering, interruption, and approvals. IPC failures are fail-closed and never fall back to a duplicate conversation.
- Hardened Desktop IPC for long conversations and restarts: snapshots up to 256 MiB are parsed without a duplicate byte buffer, failed streams are discarded, read-only owner discovery/state refresh reconnect once automatically, and mutating requests are never blindly retried.
- Added proactive WeChat progress feedback for long turns, with an initial processing update, activity-aware deduplication, rate limiting, heartbeat updates, highlighted approvals, and explicit completed/stopped/failed messages.
- Changed WeChat attachment to target only the selected original Codex task. Writer conflicts now return an explicit not-attached message, and X-Ray never creates a fork automatically; new tasks require an explicit action in X-Ray.
- Simplified WeChat control: choose or explicitly create the target in X-Ray, then send plain text in WeChat. `/list` is now an optional remote switcher with direct number selection, and compatibility failures explain that the original task was not controlled and no copy was created.
- Added a loopback-only compatibility bridge for OpenAI-compatible Chat Completions providers, including model metadata, streaming text, function tools, tool-result round trips, token usage, and configurable context windows.
- Added direct GLM and DeepSeek Chat Completions presets while keeping native Responses providers on Codex's direct path.
- Added direct Provider API-key entry backed by a user-only credential file, with environment-variable authentication still available.
- Added command-backed Provider authentication so Codex can read a saved key without writing it to `config.toml`, SQLite, logs, or process arguments.
- Hardened privacy by removing message bodies, full patches, complete tool output, and unredacted credentials from the persistent Trace index; old Trace caches are cleared during migration.
- Shortened local paths and redacted common command credentials in X-Ray-generated WeChat messages.
- Moved environment and data checks into the Console diagnostics section and removed the duplicate sidebar destination.
- Added a daily GitHub Releases check with per-version ignore, signed in-app installation on macOS and Windows, and manual Linux downloads.
- Added release gates that require signed updater packages and a complete platform manifest before a draft can be published.
- Added a GitHub version preflight so an incomplete same-version manifest no longer reports a false upgrade failure.
- Fixed prerelease packaging on Windows by publishing an NSIS installer instead of MSI.
- Reduced release assets to DMG, NSIS, DEB, and RPM installers with resumable draft builds.
- Added installer selection and version guidance to both READMEs.
- Replaced manual release tags with a guided GitHub Actions prerelease/release workflow.
- Added release tag/version validation and full quality gates before packaging.
- Added Clippy and npm dependency auditing to CI.
- Removed the retired data-source view and its bundled resources.
- Added effective-dated custom pricing versions.
- Added real Provider connection verification with latency and HTTP status.
- Added explicit batch-analysis confirmation and failed-session retry.
- Added GitHub CI and cross-platform draft release workflows.
- Removed the tray/menu-bar mode; closing the main window now exits X-Ray normally.

## [0.1.0] - 2026-07-29

- Initial Usage, project/session/turn ledger, execution trace, Provider control, Codex settings, environment diagnosis, themes, and desktop shell.
