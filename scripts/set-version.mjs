import { readFile, writeFile } from "node:fs/promises";

const version = process.argv[2];
const channelFlag = process.argv.indexOf("--channel");
const channel = channelFlag === -1 ? undefined : process.argv[channelFlag + 1];

if (!version) {
  throw new Error(
    "Usage: node scripts/set-version.mjs <version> --channel <prerelease|release>",
  );
}

if (version.startsWith("v")) {
  throw new Error(`Enter the version without the v prefix: ${version}`);
}

if (channel !== "prerelease" && channel !== "release") {
  throw new Error(
    "Release channel must be either prerelease or release.",
  );
}

const semanticVersion =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;
const match = version.match(semanticVersion);

if (!match) {
  throw new Error(`Invalid semantic version: ${version}`);
}

const prerelease = match[4];
if (channel === "release" && prerelease) {
  throw new Error(
    `A release version cannot contain a prerelease suffix: ${version}`,
  );
}
if (channel === "prerelease" && !prerelease) {
  throw new Error(
    `A prerelease version needs a suffix, for example ${version}-beta.1`,
  );
}
if (
  prerelease &&
  prerelease
    .split(".")
    .some((identifier) => /^\d+$/.test(identifier) && /^0\d+/.test(identifier))
) {
  throw new Error(
    `Numeric prerelease identifiers cannot contain leading zeroes: ${version}`,
  );
}

const packageJsonPath = "package.json";
const packageLockPath = "package-lock.json";
const tauriConfigPath = "src-tauri/tauri.conf.json";
const cargoTomlPath = "src-tauri/Cargo.toml";
const cargoLockPath = "src-tauri/Cargo.lock";

const packageJson = JSON.parse(await readFile(packageJsonPath, "utf8"));
const packageLock = JSON.parse(await readFile(packageLockPath, "utf8"));
const tauriConfig = JSON.parse(await readFile(tauriConfigPath, "utf8"));
const cargoToml = await readFile(cargoTomlPath, "utf8");
const cargoLock = await readFile(cargoLockPath, "utf8");

if (!packageLock.packages?.[""]) {
  throw new Error("Could not find the root package in package-lock.json");
}

const nextCargoToml = cargoToml.replace(
  /(\[package\][\s\S]*?^version\s*=\s*")[^"]+(")/m,
  `$1${version}$2`,
);
if (nextCargoToml === cargoToml && !cargoToml.includes(`version = "${version}"`)) {
  throw new Error("Could not update [package].version in src-tauri/Cargo.toml");
}

const nextCargoLock = cargoLock.replace(
  /(\[\[package\]\]\r?\nname = "codex-xray"\r?\nversion = ")[^"]+(")/,
  `$1${version}$2`,
);
if (nextCargoLock === cargoLock && !cargoLock.includes(`version = "${version}"`)) {
  throw new Error("Could not update the codex-xray package in Cargo.lock");
}

packageJson.version = version;
packageLock.version = version;
packageLock.packages[""].version = version;
tauriConfig.version = version;

await Promise.all([
  writeFile(packageJsonPath, `${JSON.stringify(packageJson, null, 2)}\n`),
  writeFile(packageLockPath, `${JSON.stringify(packageLock, null, 2)}\n`),
  writeFile(tauriConfigPath, `${JSON.stringify(tauriConfig, null, 2)}\n`),
  writeFile(cargoTomlPath, nextCargoToml),
  writeFile(cargoLockPath, nextCargoLock),
]);

console.log(`Set Codex X-Ray version to ${version} (${channel}).`);
