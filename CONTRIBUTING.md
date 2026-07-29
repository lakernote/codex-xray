# Contributing

Codex X-Ray is a Tauri desktop application. Keep changes evidence-based: official Codex values, local Session facts, and X-Ray estimates must remain visibly distinct.

## Local verification

```bash
npm ci
npm run version:check
npm run check
npm run build
npm audit --audit-level=high
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

## Pull requests

- Keep Codex Session and database access read-only.
- Never add credentials, `auth.json`, Session contents, local databases, build output, or screenshots containing secrets.
- Configuration writes must use the official Codex configuration API, show a diff, require explicit confirmation, and retain a recoverable restore point.
- New metrics must document their source, formula, completeness, and whether they are official, locally derived, or estimated.
- Update both Chinese and English copy when a user-visible concept changes.
- Add Rust tests for parsers, aggregation, migrations, and configuration edits.

## Versioning

Keep these three versions identical:

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`

`npm run version:check` enforces this in CI.
