import { constants } from "node:fs";
import { open, realpath } from "node:fs/promises";
import { resolve } from "node:path";
import { parseArgs } from "node:util";
import { pathToFileURL } from "node:url";
import { configuration } from "./auth.mjs";
import { sharingUrl } from "./graph.mjs";
import { PocError, json, requireValue } from "./http.mjs";
import { runProbe } from "./probe.mjs";

export function parseConfiguration(value) {
  requireValue(value && Object.keys(value).length === 3
    && Object.keys(value).every((key) => ["clientId", "tenant", "sharingUrl"].includes(key)),
  "invalid_configuration");
  return { ...configuration(value), sharingUrl: sharingUrl(value.sharingUrl) };
}

export async function readConfiguration(file) {
  let handle;
  try {
    requireValue(typeof file === "string" && file.length > 0, "configuration_required");
    const path = resolve(file);
    requireValue(await realpath(path) === path, "configuration_must_be_canonical");
    handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0));
    const before = await handle.stat();
    requireValue(before.isFile() && before.size > 0 && before.size <= 16 * 1024, "invalid_configuration_file");
    requireValue(process.platform === "win32" || (before.mode & 0o077) === 0, "configuration_must_be_private");
    const bytes = Buffer.alloc(16 * 1024 + 1);
    const { bytesRead } = await handle.read(bytes, 0, bytes.length, 0);
    const after = await handle.stat();
    requireValue(bytesRead === before.size && before.size === after.size && before.mtimeMs === after.mtimeMs
      && before.ctimeMs === after.ctimeMs && before.ino === after.ino && before.dev === after.dev,
    "configuration_changed");
    return parseConfiguration(json(bytes.subarray(0, bytesRead)));
  } catch (error) {
    if (error instanceof PocError) throw error;
    throw new PocError("configuration_unreadable");
  } finally { await handle?.close(); }
}

const HELP = `Microsoft 365 / SharePoint shared-folder POC (Node 22+)

  npm start -- --config ./config.local.json
  npm start -- --config ./config.local.json --write-probe

Default: two browser sign-ins, forced renewal, same-root and listing checks.
Requests User.Read and Files.ReadWrite.All in both modes. Inspect sends only Graph GETs.
--write-probe additionally leaves three synthetic files in the shared test folder.
No deletions, permission changes, link redemption, or production Mallard integration.
Register http://localhost/microsoft-shared-folder/callback as a native redirect.
See README.md for app registration, config, test users, and the limits of a PASS.`;

export async function main(argv = process.argv.slice(2)) {
  let values;
  try {
    ({ values } = parseArgs({ args: argv, options: {
      config: { type: "string" }, "write-probe": { type: "boolean", default: false },
      help: { type: "boolean", default: false },
    } }));
  } catch { throw new PocError("invalid_arguments"); }
  if (values.help) { console.log(HELP); return; }
  const config = await readConfiguration(values.config);
  console.error(values["write-probe"]
    ? "Write probe: synthetic files will remain in the configured test folder."
    : "Inspect only: Graph file writes are disabled; write permission is still requested at sign-in.");
  const result = await runProbe(config, { writeProbe: values["write-probe"], progress: (message) => console.error(message) });
  console.log(JSON.stringify(result, null, 2));
  if (result.verdict !== "PASS") process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(JSON.stringify({ verdict: "FAIL", failureCode: error instanceof PocError ? error.code : "poc_failed" }));
    process.exitCode = 1;
  });
}
