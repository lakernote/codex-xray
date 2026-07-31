# Release pipeline

The repository builds draft installers on GitHub. Stable builds can check, download, verify, install, and restart through Tauri's signed updater.

## 1. Repository settings

The public repository is <https://github.com/lakernote/codex-xray> and its default branch is `main`.

- Keep GitHub Actions enabled.
- In **Settings → Actions → General → Workflow permissions**, allow read and write access so the release workflow can commit the version and create its tag.
- Enable private vulnerability reporting.
- Keep the non-affiliation disclaimer in the repository description and README.
- Do not push local databases, Session files, credentials, `.playwright-cli`, `dist`, `node_modules`, or `src-tauri/target`.

### Updater signing

Updater signatures are separate from Apple or Windows publisher signatures. They prevent a modified package from being installed by Codex X-Ray.

The updater public key is committed in `src-tauri/tauri.conf.json`. Keep the matching private key outside the repository and add its complete contents as the protected `release` environment secret:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` only when the private key was generated with a password

The current maintainer key is stored at `~/.tauri/codex-xray-updater.key`; back it up securely. Do not commit or share it. The workflow keeps building ordinary installers when the secret is absent, but it cannot publish updater artifacts or `latest.json` in that case.

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

Unsigned development artifacts are suitable for internal verification only. Public macOS distribution should add a Developer ID Application certificate, hardened runtime, and Apple notarization. Tauri updater signing secures the update package but does not replace Apple's trust checks for a newly downloaded app.

## 4. Version reminders

The app asks the signed Tauri updater endpoint at most once per day. A user may ignore one version, or download and install the update in place. The updater verifies the package signature before installation and restarts the app afterward.

GitHub's `/releases/latest` endpoint points to stable releases, so in-app installation currently follows the stable channel. Preview releases remain available from the Releases page. macOS and Windows bundles support the updater; the current Linux DEB/RPM distribution remains manual because Tauri's Linux updater uses AppImage packages.

References:

- <https://v2.tauri.app/distribute/pipelines/github/>
- <https://v2.tauri.app/plugin/updater/>
- <https://v2.tauri.app/distribute/sign/macos/>
- <https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution>
