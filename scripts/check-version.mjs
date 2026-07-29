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

const tagFlag = process.argv.indexOf("--tag");
if (tagFlag !== -1) {
  const tag = process.argv[tagFlag + 1];
  if (!tag) {
    throw new Error("Missing value after --tag");
  }
  if (!tag.startsWith("v")) {
    throw new Error(`Release tag must start with v: ${tag}`);
  }

  const taggedVersion = tag.slice(1);
  if (taggedVersion !== versions.packageJson) {
    throw new Error(
      `Release tag ${tag} does not match application version ${versions.packageJson}`,
    );
  }
}

const channelFlag = process.argv.indexOf("--channel");
if (channelFlag !== -1) {
  const channel = process.argv[channelFlag + 1];
  if (channel !== "prerelease" && channel !== "release") {
    throw new Error(
      "Release channel must be either prerelease or release.",
    );
  }

  const hasPrereleaseSuffix = versions.packageJson.includes("-");
  if (channel === "prerelease" && !hasPrereleaseSuffix) {
    throw new Error(
      `Prerelease channel requires a prerelease version: ${versions.packageJson}`,
    );
  }
  if (channel === "release" && hasPrereleaseSuffix) {
    throw new Error(
      `Release channel requires a stable version: ${versions.packageJson}`,
    );
  }
}

console.log(`Version ${versions.packageJson} is consistent.`);
