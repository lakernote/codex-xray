# Changelog

All notable user-visible changes are recorded here.

## [Unreleased]

- Added a loopback-only compatibility bridge for OpenAI-compatible Chat Completions providers, including model metadata, streaming text, function tools, tool-result round trips, token usage, and configurable context windows.
- Added direct GLM and DeepSeek Chat Completions presets while keeping native Responses providers on Codex's direct path.
- Added direct Provider API-key entry backed by macOS Keychain, Windows Credential Manager, or Linux Secret Service.
- Added command-backed Provider authentication so Codex can read a saved key without writing it to `config.toml`, SQLite, logs, or process arguments.
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
