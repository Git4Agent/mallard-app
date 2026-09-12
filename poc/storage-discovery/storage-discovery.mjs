import { pathToFileURL } from "node:url";
import { authorizeDropbox, wranglerAuthorization } from "./auth.mjs";
import { discoverDropbox, discoverR2, publicReport } from "./discovery.mjs";
import { PocError, requireValue } from "./http.mjs";

export const HELP = `Read-only Mallard storage discovery POC. Node 20.19 or later.

  node poc/storage-discovery/storage-discovery.mjs mock
  node poc/storage-discovery/storage-discovery.mjs r2 --wrangler [--jurisdiction default|eu|us|fedramp]
  node poc/storage-discovery/storage-discovery.mjs r2 --env [--jurisdiction default|eu|us|fedramp]
  node poc/storage-discovery/storage-discovery.mjs dropbox --browser
  node poc/storage-discovery/storage-discovery.mjs dropbox --env

R2 environment input: MALLARD_DISCOVERY_CF_TOKEN and MALLARD_DISCOVERY_CF_ACCOUNT_ID.
Wrangler uses its current OAuth login. The account environment variable can select one account.
Dropbox environment input: MALLARD_DISCOVERY_DROPBOX_TOKEN.
Dropbox browser consent requests only account_info.read, files.metadata.read, files.content.read, sharing.read.
Optional public app ID: MALLARD_DISCOVERY_DROPBOX_APP_KEY.

Output omits credentials, names, paths, and destination IDs. It never saves Mallard configuration.
Exit 0: complete scan (including no matches). Exit 2: partial scan. Exit 1: failed to start.
`;

export async function main(args, env = process.env) {
  if (args.length === 0 || args.length === 1 && ["--help", "help"].includes(args[0])) {
    process.stdout.write(HELP);
    return 0;
  }
  const [provider, auth, flag, jurisdiction] = args;
  if (provider === "mock") {
    requireValue(args.length === 1, "invalid_arguments");
    const { runMock } = await import("./mock.mjs");
    process.stdout.write(`${JSON.stringify(await runMock(), null, 2)}\n`);
    return 0;
  }
  let result;
  if (provider === "r2") {
    requireValue(["--wrangler", "--env"].includes(auth) && (args.length === 2
      || args.length === 4 && flag === "--jurisdiction"), "invalid_arguments");
    const credentials = auth === "--wrangler" ? await wranglerAuthorization(env.MALLARD_DISCOVERY_CF_ACCOUNT_ID)
      : { token: env.MALLARD_DISCOVERY_CF_TOKEN, accountId: env.MALLARD_DISCOVERY_CF_ACCOUNT_ID };
    result = await discoverR2({ ...credentials, jurisdiction: jurisdiction ?? "default" });
  } else if (provider === "dropbox") {
    requireValue(args.length === 2 && ["--browser", "--env"].includes(auth), "invalid_arguments");
    if (auth === "--browser") process.stderr.write("Complete Dropbox read consent in the system browser. No storage will be changed.\n");
    const token = auth === "--browser" ? await authorizeDropbox({ appKey: env.MALLARD_DISCOVERY_DROPBOX_APP_KEY })
      : env.MALLARD_DISCOVERY_DROPBOX_TOKEN;
    result = await discoverDropbox({ token });
  } else {
    throw new PocError("invalid_arguments");
  }
  const report = publicReport(result, "live");
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  return report.verdict === "SCAN_COMPLETE" ? 0 : 2;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).then((code) => { process.exitCode = code; }).catch((error) => {
    process.stderr.write(`${JSON.stringify({ verdict: "SCAN_FAILED",
      code: error instanceof PocError ? error.code : "discovery_failed",
      ...(error instanceof PocError && error.scopeEvidence ? { scopeEvidence: error.scopeEvidence } : {}) })}\n`);
    process.exitCode = 1;
  });
}
