# Security policy

## Supported versions

Security fixes are applied to the latest released minor version.

## Reporting

Do not post credentials, Codex Session contents, local paths containing personal data, or proof-of-concept exploits in a public issue. Use GitHub private vulnerability reporting for <https://github.com/lakernote/codex-xray>.

Include the application version, operating system, affected data path or command, impact, and a minimal reproduction with secrets removed.

## Security boundary

Codex X-Ray reads local Codex Session metadata and can update selected Codex configuration keys only after a visible diff and explicit confirmation. Provider keys may be kept in the operating-system credential store or supplied through an environment variable. They must never be written to `config.toml`, SQLite, logs, the webview, or process arguments.

When a user explicitly selects a Chat Completions provider, Codex connects to a loopback-only X-Ray bridge. The bridge accepts Responses requests from Codex, forwards only the credential belonging to that provider, and translates function-tool traffic to the selected upstream. It does not expose the credential through its health or model-catalog endpoints and does not forward unrelated inbound authorization.
