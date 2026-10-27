import assert from "node:assert/strict";
import { cp, mkdir, readdir, readFile, rm, stat, writeFile, lstat, symlink } from "node:fs/promises";
import { join } from "node:path";
import { performance } from "node:perf_hooks";
import { platform } from "node:os";
import { CANARIES, body, scenarioDocument, textScenario, transported, sha256 } from "./scenario.mjs";
import { codexServer, claudeTurn, startMock, runBounded, environment, codexArguments } from "./process.mjs";

export async function directories(root) {
  const result = {};
  await writeFile(join(root, "fixture-owner"), "mixed-aimock-v1\n", { mode: 0o600, flag: "wx" });
  for (const name of ["storage", "control", "snapshots", "process-home", "process-home/tmp"]) await mkdir(join(root, name), { recursive: true, mode: 0o700 });
  for (const [name, projectName] of [["a", "project-a"], ["b", "project B ü"], ["c", "project C longer 中文"]]) {
    const base = join(root, name);
    const replica = { name, base, project: join(base, projectName), codex: join(base, "codex"), claude: join(base, "claude"), home: join(base, "process-home") };
    for (const path of [replica.project, replica.codex, replica.claude, replica.home, join(replica.home, "tmp"), join(base, "machine")]) await mkdir(path, { recursive: true, mode: 0o700 });
    const text = `MIXED_READ_RESULT\nTOKEN=${CANARIES.key}\nDB_PASSWORD=${CANARIES.password}\n`;
    await writeFile(join(replica.project, "mixed-read.txt"), text, { mode: 0o600 });
    result[name] = replica;
  }
  return result;
}

export async function nativePreflight(replica) {
  if (process.env.MALLARD_MIXED_NETWORK_ISOLATED === "1") {
    assert.equal(platform(), "linux", "isolated test requires Linux");
    assert.deepEqual((await readdir("/sys/class/net")).sort(), ["lo"], "isolated test has a non-loopback interface");
  }
  const options = { cwd: replica.project, env: environment(replica.home, { CODEX_HOME: replica.codex, CLAUDE_CONFIG_DIR: replica.claude }) };
  const codex = (await runBounded("codex", ["--version"], options, 15_000)).trim();
  const claude = (await runBounded("claude", ["--version"], options, 15_000)).trim();
  assert.equal(codex, "codex-cli 0.164.0-alpha.11.1", "Codex version pin");
  assert.ok(claude === "2.1.266 (Claude Code)", "Claude version pin");
  const probe = process.platform === "win32" ? ["cmd.exe", "/d", "/c", "exit", "0"] : ["/usr/bin/true"];
  await runBounded("codex", ["sandbox", "--", ...probe], options, 15_000);
  // The graph-fault checks require a real file symlink on every platform.
  const target = join(replica.home, "symlink-probe-target");
  const link = join(replica.home, "symlink-probe-link");
  await writeFile(target, "fixture", { mode: 0o600, flag: "wx" });
  let linked = false;
  try {
    await symlink(target, link, "file"); linked = true;
    assert.ok((await lstat(link)).isSymbolicLink(), "file symlink preflight");
  } catch (error) {
    if (["EPERM", "EACCES"].includes(error.code)) throw new Error("native_file_symlink_permission_denied");
    throw error;
  } finally {
    if (linked) await rm(link);
    await rm(target);
  }
  return { codex, claude };
}

export class NativeDriver {
  constructor(repository, root, replicas) {
    Object.assign(this, { repository, root, replicas }); this.batch = 0; this.audits = []; this.sourceTurns = 0;
    this.sessionIds = new Map(); this.histories = new Map(); this.times = []; this.cutoffs = new Map();
  }
  async turns(replica, node, start, count, inherited = [], id = null, parentId = null, suffix = "SOURCE") {
    const scenarios = []; const history = inherited.map((entry) => ({ ...entry })); const turns = [];
    for (let turn = start; turn < start + count; turn++) {
      const prompt = `MIXED_${node.label}_USER_${suffix}_${turn}`;
      const marker = `MIXED_${node.label}_RESPONSE_${suffix}_${turn}`;
      const response = suffix === "SOURCE" ? body(marker, node.body_bytes, replica.project, this.replicas.b.project)
        : node.richAppend ? `${marker}\nProject file: ${replica.project}/new-file.txt\nURI: file://${replica.project}/new-file.txt\nTOKEN=${CANARIES.key}` : marker;
      const fixture = textScenario(node.provider, `case_${node.label.toLowerCase()}_${suffix.toLowerCase()}_${turn}`, prompt, history, response);
      const tool = suffix === "SOURCE" && turn === 1 && (node.tool || (node.provider === "codex" && Number(node.label.slice(1)) <= 12));
      if (tool) {
        const initial = structuredClone(fixture); initial.id += "_tool";
        const callId = `mixed_read_${node.label.toLowerCase()}`;
        initial.response = node.provider === "claude" ? { kind: "read_tool", call_id: callId, file_path: join(replica.project, "mixed-read.txt") }
          : { kind: "tool_call", name: "exec_command", call_id: callId, arguments: { cmd: `cat -- '${join(replica.project, "mixed-read.txt")}'`, workdir: replica.project } };
        scenarios.push(initial); fixture.require_tool_result = true; fixture.expected_tool_call_id = callId;
        if (node.provider === "claude") fixture.expected_tool_result_marker = "MIXED_READ_RESULT";
      }
      scenarios.push(fixture); turns.push({ prompt, response });
      history.push({ role: "user", marker: prompt }, { role: "assistant", marker: response });
    }
    const document = scenarioDocument(node.provider, scenarios, turns.length);
    const mock = await startMock(this.repository, join(this.root, "control", `batch-${++this.batch}`), node.provider, document);
    let server; const started = performance.now();
    try {
      if (node.provider === "codex") {
        server = await codexServer(replica, mock.url);
        if (id) await server.resume(id); else id = await server.create(parentId);
        for (const turn of turns) await server.turn(id, turn.prompt);
        await server.close(); server = null;
      } else {
        let fresh = !id;
        id ??= `018f3f2e-7b91-7ad0-8000-${String(Number(node.label.slice(1))).padStart(12, "0")}`;
        for (const turn of turns) { await claudeTurn(replica, mock.url, id, fresh, turn.prompt, turn.response); fresh = false; }
      }
      this.audits.push(await mock.finish());
      this.times.push({ provider: node.provider, operation: suffix === "SOURCE" ? "native_source" : "native_append", ms: performance.now() - started, turns: turns.length });
      if (suffix === "SOURCE") this.sourceTurns += turns.length;
      return { id, history };
    } catch (error) {
      error.privateDetail = `${node.label} ${suffix}: ${error.privateDetail ?? "native turn check"}`;
      throw error;
    } finally { server?.kill(); mock.kill(); }
  }
  async generate(nodes, onProgress) {
    const byLabel = new Map(nodes.map((node) => [node.label, node]));
    const generate = async (node) => {
      if (this.sessionIds.has(node.label)) return;
      const inherited = node.parent ? this.histories.get(node.parent) : [];
      assert.ok(inherited, "native parent missing");
      if (node.parent) this.cutoffs.set(node.label, await nativeCutoff(this.replicas.a.codex, this.sessionIds.get(node.parent)));
      const count = node.label === "C002" && nodes.some((n) => n.early) ? 2 : node.turns;
      let result = await this.turns(this.replicas.a, node, 1, count, inherited, null, node.parent ? this.sessionIds.get(node.parent) : null);
      this.sessionIds.set(node.label, result.id); this.histories.set(node.label, result.history);
      if (count !== node.turns) {
        for (const child of nodes.filter((n) => n.early)) await generate(child);
        result = await this.turns(this.replicas.a, node, 3, 2, result.history, result.id);
        this.histories.set(node.label, result.history);
      }
      onProgress?.(node.label, this.sessionIds.size);
    };
    for (const node of byLabel.values()) await generate(node);
    assert.equal(this.sessionIds.size, nodes.length);
    assert.equal(this.sourceTurns, nodes.reduce((sum, node) => sum + node.turns, 0));
    for (const node of nodes) {
      await assertStoredBodies(this.replicas.a, node.provider, this.sessionIds.get(node.label),
        this.histories.get(node.label).slice(-node.turns * 2));
    }
  }
  async reopen(replica, node, id, history, suffix) {
    const started = performance.now();
    // Native source forks store inherited bodies in separate bases. The strict
    // API oracle checks those histories after the native client resolves them.
    const graph = node.provider === "codex" ? await nativeCutoff(replica.codex, id) : null;
    if (graph) {
      assert.equal(graph.cwd, replica.project, "Codex transcript cwd was not remapped");
      if (replica.name !== "a") assert.equal(graph.base, null, "restored Codex history is not standalone");
    }
    if (!graph?.base) await assertStoredBodies(replica, node.provider, id, history);
    const snapshot = join(this.root, "snapshots", `resume-${++this.batch}`);
    await mkdir(snapshot, { mode: 0o700 });
    const copy = { ...replica, codex: join(snapshot, "codex"), claude: join(snapshot, "claude"), home: join(snapshot, "home") };
    await mkdir(copy.home, { mode: 0o700 }); await mkdir(join(copy.home, "tmp"), { mode: 0o700 });
    // Codex's SQLite index contains absolute rollout paths. A resume snapshot
    // must rebuild it from the copied JSONL files or it can write to the source.
    await copyProviderHome(node.provider === "codex" ? replica.codex : replica.claude, node.provider === "codex" ? copy.codex : copy.claude, node.provider, false);
    const authoritative = node.provider === "codex" ? join(replica.codex, "sessions") : join(replica.claude, "projects");
    const before = await digestFiles(authoritative);
    const prompt = `MIXED_${node.label}_USER_REOPEN_${suffix}`; const response = `MIXED_${node.label}_REOPEN_${suffix}`;
    const fixture = textScenario(node.provider, `reopen_${node.label.toLowerCase()}_${suffix.toLowerCase()}`, prompt, history, response);
    const mock = await startMock(this.repository, join(this.root, "control", `batch-${++this.batch}`), node.provider, scenarioDocument(node.provider, [fixture], 1));
    try {
      if (node.provider === "codex") {
        const output = await runBounded("codex", ["--ask-for-approval", "never", "exec", "--json", "--ignore-user-config", "--ignore-rules", "--skip-git-repo-check",
          "--model", "gpt-5.6-luna", ...codexArguments(mock.url), "--cd", replica.project, "--sandbox", "read-only", "resume", id, prompt],
        { cwd: replica.project, env: environment(copy.home, { CODEX_HOME: copy.codex }) });
        const events = output.trim().split("\n").map((line) => JSON.parse(line));
        assert.equal(events.filter((e) => e.type === "thread.started")[0]?.thread_id, id, "Codex resume identity");
        assert.equal(events.filter((e) => e.type === "item.completed" && e.item?.type === "agent_message")[0]?.item.text, response, "Codex resume response");
      } else await claudeTurn(copy, mock.url, id, false, prompt, response);
      this.audits.push(await mock.finish());
      assert.equal(await digestFiles(authoritative), before, "native snapshot resume changed authoritative files");
      this.times.push({ provider: node.provider, operation: "native_resume", ms: performance.now() - started });
    } finally { mock.kill(); await rm(snapshot, { recursive: true, force: true, maxRetries: 10 }); }
  }
}

export function relocatedHistory(history, source, destination) {
  return history.map((entry) => ({ ...entry, marker: entry.role === "assistant" ? transported(entry.marker, source, destination) : entry.marker }));
}

export async function files(root, limit = 20000) {
  const result = []; let entries = 0;
  const visit = async (directory, depth = 0) => {
    assert.ok(depth <= 32, "fixture directory depth");
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      if (++entries > limit) throw new Error("fixture_file_count");
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink()) throw new Error("fixture_symlink");
      if (entry.isDirectory()) await visit(path, depth + 1);
      else { assert.ok(entry.isFile(), "fixture special file"); result.push(path); }
    }
  };
  await visit(root); return result;
}

export async function digestFiles(root) {
  const entries = [];
  for (const path of await files(root)) {
    const metadata = await stat(path); assert.ok(metadata.size <= 100 * 1024 * 1024, "fixture file bound");
    entries.push([path.slice(root.length), sha256(await readFile(path))]);
  }
  return sha256(JSON.stringify(entries.sort(([a], [b]) => a.localeCompare(b))));
}

export async function auditStore(root, needles) {
  let bytes = 0; let count = 0;
  for (const path of await files(root)) {
    const metadata = await lstat(path); assert.ok(metadata.size <= 100 * 1024 * 1024, "storage object bound");
    const content = await readFile(path); bytes += content.length; count++;
    for (const needle of needles) assert.ok(!content.includes(Buffer.from(needle)), "portable credential leak");
  }
  return { bytes, files: count };
}

export async function copyProviderHome(source, destination, provider, includeState = true) {
  await mkdir(destination, { recursive: true, mode: 0o700 });
  for (const entry of await readdir(source, { withFileTypes: true })) {
    const allowed = provider === "codex"
      ? ["sessions", "archived_sessions", "session_index.jsonl"].includes(entry.name) || (includeState && /^state_\d+\.sqlite(?:-wal|-shm)?$/u.test(entry.name))
      : entry.name === "projects";
    if (!allowed) continue;
    assert.ok(!entry.isSymbolicLink(), "provider snapshot symlink");
    const from = join(source, entry.name); const to = join(destination, entry.name);
    if (entry.isDirectory()) await files(from);
    else assert.ok(entry.isFile() && (await stat(from)).size <= 100 * 1024 * 1024, "provider snapshot file");
    await cp(from, to, { recursive: true, dereference: false, errorOnExist: true });
    if (entry.isDirectory()) await files(to);
  }
}

export async function assertStoredBodies(replica, provider, id, history) {
  const base = provider === "codex" ? join(replica.codex, "sessions") : join(replica.claude, "projects");
  const candidates = (await files(base)).filter((path) => path.endsWith(`${id}.jsonl`));
  assert.equal(candidates.length, 1, "native transcript lookup");
  const text = await readFile(candidates[0], "utf8");
  const strings = [];
  const visit = (value) => {
    if (typeof value === "string") { if (value.startsWith("MIXED_")) strings.push(value); }
    else if (value && typeof value === "object") Object.values(value).forEach(visit);
  };
  for (const line of text.split("\n").filter(Boolean)) {
    const record = JSON.parse(line);
    if (provider === "claude" && Object.hasOwn(record, "cwd")) assert.equal(record.cwd, replica.project, "Claude transcript cwd was not remapped");
    visit(record);
  }
  for (const [index, entry] of history.entries()) {
    if (entry.role !== "assistant") continue;
    const marker = entry.marker.split("\n", 1)[0];
    const actual = strings.filter((value) => value.split("\n", 1)[0] === marker).sort((a,b) => b.length-a.length)[0];
    if (actual !== entry.marker) {
      let offset = 0; while (actual && offset < Math.min(actual.length, entry.marker.length) && actual[offset] === entry.marker[offset]) offset++;
      const error = new Error("restored_body_mismatch");
      error.privateDetail = JSON.stringify({ index, expected_bytes: Buffer.byteLength(entry.marker), actual_bytes: actual ? Buffer.byteLength(actual) : null, first_offset: offset,
        expected_labels: entry.marker.match(/<[a-z_-]+>/gu), actual_labels: actual?.match(/<[a-z_-]+>/gu),
        expected_destination_count: entry.marker.split(replica.project).length-1, actual_destination_count: actual?.split(replica.project).length-1 });
      throw error;
    }
  }
}

export async function nativeCutoff(home, id) {
  const candidates = (await files(join(home, "sessions"))).filter((path) => path.endsWith(`${id}.jsonl`));
  assert.equal(candidates.length, 1, "native graph lookup");
  const bytes = await readFile(candidates[0]);
  assert.equal(bytes.at(-1), 10, "native graph record boundary");
  const records = bytes.toString("utf8").trimEnd().split("\n").map((line) => JSON.parse(line));
  const meta = records[0].payload;
  for (let i = 1; i < records.length; i++) assert.equal(records[i].ordinal, records[i - 1].ordinal + 1, "native graph ordinal gap");
  return { path: candidates[0], bytes: bytes.length, records: records.length, ordinal: records.at(-1).ordinal + 1,
    digest: sha256(bytes), cwd: meta.cwd, base: meta.history_base ?? null, parent: meta.forked_from_id ?? null };
}

export async function verifyGraph(native, nodes, worker) {
  const codex = nodes.filter((node) => node.provider === "codex");
  const resolved = (await worker.call("graph", { ids: codex.map((node) => native.sessionIds.get(node.label)) })).value;
  let physicalBytes = 0; let resolvedBytes = 0; let maxDepth = 0;
  for (const node of codex) {
    const id = native.sessionIds.get(node.label);
    const disk = await nativeCutoff(native.replicas.a.codex, id);
    physicalBytes += disk.bytes;
    const actual = resolved.find((entry) => entry.id === id);
    const chain = []; let current = node;
    while (current) { chain.unshift(native.sessionIds.get(current.label)); current = nodes.find((n) => n.label === current.parent); }
    assert.deepEqual(actual.segments.map((s) => s.id), chain, "resolved native graph order");
    maxDepth = Math.max(maxDepth, chain.length - 1);
    resolvedBytes += actual.segments.reduce((sum, segment) => sum + segment.bytes, 0);
    if (node.parent) {
      const expected = native.cutoffs.get(node.label);
      assert.deepEqual(disk.base, { thread_id: native.sessionIds.get(node.parent), end_ordinal_exclusive: expected.ordinal, end_byte_offset: expected.bytes }, "native fork cutoff");
      const parentBytes = await readFile(expected.path);
      assert.equal(sha256(parentBytes.subarray(0, expected.bytes)), expected.digest, "native fork prefix changed");
    } else assert.equal(disk.base, null, "native root has a base");
  }
  return { edges: codex.filter((node) => node.parent).length, max_depth: maxDepth, physical_bytes: physicalBytes, resolved_bytes: resolvedBytes };
}
