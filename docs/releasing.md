# Release and update pipeline

The repository can build draft installers on GitHub today. In-app automatic update must remain disabled until signing, notarization, and update verification are configured.

## 1. Repository settings

The public repository is <https://github.com/lakernote/codex-xray> and its default branch is `main`.

- Keep GitHub Actions enabled.
- In **Settings → Actions → General → Workflow permissions**, allow read and write access so the release workflow can commit the version and create its tag.
- Enable private vulnerability reporting.
- Keep the non-affiliation disclaimer in the repository description and README.
- Do not push local databases, Session files, credentials, `.playwright-cli`, `dist`, `node_modules`, or `src-tauri/target`.

## 2. Validate a change

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

The `CI` workflow runs the same gates for `main` and pull requests.

## 3. Create a release

Update `CHANGELOG.md`, merge it to `main`, and wait for CI. Then open **GitHub → Actions → Release → Run workflow**.

Choose one release type and enter the version without a `v` prefix:

- `prerelease`: use a semantic prerelease version such as `0.2.0-beta.1`.
- `release`: use a stable semantic version such as `0.2.0`.

The workflow synchronizes the version across the npm and Tauri manifests, commits that version to `main`, creates the annotated `v<version>` tag, reruns every quality gate, and builds draft macOS DMG, Windows NSIS, Linux DEB, and Linux RPM installers. Prerelease versions are marked as GitHub prereleases automatically. NSIS is used instead of MSI so semantic prerelease versions such as `beta.1` remain valid on Windows.

Configure a protected GitHub `release` environment before the first public release. Review every draft artifact, then publish the GitHub Release manually. If a platform build fails, fix the workflow and run the same version again. An existing draft tag is reused and only missing installers are rebuilt; already published releases remain immutable.

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
