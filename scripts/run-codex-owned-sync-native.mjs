// Native Codex -> Mallard Push/Pull -> cold native resume. Temporary homes only.
import { spawn, spawnSync } from "node:child_process";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { makeEnvironment, runCli, transcripts, codexArgs, sendResponses, toolResponse } from "./run-local-subagent-poc.mjs";

const script = fileURLToPath(import.meta.url);
const repository = resolve(dirname(script), "..");
const check = (condition, code) => { if (!condition) throw new Error(code); };
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

async function cargoBridge(root, sharedOmission) {
  // Cargo needs its installed toolchain. Only this fixture variable is added;
  // the test itself refuses non-temporary provider inputs.
  await new Promise((accept, reject) => {
    const child = spawn("cargo", ["test", "--manifest-path", join(repository, "src-tauri/Cargo.toml"), "--lib", "native_owned_subagents_transport_bridge", "--", "--ignored", "--nocapture"], {
      cwd: repository, env: { ...process.env, MALLARD_OWNED_NATIVE_ROOT: root, MALLARD_OWNED_NATIVE_OMIT: sharedOmission ? "1" : "0" }, stdio: ["ignore", "pipe", "pipe"],
    });
    let output = "";
    for (const stream of [child.stdout, child.stderr]) stream.on("data", (chunk) => { if (output.length < 65536) output += chunk; });
    child.on("error", () => reject(new Error("bridge_start_failed")));
    child.on("exit", (code) => {
      if (code === 0) accept();
      else {
        // This output contains fixture data only. Keep diagnostics in the
        // temporary run directory; the console gets the fixed failure code.
        import("node:fs/promises").then(({ writeFile }) => writeFile(join(root, "bridge.log"), output)).finally(() => reject(new Error("bridge_failed")));
      }
    });
  });
}

async function main() {
  const root = await mkdtemp("/private/tmp/mallard-owned-native-");
  const sharedOmission = process.argv.includes("--shared-home-omission");
  let omittedChildControlled = false;
  const sourceEnv = await makeEnvironment(join(root, "source"));
  const targetEnv = await makeEnvironment(join(root, "target"));
  if (sharedOmission) targetEnv.CODEX_HOME = sourceEnv.CODEX_HOME;
  const { LLMock } = await import("@copilotkit/aimock");
  const mock = new LLMock({ host: "127.0.0.1", port: 0, strict: true, logLevel: "silent", journalMaxEntries: 32 });
  let phase = "spawn";
  let requests = 0;
  let childFinished;
  const childDone = new Promise((accept) => { childFinished = accept; });
  let protocolError;
  mock.mount("/v1/responses", { async handleRequest(incoming, outgoing) {
    try {
      check(incoming.method === "POST" && !incoming.headers.authorization && !incoming.headers["x-api-key"], "unexpected_auth_or_method");
      const chunks = [];
      let size = 0;
      for await (const chunk of incoming) { size += chunk.length; check(size < 4 * 1024 * 1024, "request_limit"); chunks.push(chunk); }
      const raw = JSON.parse(Buffer.concat(chunks));
      check(raw.stream && raw.model === "gpt-5.4" && ++requests <= 32, "request_shape");
      let response;
      if (phase === "control") {
        if (raw.input.some((item) => item.type === "agent_message" && JSON.stringify(item.content).includes("MALLARD_NATIVE_CHILD_FOLLOWUP"))) {
          omittedChildControlled = sharedOmission;
          response = { content: "MALLARD_NATIVE_CHILD_RESUMED" };
        } else {
          const results = raw.input.filter((item) => item.type === "function_call_output");
          if (!results.some((item) => item.call_id === "native_followup")) {
            const tools = raw.tools?.find((tool) => tool.type === "namespace" && tool.name === "collaboration")?.tools ?? [];
            check(tools.some((tool) => tool.name === "followup_task"), "control_tool_missing");
            response = toolResponse("followup_task", { target: "/root/native_child", message: "MALLARD_NATIVE_CHILD_FOLLOWUP Continue in the same child." }, "native_followup");
          } else if (!results.some((item) => item.call_id === "native_control_wait")) {
            response = toolResponse("wait_agent", { timeout_ms: 10000 }, "native_control_wait");
          } else {
            if (!sharedOmission) check(raw.input.some((item) => item.type === "agent_message" && JSON.stringify(item.content).includes("MALLARD_NATIVE_CHILD_RESUMED")), "cold_child_control_failed");
            response = { content: "MALLARD_NATIVE_MAIN_RESUMED" };
          }
        }
      } else if (raw.input.some((item) => item.type === "agent_message" && JSON.stringify(item.content).includes("MALLARD_NATIVE_CHILD_PROMPT"))) {
        response = { content: "MALLARD_NATIVE_CHILD_RESULT" };
        childFinished();
      } else {
        const results = raw.input.filter((item) => item.type === "function_call_output");
        if (!results.some((item) => item.call_id === "native_spawn")) {
          response = toolResponse("spawn_agent", { task_name: "native_child", message: "MALLARD_NATIVE_CHILD_PROMPT Return the fixture result.", fork_turns: "all" }, "native_spawn");
        } else if (!results.some((item) => item.call_id === "native_wait")) {
          await childDone;
          response = toolResponse("wait_agent", { timeout_ms: 10000 }, "native_wait");
        } else {
          check(raw.input.some((item) => item.type === "agent_message" && JSON.stringify(item.content).includes("MALLARD_NATIVE_CHILD_RESULT")), "missing_child_result");
          response = { content: "MALLARD_NATIVE_MAIN_RESULT" };
        }
      }
      sendResponses(outgoing, response, requests);
    } catch (error) {
      protocolError = error.message;
      outgoing.writeHead(422, { "content-type": "application/json" }); outgoing.end('{"error":{"message":"native fixture mismatch"}}');
    }
    return true;
  }});
  try {
    check((await runCli("codex", ["--version"], sourceEnv, root)).trim() === "codex-cli 0.162.1", "codex_version");
    await mock.start();
    await runCli("codex", codexArgs(mock.url), sourceEnv, join(root, "source/project"), join(root, "spawn.log"));
    check(!protocolError, protocolError ?? "protocol_failed");
    const original = await transcripts(join(root, "source/config/sessions"));
    check(original.length === 2, "native_tree_size");
    const child = original.find((records) => records[0].payload.source?.subagent);
    check(Number.isInteger(child?.[0].payload.subagent_history_start_ordinal), "native_child_boundary");
    if (sharedOmission) {
      const sessions = join(root, "source/config/sessions");
      const relative = (await readdir(sessions, { recursive: true })).find((name) => name.endsWith(`-${child[0].payload.id}.jsonl`));
      const path = join(sessions, relative);
      const lines = (await readFile(path, "utf8")).trimEnd().split("\n");
      const meta = JSON.parse(lines[0]); delete meta.payload.subagent_history_start_ordinal;
      lines[0] = JSON.stringify(meta); await writeFile(path, `${lines.join("\n")}\n`);
    }
    const originalIds = new Set(original.map((records) => records[0].payload.id));
    const sourceState = async () => (await transcripts(join(root, "source/config/sessions"))).filter((records) => originalIds.has(records[0].payload.id));
    const sourceDigest = hash(JSON.stringify(await sourceState()));
    await cargoBridge(root, sharedOmission);
    const imported = JSON.parse(await readFile(join(root, "transport.json"), "utf8"));
    const mainBefore = await readFile(imported.main_path);
    const childSizes = await Promise.all(imported.children.map(async (child) => (await readFile(child.path)).length));
    const resumeArgs = (id, prompt) => {
      const args = codexArgs(mock.url).slice(0, -1);
      // `exec resume` accepts the same provider settings but owns its own
      // sandbox and cwd options through the global CLI.
      const sandbox = args.indexOf("--sandbox"); args.splice(sandbox, 2);
      args.splice(args.indexOf("exec") + 1, 0, "resume");
      return [...args, id, prompt];
    };
    // Codex rejects direct CLI resume of an unloaded v2 child. Resume the
    // parent in a fresh process, then address its existing child by task path.
    phase = "control";
    const resumedMain = await runCli("codex", resumeArgs(imported.main_id, "MALLARD_NATIVE_CONTROL_CHILD"), targetEnv, join(root, "target/project"), join(root, `${phase}.log`));
    check(!protocolError, protocolError ?? "protocol_failed");
    check(resumedMain.includes("MALLARD_NATIVE_MAIN_RESUMED"), "native_parent_resume_result");
    for (let index = 0; index < imported.children.length; index += 1) {
      check((await readFile(imported.children[index].path)).length > childSizes[index], "child_file_not_appended");
    }
    check((await readFile(imported.main_path)).length > mainBefore.length, "parent_file_not_appended");
    if (sharedOmission) {
      check(imported.children.length === 0 && !omittedChildControlled, "omitted_child_was_controlled");
    } else check((await transcripts(join(root, "target/config/sessions"))).length === 2, "unexpected_new_native_child");
    check(hash(JSON.stringify(await sourceState())) === sourceDigest, "source_tree_changed");
    console.log(JSON.stringify({ checks_passed: true, codex: "0.162.1", native_children: imported.children.length, shared_home_omission: sharedOmission, cold_main_resume: true, cold_child_resume: !sharedOmission, child_file_appended: !sharedOmission, omitted_child_control_blocked: sharedOmission, source_tree_unchanged: true, model_requests: requests }));
    if (process.argv.includes("--keep-fixture")) console.log(`Fixture: ${root}`);
    else await rm(root, { recursive: true, force: true });
  } catch (error) {
    console.error(`Native owned sync failed: ${/^[a-z0-9_]+$/u.test(error.message) ? error.message : "unexpected_error"}`);
    console.error(`Fixture diagnostics: ${root}`);
    process.exitCode = 1;
  } finally { await mock.stop(); }
}

// Strip provider credentials and endpoints before native clients and the mock.
// Retain toolchain paths for the isolated Rust bridge; never pass this
// environment to a model request.
if (process.argv[2] !== "--isolated-child") {
  const launch = await mkdtemp("/private/tmp/mallard-owned-launch-");
  try {
    const environment = await makeEnvironment(launch);
    for (const key of ["CARGO_HOME", "RUSTUP_HOME"]) if (process.env[key]) environment[key] = process.env[key];
    environment.CARGO_HOME ??= join(process.env.HOME, ".cargo");
    environment.RUSTUP_HOME ??= join(process.env.HOME, ".rustup");
    const result = spawnSync(process.execPath, [script, "--isolated-child", ...process.argv.slice(2)], { env: environment, cwd: repository, stdio: "inherit", timeout: 180000 });
    process.exitCode = result.status ?? 1;
  } finally { await rm(launch, { recursive: true, force: true }); }
} else await main();
