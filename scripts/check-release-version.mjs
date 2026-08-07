import { readFile } from "node:fs/promises";

const root = new URL("../", import.meta.url);
const requestedTag = process.argv[2] ?? process.env.GITHUB_REF_NAME ?? "";
const version = requestedTag.replace(/^v/, "");

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`Expected a release tag such as v0.2.0, received “${requestedTag || "nothing"}”.`);
}

const [packageSource, tauriSource, workspaceSource, memberSource] = await Promise.all([
  readFile(new URL("package.json", root), "utf8"),
  readFile(new URL("src-tauri/tauri.conf.json", root), "utf8"),
  readFile(new URL("Cargo.toml", root), "utf8"),
  readFile(new URL("src-tauri/Cargo.toml", root), "utf8"),
]);

const packageVersion = JSON.parse(packageSource).version;
const tauriVersion = JSON.parse(tauriSource).version;
const workspacePackageStart = workspaceSource.indexOf("[workspace.package]");
const workspacePackageEnd = workspaceSource.indexOf("\n[", workspacePackageStart + "[workspace.package]".length);
const workspacePackage = workspacePackageStart >= 0
  ? workspaceSource.slice(workspacePackageStart, workspacePackageEnd >= 0 ? workspacePackageEnd : undefined)
  : "";
const cargoVersion = workspacePackage.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const memberInherits = /^version\.workspace\s*=\s*true$/m.test(memberSource);
const versions = {
  "release tag": version,
  "package.json": packageVersion,
  "src-tauri/tauri.conf.json": tauriVersion,
  "Cargo.toml (workspace)": cargoVersion,
  "src-tauri/Cargo.toml": memberInherits ? cargoVersion : undefined,
};

const mismatches = Object.entries(versions).filter(([, candidate]) => candidate !== version);
if (mismatches.length > 0) {
  const details = Object.entries(versions).map(([source, candidate]) => `  ${source}: ${candidate ?? "missing"}`).join("\n");
  throw new Error(`Release versions must match:\n${details}`);
}

console.log(`Release version ${version} is consistent.`);
