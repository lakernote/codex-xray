# Security policy

## Supported versions

Security fixes are applied to the latest released minor version.

## Reporting

Do not post credentials, Codex Session contents, local paths containing personal data, or proof-of-concept exploits in a public issue. Use GitHub private vulnerability reporting for <https://github.com/lakernote/codex-xray>.

Include the application version, operating system, affected data path or command, impact, and a minimal reproduction with secrets removed.

## Security boundary

Codex X-Ray reads local Codex Session metadata and can update selected Codex configuration keys only after a visible diff and explicit confirmation. Provider keys may be kept in a user-only file under `~/.codex/codex-xray/credentials/` or supplied through an environment variable. They must never be written to `config.toml`, SQLite, logs, the webview, or process arguments.

When a user explicitly selects a Chat Completions provider, Codex connects to a loopback-only X-Ray bridge. The bridge requires inbound authorization to match the selected credential, forwards only that credential, and translates function-tool traffic to the selected upstream. It does not expose the credential through its health or model-catalog endpoints.

The execution index stores structural metadata, source references, and bounded/redacted argument summaries. Session message bodies, full patches, file contents, and complete tool output are read from the original Session only on demand and are not persisted in the X-Ray index.

Enabling the WeChat channel sends the paired owner's prompts, Codex replies, progress, and approval summaries through WeChat. The channel accepts only the paired owner's direct messages, shortens local paths in system-generated messages, redacts common credential forms, and exposes no public App Server port. Model-generated content may still contain information the user asked Codex to process.
