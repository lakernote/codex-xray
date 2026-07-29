# Security policy

## Supported versions

Security fixes are applied to the latest released minor version.

## Reporting

Do not post credentials, Codex Session contents, local paths containing personal data, or proof-of-concept exploits in a public issue. Use GitHub private vulnerability reporting for <https://github.com/lakernote/codex-xray>.

Include the application version, operating system, affected data path or command, impact, and a minimal reproduction with secrets removed.

## Security boundary

Codex X-Ray reads local Codex Session metadata and can update selected Codex configuration keys only after a visible diff and explicit confirmation. Provider connection tests transiently read the selected environment variable in the Rust backend. Keys must never be persisted, returned to the webview, logged, or placed in process arguments.
