import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const cli = resolve(
  repositoryRoot,
  "node_modules",
  "@tauri-apps",
  "cli",
  "tauri.js",
);
const env = { ...process.env };

// Finder and Launchpad use Spotlight, which otherwise discovers every local
// .app bundle under Cargo's target directory and presents it like an installed
// application. A .noindex target keeps local development bundles out of app
// search. CI must keep Cargo's default target directory because release actions
// discover and upload bundles from that standard path.
if (process.platform === "darwin" && !env.CI && !env.CARGO_TARGET_DIR) {
  const target = resolve(repositoryRoot, "src-tauri", "target.noindex");
  mkdirSync(target, { recursive: true });
  env.CARGO_TARGET_DIR = target;
}

const result = spawnSync(process.execPath, [cli, ...process.argv.slice(2)], {
  cwd: repositoryRoot,
  env,
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 1);
