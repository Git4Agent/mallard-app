import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Assembles the Tauri updater manifest (`latest.json`) for one release from
// the three per-platform signatures the build jobs produced. The release
// workflow runs this once, after every build job has attached its artifacts,
// so the manifest is written whole: no job uploads a partial one, and no job
// needs `--clobber` to fix another's. Publishing the draft is what makes
// `releases/latest/download/latest.json` resolve to it.

export const RELEASE_REPOSITORY = "Git4Agent/mallard-app";

/** Updater platform key → the release asset that platform downloads. */
export function updaterAssetName(platform, version) {
  switch (platform) {
    case "darwin-aarch64":
      return `Mallard_${version}_aarch64.app.tar.gz`;
    case "darwin-x86_64":
      return `Mallard_${version}_x64.app.tar.gz`;
    case "windows-x86_64":
      return `Mallard_${version}_x64-setup.exe`;
    default:
      throw new Error(`Unknown updater platform ${platform}.`);
  }
}

export const UPDATER_PLATFORMS = ["darwin-aarch64", "darwin-x86_64", "windows-x86_64"];

function assertVersion(version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`Release version must be MAJOR.MINOR.PATCH, got "${version}".`);
  }
}

/**
 * Build the manifest object. `signatures` maps every updater platform to the
 * contents of its `.sig` file. The `notes` field is the release page: the
 * client links it rather than repeating the body, and the maintainer edits
 * the page after this file is written.
 */
export function buildUpdaterManifest({ version, tag, repository = RELEASE_REPOSITORY, signatures, pubDate }) {
  assertVersion(version);
  if (tag !== `v${version}`) throw new Error(`Tag "${tag}" does not match version ${version}.`);
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    throw new Error(`Repository must be owner/name, got "${repository}".`);
  }
  const date = pubDate ?? new Date().toISOString();
  if (Number.isNaN(Date.parse(date))) throw new Error(`Publication date "${date}" is not a date.`);

  const platforms = {};
  for (const platform of UPDATER_PLATFORMS) {
    const signature = signatures[platform];
    if (typeof signature !== "string" || !signature.trim()) {
      throw new Error(`Missing updater signature for ${platform}.`);
    }
    platforms[platform] = {
      signature: signature.trim(),
      url: `https://github.com/${repository}/releases/download/${tag}/${updaterAssetName(platform, version)}`,
    };
  }
  const extra = Object.keys(signatures).filter((platform) => !UPDATER_PLATFORMS.includes(platform));
  if (extra.length > 0) throw new Error(`Unexpected updater platforms: ${extra.join(", ")}.`);

  return {
    version,
    notes: `https://github.com/${repository}/releases/tag/${tag}`,
    pub_date: date,
    platforms,
  };
}

function parseArgs(argv) {
  const options = { signatures: {} };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const value = argv[index + 1];
    if (arg === "--version" || arg === "--tag" || arg === "--repository" || arg === "--output") {
      if (value === undefined) throw new Error(`${arg} needs a value.`);
      options[arg.slice(2)] = value;
      index += 1;
    } else if (arg === "--signature") {
      const [platform, path] = (value ?? "").split("=");
      if (!platform || !path) throw new Error("--signature needs <platform>=<path-to-.sig>.");
      options.signatures[platform] = path;
      index += 1;
    } else {
      throw new Error(`Unknown argument ${arg}.`);
    }
  }
  if (!options.version || !options.tag || !options.output) {
    throw new Error("Usage: build-updater-manifest.mjs --version X.Y.Z --tag vX.Y.Z --output latest.json --signature <platform>=<file> ...");
  }
  return options;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const options = parseArgs(process.argv.slice(2));
  const packageVersion = JSON.parse(await readFile(resolve("package.json"), "utf8")).version;
  if (options.version !== packageVersion) {
    throw new Error(`--version ${options.version} does not match package.json (${packageVersion}).`);
  }
  const signatures = {};
  for (const [platform, path] of Object.entries(options.signatures)) {
    signatures[platform] = await readFile(path, "utf8");
  }
  const manifest = buildUpdaterManifest({
    version: options.version,
    tag: options.tag,
    repository: options.repository,
    signatures,
  });
  await writeFile(options.output, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`Wrote ${options.output} for ${manifest.version} (${Object.keys(manifest.platforms).join(", ")}).`);
}
