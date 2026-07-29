# Release and update pipeline

The repository can build draft installers on GitHub today. In-app automatic update must remain disabled until the repository URL and signing identity are final.

## 1. Publish the repository

1. Create a public GitHub repository for Codex X-Ray.
2. Keep the default branch as `main`.
3. Enable GitHub Actions with read/write workflow permission.
4. Enable private vulnerability reporting.
5. Add the non-affiliation disclaimer to the GitHub description.

Do not push local databases, Session files, credentials, `.playwright-cli`, `dist`, `node_modules`, or `src-tauri/target`.

## 2. Validate a change

```bash
npm ci
npm run version:check
npm run check
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

The `CI` workflow runs the same gates for `main` and pull requests.

## 3. Cut a draft release

Update the same semantic version in:

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`

Update `CHANGELOG.md`, merge to `main`, then push `v<version>`. The `Draft release` workflow follows Tauri's official GitHub pipeline and creates draft macOS Apple Silicon, macOS Intel, Windows, and Linux artifacts. Review every artifact before publishing the GitHub Release.

Unsigned development artifacts are suitable for internal verification only. Public macOS and Windows distribution should add platform signing and macOS notarization first.

## 4. Enable signed in-app updates

This stage requires a stable public repository and must not use placeholder values.

1. Generate a Tauri updater signing keypair and store the private key only as GitHub Actions secrets.
2. Add the public key and the repository's real `latest.json` endpoint to Tauri updater configuration.
3. Enable updater artifact generation in `tauri.conf.json`.
4. Add the Tauri updater plugin and a user-controlled “Check for updates” action.
5. Pass the signing key and password to the release workflow as secrets.
6. Test upgrade from the previous released version on every supported platform.

The updater verifies signed artifacts. Never ship an updater that trusts unsigned files or a mutable placeholder endpoint.

Official references:

- <https://v2.tauri.app/distribute/pipelines/github/>
- <https://v2.tauri.app/plugin/updater/>
