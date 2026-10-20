// POC only: Pi SDK -> Mallard capture/path mapper -> Pi SDK reopen/append.
// Installs a pinned SDK in a disposable directory; no application dependency.
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { isDeepStrictEqual } from "node:util";

const packageName = "@earendil-works/pi-coding-agent";
const version = "1.3.0";
const script = fileURLToPath(import.meta.url);
const repository = resolve(dirname(script), "..");
const testName = "providers::pi_poc::pi_poc_native_sdk_roundtrip";
const marker = "mallard-pi-synthetic-only-v1\n";
const timestamp = 1792022400000;

function check(condition, code) {
  if (!condition) throw new Error(code);
}

async function run(command, args, options, label, timeout = 120_000) {
  return await new Promise((accept, reject) => {
    const child = spawn(command, args, { ...options, stdio: ["ignore", "pipe", "pipe"] });
    let output = "";
    let bytes = 0;
    let failure;
    const timer = setTimeout(() => {
      failure = `${label}: timeout`;
      child.kill("SIGKILL");
    }, timeout);
    for (const stream of [child.stdout, child.stderr]) {
      stream.on("data", (chunk) => {
        bytes += chunk.length;
        if (bytes > 4 * 1024 * 1024) {
          failure = `${label}: output limit`;
          child.kill("SIGKILL");
        } else output += chunk.toString("utf8");
      });
    }
    child.on("error", () => {
      clearTimeout(timer);
      reject(new Error(`${label}: could not start`));
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      const assertion = output.match(/^Pi POC failed: ([a-z_]+)$/m)?.[1];
      if (failure || code !== 0) reject(new Error(failure ?? `${label}: ${assertion ?? `exit ${code}`}`));
      else accept(output);
    });
  });
}

function environment(root, agent) {
  // An allowlist prevents inherited provider credentials, proxies, or resources
  // from entering the SDK process. No agent, extension, tool, or model is run.
  return {
    PATH: process.env.PATH,
    ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
    HOME: join(root, "home"), USERPROFILE: join(root, "home"),
    XDG_CONFIG_HOME: join(root, "home", "config"),
    TMPDIR: join(root, "tmp"), TEMP: join(root, "tmp"), TMP: join(root, "tmp"),
    PI_CODING_AGENT_DIR: join(root, agent),
    npm_config_cache: join(root, "npm-cache"),
  };
}

async function sdk(root, directory) {
  check(await readFile(join(root, "poc-marker"), "utf8") === marker, "fixture_marker");
  const manifest = JSON.parse(await readFile(join(directory, "package.json"), "utf8"));
  check(manifest.name === packageName && manifest.version === version, "sdk_version");
  // Import only Pi's shipped session manager: no resource discovery or auth.
  return await import(pathToFileURL(join(directory, "dist/core/session-manager.js")).href);
}

function user(content) {
  return { role: "user", content, timestamp };
}

function assistant(content) {
  return {
    role: "assistant", content, timestamp, api: "openai-completions",
    provider: "poc", model: "fixture", stopReason: "stop",
    usage: {
      input: 1, output: 1, cacheRead: 0, cacheWrite: 0, totalTokens: 2,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
  };
}

async function seed(root, directory) {
  const { SessionManager } = await sdk(root, directory);
  const project = join(root, "source-project");
  const session = SessionManager.create(project, undefined, { id: "mallard-pi-poc" });
  session.appendModelChange("poc", "fixture");
  session.appendThinkingLevelChange("low");
  const start = session.appendMessage(user(`Inspect ${project}/src/main.ts`));
  session.appendMessage(assistant([{ type: "toolCall", id: "tool-poc", name: "read", arguments: { path: `${project}/src/main.ts` } }]));
  session.appendMessage({
    role: "toolResult", toolCallId: "tool-poc", toolName: "read", timestamp,
    content: [{ type: "text", text: "synthetic source" }], isError: false,
  });
  session.appendMessage(assistant([{ type: "text", text: "This is the abandoned branch." }]));
  const abandoned = session.getLeafId();
  session.branchWithSummary(start, "Read the synthetic source on the other branch.");
  const kept = session.appendMessage(user("Keep this branch."));
  session.appendMessage(assistant([{ type: "thinking", thinking: "synthetic reasoning" }, { type: "text", text: "Active branch answer." }]));
  session.appendCompaction("Synthetic compaction summary.", kept, 100, { readFiles: [`${project}/src/main.ts`] });
  session.appendCustomEntry("mallard-poc", { count: 42, path: `${project}/src/main.ts` });
  session.appendCustomMessageEntry("mallard-poc", "Synthetic extension context.", true);
  session.appendContextEdit(kept, { content: "Keep this edited branch context." });
  session.appendLabelChange(start, "poc-start");
  session.appendSessionInfo("Mallard Pi feasibility POC");
  session.appendMessage(user("Continue after compaction."));
  session.appendMessage(assistant([{ type: "text", text: "Ready to relocate." }]));

  const foreign = SessionManager.create(join(root, "other-project"), undefined, { id: "foreign-pi-poc" });
  foreign.appendMessage(user("Must stay outside the selected project."));
  foreign.appendMessage(assistant([{ type: "text", text: "Foreign session." }]));
  await writeFile(join(root, "source-agent", "auth.json"), '{"poc":"excluded"}\n');
  await writeFile(join(root, "source-agent", "settings.json"), '{"poc":"excluded"}\n');
  await writeFile(join(project, ".pi", "settings.json"), '{"poc":"excluded"}\n');
  await writeFile(join(root, "expected.json"), JSON.stringify({
    // Only synthetic data in temporary storage; never printed or committed.
    sourceFile: session.getSessionFile(), start, abandoned,
    sourceBytes: await readFile(session.getSessionFile(), "utf8"),
    entries: session.getEntries(), tree: session.getTree(),
    context: session.buildSessionContext(), leaf: session.getLeafId(),
  }));
}

async function verify(root, directory) {
  const { SessionManager, getDefaultSessionDir } = await sdk(root, directory);
  const expected = JSON.parse(await readFile(join(root, "expected.json"), "utf8"));
  const project = join(root, "destination-project");
  const source = join(root, "source-project");
  const normalize = (value) => JSON.parse(JSON.stringify(value));
  // A separate oracle for this fixture's simple in-project path spellings.
  const relocated = (value) => JSON.parse(JSON.stringify(value).replaceAll(source, project));
  const list = await SessionManager.list(project);
  check(list.length === 1 && list[0].id === "mallard-pi-poc", "pi_native_discovery");
  check(dirname(list[0].path) === getDefaultSessionDir(project), "pi_native_bucket");
  check(list[0].name === "Mallard Pi feasibility POC", "pi_native_title");
  const session = SessionManager.open(list[0].path);
  check(session.getCwd() === project, "pi_native_destination_cwd");
  check(session.getLeafId() === expected.leaf, "pi_native_leaf");
  check(isDeepStrictEqual(normalize(session.getEntries()), relocated(expected.entries)), "pi_all_records_preserved");
  check(isDeepStrictEqual(normalize(session.getTree()), relocated(expected.tree)), "pi_all_branches_preserved");
  check(isDeepStrictEqual(normalize(session.buildSessionContext()), relocated(expected.context)), "pi_context_preserved");
  check(session.getLabel(expected.start) === "poc-start", "pi_label_preserved");
  session.branch(expected.abandoned);
  check(session.getBranch().at(-1).id === expected.abandoned, "pi_abandoned_branch_reachable");
  session.branch(expected.leaf);
  const continuation = session.appendMessage(user("Continued on machine B."));
  session.appendMessage(assistant([{ type: "text", text: "Native append succeeded." }]));
  const reopened = SessionManager.open(list[0].path);
  check(reopened.getEntry(continuation)?.parentId === expected.leaf, "pi_continuation_parent");
  check(reopened.getEntries().length === expected.entries.length + 2, "pi_continuation_persisted");
  check(await readFile(expected.sourceFile, "utf8") === expected.sourceBytes, "pi_source_unchanged");
  console.log(JSON.stringify({
    pi: version, schema: 3, capturedSessions: 1, excludedForeignSessions: 1,
    preservedEntries: expected.entries.length, allBranchesPreserved: true,
    contextPreserved: true, nativeDiscovery: true, nativeContinuation: true,
    productionPushPull: "not implemented",
  }));
}

async function main() {
  if (process.argv[2] === "--seed") return await seed(process.argv[3], process.argv[4]);
  if (process.argv[2] === "--verify") return await verify(process.argv[3], process.argv[4]);
  check(process.argv.length === 2, "usage: node scripts/run-pi-harness-poc.mjs");
  const root = await realpath(await mkdtemp(join(tmpdir(), "mallard-pi-poc-")));
  try {
    for (const name of ["home", "tmp", "sdk", "source-agent", "destination-agent", "source-project/.pi", "destination-project", "other-project"]) {
      await mkdir(join(root, name), { recursive: true, mode: 0o700 });
    }
    await writeFile(join(root, "poc-marker"), marker, { mode: 0o600 });
    await writeFile(join(root, "sdk", "package.json"), '{"private":true,"type":"module"}\n');
    console.log(`Installing isolated ${packageName}@${version}...`);
    await run(process.platform === "win32" ? "npm.cmd" : "npm", [
      "install", "--prefix", join(root, "sdk"), "--ignore-scripts", "--no-package-lock",
      "--no-audit", "--no-fund", "--loglevel=error", `${packageName}@${version}`,
    ], { cwd: root, env: environment(root, "source-agent") }, "Pi SDK installation");
    const directory = join(root, "sdk", "node_modules", packageName);
    await run(process.execPath, [script, "--seed", root, directory], {
      cwd: root, env: environment(root, "source-agent"),
    }, "Pi fixture creation");
    console.log("Checking Mallard discovery, capture, and path relocation...");
    const cargoResult = await run("cargo", ["test", "-p", "mallard-infrastructure", "--lib", testName, "--", "--ignored", "--exact"], {
      cwd: repository, env: { ...process.env, MALLARD_PI_POC_ROOT: root },
    }, "Mallard Pi POC test (run cargo test -p mallard-infrastructure --lib pi_poc for diagnostics)", 600_000);
    check(cargoResult.includes("1 passed; 0 failed"), "native_cargo_test_did_not_run");
    const result = await run(process.execPath, [script, "--verify", root, directory], {
      cwd: root, env: environment(root, "destination-agent"),
    }, "Pi native reopen and continuation");
    console.log(result.trim());
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

try {
  await main();
} catch (error) {
  // Keep temporary paths and transcript contents out of console output.
  console.error(`Pi POC failed: ${error.message}`);
  process.exitCode = 1;
}
