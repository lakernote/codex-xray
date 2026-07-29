import { readFile } from "node:fs/promises";

const packageJson = JSON.parse(await readFile("package.json", "utf8"));
const tauriConfig = JSON.parse(
  await readFile("src-tauri/tauri.conf.json", "utf8"),
);
const cargoToml = await readFile("src-tauri/Cargo.toml", "utf8");
const cargoPackage = cargoToml.match(
  /\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
);

if (!cargoPackage) {
  throw new Error("Could not find [package].version in src-tauri/Cargo.toml");
}

const versions = {
  packageJson: packageJson.version,
  tauriConfig: tauriConfig.version,
  cargoToml: cargoPackage[1],
};
const unique = new Set(Object.values(versions));

if (unique.size !== 1) {
  throw new Error(`Version mismatch: ${JSON.stringify(versions)}`);
}

console.log(`Version ${versions.packageJson} is consistent.`);
