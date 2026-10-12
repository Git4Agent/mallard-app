// Compile and test the standalone AX probe against its own temporary fixture app.
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { mkdtemp, mkdir, readFile, realpath, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

if (process.platform !== "darwin") throw new Error("This POC requires macOS.");
const option = process.argv[2];
if (process.argv.length > 3 || (option && !option.startsWith("--electron="))) throw new Error("Unknown argument.");
const electron = option ? await realpath(option.slice("--electron=".length)) : undefined;
if (electron && (!electron.startsWith("/private/tmp/") || !electron.endsWith("/Electron.app/Contents/MacOS/Electron"))) {
  throw new Error("Use a separate, unpacked Electron runtime under /private/tmp.");
}
const repository = dirname(dirname(fileURLToPath(import.meta.url)));
const root = await mkdtemp("/private/tmp/mallard-accessibility-poc-");
const run = promisify(execFile);
const probe = join(root, "ax-probe");
const bundle = join(root, "Mallard Accessibility Fixture.app");
const executable = electron ?? join(bundle, "Contents/MacOS/fixture");
const bundleId = electron ? "com.github.Electron" : "io.mallard.accessibility-poc.fixture";
const env = { ...process.env, CLANG_MODULE_CACHE_PATH: join(root, "module-cache") };
delete env.ELECTRON_RUN_AS_NODE;
const targetID = "11111111-1111-4111-8111-111111111111";
const report = { poc: "native-thread-id", fixtureEngine: electron ? "Electron" : "WebKit",
  realCodexTested: false, passed: false, checks: [], scenarios: {} };
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let child;

async function invoke(args) {
  try {
    const result = await run(probe, args, { env, timeout: 25_000, maxBuffer: 64 * 1024 });
    return JSON.parse(result.stdout);
  } catch (error) {
    if (error.stdout?.trim()) return JSON.parse(error.stdout);
    throw error;
  }
}

async function fixtureState(path, minimumPresses = 0) {
  for (let count = 0; count < 60; count++) {
    try {
      const state = JSON.parse(await readFile(path, "utf8"));
      if (state.presses >= minimumPresses) return state;
    }
    catch (error) { if (error.code !== "ENOENT") throw error; }
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(`Fixture exited before it became ready (${child.signalCode ?? child.exitCode}).`);
    }
    await sleep(100);
  }
  throw new Error("Fixture did not report the expected state.");
}

async function stopFixture() {
  if (!child || child.exitCode !== null || child.signalCode !== null) { child = undefined; return; }
  const target = child;
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Fixture cleanup timeout.")), 5_000);
    target.once("exit", () => { clearTimeout(timer); resolve(); });
    target.kill("SIGTERM");
  });
  child = undefined;
}

try {
  await mkdir(dirname(executable), { recursive: true });
  const compile = (source, destination, frameworks) => run("xcrun", ["swiftc", "-module-cache-path",
    env.CLANG_MODULE_CACHE_PATH, source, ...frameworks.flatMap((name) => ["-framework", name]),
    "-o", destination], { env, cwd: repository, timeout: 120_000, maxBuffer: 1024 * 1024 });
  await compile("scripts/poc-codex-macos-accessibility.swift", probe, ["AppKit", "ApplicationServices"]);
  if (electron) {
    const plist = join(dirname(dirname(electron)), "Info.plist");
    await readFile(plist);
    const identity = await run("/usr/libexec/PlistBuddy", ["-c", "Print :CFBundleIdentifier", plist]);
    assert.equal(identity.stdout.trim(), bundleId);
  } else {
    await compile("tests/fixtures/codex-sidebar-accessibility.swift", executable, ["AppKit", "WebKit"]);
    await writeFile(join(bundle, "Contents/Info.plist"), `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>${bundleId}</string>
<key>CFBundleExecutable</key><string>fixture</string><key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleName</key><string>Mallard Accessibility Fixture</string></dict></plist>\n`);
  }
  report.checks.push("native_probe_compiled_and_fixture_ready");
  const unit = await invoke(["--mode=self-test"]);
  assert.equal(unit.ok, true);
  report.checks.push(...unit.checks);
  assert.equal((await invoke(["--mode=press", "--allow-title-match=true"])).code, "unknown_argument");
  report.checks.push("title_only_press_option_removed");
  const permission = await invoke(["--mode=permission"]);
  assert.equal(permission.ok, true);
  report.accessibilityTrusted = permission.trusted;
  if (!permission.trusted) {
    report.blocked = "accessibility_permission_required";
    process.exitCode = 2;
    for (const mode of ["inspect", "press", "diagnose", "enable-accessibility"]) {
      const refused = await invoke([`--mode=${mode}`]);
      assert.equal(refused.code, "accessibility_permission_required");
    }
    report.checks.push("permission_absent_all_app_modes_refused");
  }
  for (const scenario of ["data-only", "exact-id", "ancestor-id", "duplicate-id", "duplicate-project"]) {
    const statePath = join(root, `${scenario}.json`);
    const launchArgs = electron
      ? [join(repository, "tests/fixtures/codex-sidebar-accessibility-electron.cjs"), statePath, scenario,
        `--user-data-dir=${join(root, `electron-${scenario}`)}`]
      : [statePath, scenario];
    child = spawn(executable, launchArgs, { env, stdio: "ignore" });
    const before = await fixtureState(statePath);
    assert.equal(before.locationId, "chats");
    assert.equal(before.presses, 0);
    assert.equal(before.recentsPresent, true);
    assert.equal(before.buttonCount, scenario === "data-only" ? 2 : 3);
    if (electron) {
      report.electronVersion = before.electronVersion;
      assert.equal(before.accessibilityEnabled, false);
    } else {
      assert.equal(before.bundleIdentityMatches, true);
      assert.equal(before.workspaceIdentityMatches, true);
    }
    report.checks.push(`${scenario}_fixture_started`);
    if (!permission.trusted) {
      await stopFixture();
      continue;
    }
    const args = [`--pid=${child.pid}`, `--bundle-id=${bundleId}`, "--window-title=Mallard Accessibility Fixture",
      "--project-list-label=Scheduled tasks in Fixture project", "--chat-title=Fixture chat", `--thread-id=${targetID}`];
    // The Electron case enables Accessibility only in this owned, temporary fixture.
    if (electron) {
      const diagnosticArgs = ["--project-name=Fixture project", ...args];
      const cold = await invoke(["--mode=diagnose", ...diagnosticArgs]);
      assert.equal(cold.ok, true, JSON.stringify(cold));
      assert.equal(cold.accessibilityRequestSent, false);
      const enabled = await invoke(["--mode=enable-accessibility", ...diagnosticArgs]);
      assert.equal(enabled.ok, true, JSON.stringify(enabled));
      assert.equal(enabled.accessibilityAttribute, "AXEnhancedUserInterface");
      assert.equal(enabled.rendererExposed, true);
      assert.equal(enabled.pressSent, false);
      assert.equal((await fixtureState(statePath)).presses, 0);
      if (scenario === "data-only") {
        assert.equal(cold.rendererExposed, false);
        assert.equal(cold.matchingNodeCount, 0);
        assert.equal(enabled.accessibilityRequestSent, true);
        report.rendererActivation = { nodesBefore: cold.nodeCount, nodesAfter: enabled.nodeCount,
          roleCountsBefore: cold.roleCounts, roleCountsAfter: enabled.roleCounts, pressSent: enabled.pressSent,
          attribute: enabled.accessibilityAttribute, setResult: enabled.accessibilitySetResult };
        const repeated = await invoke(["--mode=enable-accessibility", ...diagnosticArgs]);
        assert.equal(repeated.accessibilityRequestSent, false);
        assert.equal(repeated.rendererExposed, true);
        report.checks.push("electron_renderer_enabled_without_press", "exposed_renderer_needs_no_enable_request");
        if (enabled.accessibilitySetResult !== 0) {
          assert.equal(enabled.accessibilitySetAcknowledged, false);
          report.checks.push("renderer_activation_observed_despite_setter_error");
        }
      }
    }
    let inspected;
    for (let attempt = 0; attempt < 30; attempt++) {
      inspected = await invoke(["--mode=inspect", ...args]);
      if (inspected.ok || !["project_list_missing", "window_missing_or_ambiguous"].includes(inspected.code)) break;
      await sleep(100);
    }
    const diagnosed = await invoke(["--mode=diagnose", "--project-name=Fixture project", ...args]);
    assert.equal(diagnosed.ok, true, JSON.stringify(diagnosed));
    assert.equal(diagnosed.pressSent, false);
    assert.equal(JSON.stringify(diagnosed).includes(targetID), false);
    report.scenarios[scenario] = { inspected, diagnosed };
    if (["exact-id", "ancestor-id"].includes(scenario)) {
      assert.equal(inspected.ok, true, JSON.stringify(inspected));
      assert.equal(inspected.exactThreadIdentityProven, true);
      assert.equal(inspected.projectButtonCount, 2);
      assert.equal(inspected.identitySources.some(source => source.attribute === "AXDOMIdentifier"
        && source.onButton === (scenario === "exact-id")), true);
      assert.equal(diagnosed.selectorResult, "matched");
      assert.equal(diagnosed.roleCounts.AXList, 2);
      const wrongLabelArgs = args.map((arg) => arg.startsWith("--project-list-label=")
        ? "--project-list-label=Fixture project中的已安排任务" : arg);
      const missingList = await invoke(["--mode=inspect", ...wrongLabelArgs]);
      assert.equal(missingList.code, "project_list_missing");
      const mismatch = await invoke(["--mode=diagnose", "--project-name=Fixture project", ...wrongLabelArgs]);
      assert.equal(mismatch.ok, true, JSON.stringify(mismatch));
      assert.equal(mismatch.selectorResult, "project_list_missing");
      assert.equal(mismatch.pressSent, false);
      assert.equal(mismatch.matchingNodes.some((row) =>
        row.role === "AXList" && row.projectNameInLabel && !row.exactListLabel), true);
      for (const label of ["Fixture project", "Fixture chat", "Recents"]) {
        assert.equal(JSON.stringify(mismatch).includes(label), false);
      }
      assert.equal((await fixtureState(statePath)).presses, 0);
      report.checks.push("native_diagnostics_explain_missing_label_without_press_or_label_output");
      const wrongIDArgs = args.map(arg => arg.startsWith("--thread-id=")
        ? "--thread-id=33333333-3333-4333-8333-333333333333" : arg);
      const wrongID = await invoke(["--mode=press", ...wrongIDArgs]);
      assert.equal(wrongID.code, "thread_id_not_found");
      assert.equal(wrongID.pressSent, false);
      assert.equal((await fixtureState(statePath)).presses, 0);
      const noID = await invoke(["--mode=press", ...args.filter(arg => !arg.startsWith("--thread-id="))]);
      assert.equal(noID.code, "target_arguments_missing");
      assert.equal(noID.pressSent, false);
      // No chat title is supplied to the successful press.
      const idOnlyArgs = args.filter(arg => !arg.startsWith("--chat-title="));
      const pressed = await invoke(["--mode=press", ...idOnlyArgs]);
      report.scenarios[scenario].pressed = pressed;
      assert.equal(pressed.ok, true, JSON.stringify(pressed));
      assert.equal(pressed.pressSent, true);
      assert.equal(pressed.exactThreadIdentityProven, true);
      assert.equal(pressed.currentAfter, "page");
      const after = await fixtureState(statePath, 1);
      report.scenarios[scenario].fixtureAfter = after;
      assert.equal(after.presses, 1);
      assert.equal(after.selectedThreadId, targetID);
      assert.equal(after.locationId, "unified-project:fixture-project");
      assert.equal(after.recentsPresent, true);
      assert.equal(after.buttonCount, before.buttonCount);
      const already = await invoke(["--mode=press", ...idOnlyArgs]);
      assert.equal(already.ok, true, JSON.stringify(already));
      assert.equal(already.alreadyCurrent, true);
      assert.equal(already.pressSent, false);
      assert.equal((await fixtureState(statePath)).presses, 1);
      report.checks.push(`${scenario}_native_thread_id_selected_without_title`, `${scenario}_recents_copy_retained`,
        `${scenario}_wrong_id_and_missing_id_refused`, `${scenario}_already_current_not_pressed_again`);
    } else {
      const expected = { "data-only": "thread_id_not_exposed", "duplicate-id": "thread_id_ambiguous",
        "duplicate-project": "project_list_ambiguous" }[scenario];
      assert.equal(inspected.code, expected, JSON.stringify(inspected));
      assert.equal(diagnosed.selectorResult, expected);
      assert.equal(diagnosed.exactThreadIdentityProven, false);
      const pressed = await invoke(["--mode=press", ...args]);
      report.scenarios[scenario].pressed = pressed;
      assert.equal(pressed.code, expected);
      assert.equal(pressed.pressSent, false);
      const after = await fixtureState(statePath);
      report.scenarios[scenario].fixtureAfter = after;
      assert.equal(after.presses, 0);
      report.checks.push(`${scenario}_refused_without_press`);
    }
    await stopFixture();
  }
  report.passed = permission.trusted;
} catch (error) {
  report.error = error.message;
  await writeFile(join(root, "failure.log"), `${error.stack}\n${error.stderr ?? ""}`);
  process.exitCode = 1;
} finally {
  try { await stopFixture(); } catch (error) {
    report.cleanupError = error.message;
    report.passed = false;
    process.exitCode = 1;
  }
  await writeFile(join(root, "report.json"), JSON.stringify(report, null, 2) + "\n");
  const { scenarios, ...summary } = report;
  console.log(JSON.stringify({ report: join(root, "report.json"), ...summary,
    scenarios: Object.fromEntries(Object.entries(scenarios).map(([name, result]) => [name, {
      selectorResult: result.diagnosed.selectorResult,
      exactThreadIdentityProven: result.diagnosed.exactThreadIdentityProven,
      pressSent: result.pressed?.pressSent, currentAfter: result.pressed?.currentAfter,
      fixturePresses: result.fixtureAfter?.presses, recentsPresent: result.fixtureAfter?.recentsPresent,
    }])) }, null, 2));
}
