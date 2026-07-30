# Release pipeline

The repository builds draft installers on GitHub. Codex X-Ray checks public GitHub Releases for newer versions and sends users to the download page; it does not download or install updates.

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

## 4. Version reminders

The app requests the repository's public GitHub Releases API at most once per day. Stable builds only consider stable releases; prerelease builds also consider prereleases. A user may ignore one version or open the Release page and download an installer manually. No updater signing key or update manifest is required.

GitHub Actions reference:

- <https://v2.tauri.app/distribute/pipelines/github/>
