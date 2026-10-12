// Built-in-only tests. --native launches and targets its own fixture process.
ObjC.import("Foundation");

function run(args) {
  var deadlineCases = ["--blocked-read", "--blocked-press", "--deadline-cancel"];
  if (args.length > 1 || (args.length === 1 && args[0] !== "--native" && deadlineCases.indexOf(args[0]) < 0)) {
    throw new Error("invalid_test_arguments");
  }
  var fm = $.NSFileManager.defaultManager;
  var root = ObjC.unwrap(fm.currentDirectoryPath);
  function read(path) {
    return ObjC.unwrap($.NSString.stringWithContentsOfFileEncodingError(path, $.NSUTF8StringEncoding, Ref()));
  }
  var source = read(root + "/scripts/poc-codex-system-events.js");
  var api = eval("(function(){var Application=function(){throw new Error('real_app_forbidden')};\n" + source +
    "\nreturn {parseOptions:parseOptions,identityID:identityID,runProbe:runProbe,systemEventsAdapter:systemEventsAdapter," +
    "withHardDeadline:withHardDeadline,progressReporter:progressReporter};})()");
  var passed = [];
  function assert(value, name) {
    if (!value) throw new Error(name);
    passed.push(name);
  }
  var thread = "11111111-1111-4111-8111-111111111111";
  var other = "22222222-2222-4222-8222-222222222222";
  var label = "Scheduled tasks in Fixture project";
  var options = { mode: "press", threadId: thread, projectListLabel: label };
  function node(role, attrs, children) {
    return { role: role, attrs: attrs || {}, children: children || [], actions: role === "AXButton" ? ["AXPress"] : [] };
  }
  function fixture() {
    var recents = node("AXButton", { AXDOMIdentifier: thread, AXEnabled: true, AXARIACurrent: "page" });
    var row = node("AXButton", { AXDOMIdentifier: thread, AXEnabled: true, AXTitle: "Same title" });
    var sibling = node("AXButton", { AXDOMIdentifier: other, AXEnabled: true, AXTitle: "Same title" });
    var wrapper = node("AXGroup", {}, [row]);
    var project = node("AXList", { AXDescription: label }, [wrapper, sibling]);
    var navigation = node("AXGroup", { AXSubrole: "AXLandmarkNavigation" },
      [node("AXList", { AXDescription: "Recents" }, [recents]), project]);
    var main = node("AXGroup", { AXSubrole: "AXLandmarkMain" });
    var web = node("AXWebArea", {}, [main, navigation]);
    var win = node("AXWindow", {}, [web]);
    var state = { row: row, sibling: sibling, wrapper: wrapper, project: project, web: web, recents: recents,
      navigation: navigation, main: main, childReads: [], reads: [], presses: 0, snapshots: 0 };
    state.adapter = {
      window: win, windowCount: 1, checkBudget: function () {},
      ensureTarget: function () { state.snapshots++; },
      role: function (ref) { return ref.role; },
      subrole: function (ref) { return ref.attrs.AXSubrole; },
      childInfo: function (ref) {
        state.childReads.push(ref);
        return ref.children.map(function (child) {
          return { ref: child, role: child.role, subrole: child.attrs.AXSubrole };
        });
      },
      attribute: function (ref, name) { state.reads.push({ ref: ref, name: name }); return ref.attrs[name]; },
      attributeValues: function (ref, names) {
        return names.map(function (name) { return state.adapter.attribute(ref, name); });
      },
      attributes: function (ref, names) {
        var result = {};
        names.forEach(function (name) {
          var value = state.adapter.attribute(ref, name);
          if (value !== undefined) result[name] = value;
        });
        return result;
      },
      actionNames: function (ref) { return ref.actions; },
      press: function (ref) {
        state.presses++;
        ref.attrs.AXARIACurrent = "page";
        delete recents.attrs.AXARIACurrent;
      },
      pause: function () {}
    };
    return state;
  }
  function probe(state, opts) { return api.runProbe(opts || options, function () { return state.adapter; }); }
  // These child-only cases never create an Application or use System Events.
  if (deadlineCases.indexOf(args[0]) >= 0) {
    if (args[0] === "--deadline-cancel") {
      var completed = api.withHardDeadline(1, function () { return true; });
      $.NSThread.sleepForTimeInterval(1.2);
      return JSON.stringify({ completed: completed });
    }
    var blocked = fixture();
    function block() { $.NSThread.sleepForTimeInterval(60); }
    if (args[0] === "--blocked-read") blocked.adapter.role = block;
    else blocked.adapter.press = block;
    return api.withHardDeadline(1, function () {
      return JSON.stringify(api.runProbe(options, function () { return blocked.adapter; },
        api.progressReporter(function (line) { console.log(line); }, Date.now)));
    });
  }
  function refused(name, change, code) {
    var state = fixture();
    change(state);
    var result = probe(state);
    assert(!result.ok && result.code === code && !result.pressSent && state.presses === 0, name);
  }
  var state = fixture();
  var result = probe(state, Object.assign({}, options, { mode: "diagnose" }));
  assert(result.ok && result.selectorResult === "matched" && state.presses === 0, "diagnose sends no press");
  result = probe(state);
  assert(result.ok && result.code === "selected" && result.projectRowSelected && state.presses === 1,
    "exact ID selects project occurrence among equal titles");
  assert(state.navigation.children[0].children[0] === state.recents, "Recents copy remains");
  assert(!state.reads.some(function (read) { return read.name === "AXValue" ||
    (read.ref === state.row && read.name === "AXTitle") ||
    (read.ref === state.recents && ["AXIdentifier", "AXDOMIdentifier", "AXURL"].indexOf(read.name) >= 0); }),
    "no transcript values, chat titles, or Recents identity reads");
  result = probe(state);
  assert(result.ok && result.code === "already_selected" && !result.pressSent && state.presses === 1,
    "already selected row sends no extra press");
  refused("data attribute is not AX identity", function (s) {
    delete s.row.attrs.AXDOMIdentifier; delete s.sibling.attrs.AXDOMIdentifier;
    s.row.attrs["data-app-action-sidebar-thread-id"] = "local:" + thread;
  }, "thread_id_not_exposed");
  refused("wrong ID", function (s) { s.row.attrs.AXDOMIdentifier = other; }, "thread_id_not_found");
  refused("ID in title is refused", function (s) {
    s.row.attrs.AXDOMIdentifier = other; s.row.attrs.AXTitle = thread;
  }, "thread_id_not_found");
  refused("duplicate ID", function (s) { s.sibling.attrs.AXDOMIdentifier = thread; }, "thread_id_ambiguous");
  refused("duplicate project label", function (s) {
    s.navigation.children.push(node("AXList", { AXDescription: label }));
  }, "project_list_ambiguous");
  refused("missing navigation does not scan the whole window", function (s) {
    delete s.navigation.attrs.AXSubrole;
  }, "navigation_missing");
  refused("duplicate navigation is refused", function (s) {
    s.web.children.push(node("AXGroup", { AXSubrole: "AXLandmarkNavigation" }));
  }, "navigation_ambiguous");
  ["AXLandmarkRegion", "AXLandmarkComplementary", "AXLandmarkBanner"].forEach(function (subrole) {
    var enclosed = fixture();
    enclosed.web.children = [node("AXGroup", { AXSubrole: subrole }, [enclosed.main, enclosed.navigation])];
    var outcome = probe(enclosed);
    assert(outcome.ok && outcome.code === "selected" && enclosed.presses === 1 &&
      enclosed.childReads.indexOf(enclosed.main) < 0,
      "navigation inside a container landmark remains reachable: " + subrole);
  });
  state = fixture();
  state.main.children.push(state.navigation);
  state.web.children = [state.main];
  result = probe(state);
  assert(!result.ok && result.code === "navigation_missing" && !result.pressSent && state.presses === 0 &&
    result.navigationOutline.some(function (entry) {
      return entry.subrole === "AXLandmarkMain" && entry.depth === 2 && entry.decision === "skip_main";
    }), "missing navigation reports the excluded main container without pressing");
  state = fixture();
  state.navigation.attrs.AXSubrole = "private container label";
  for (var outlineIndex = 0; outlineIndex < 100; outlineIndex++) state.web.children.push(node("AXGroup"));
  result = probe(state);
  assert(result.navigationOutline.length === 80 && result.navigationOutlineTruncated &&
    JSON.stringify(result).indexOf("private container label") < 0 &&
    JSON.stringify(result).indexOf(thread) < 0 && JSON.stringify(result).indexOf(label) < 0,
    "navigation outline is bounded and contains no labels or identity values");
  state = fixture();
  state.main.children.push(node("AXList", { AXDescription: label }, [node("AXButton", { AXDOMIdentifier: thread })]));
  for (var n = 0; n < 10000; n++) state.main.children.push(node("AXGroup", {}, [node("AXButton")]));
  var otherProject = node("AXList", { AXDescription: "Scheduled tasks in Other project" });
  for (var p = 0; p < 5000; p++) otherProject.children.push(node("AXGroup", {}, [node("AXButton")]));
  state.navigation.children.unshift(otherProject);
  result = probe(state);
  assert(result.ok && state.presses === 1 && result.nodeCount < 20 && state.childReads.indexOf(state.main) < 0 &&
    state.childReads.indexOf(otherProject) < 0 && state.childReads.indexOf(state.recents) < 0,
    "large chat and other-project subtrees are never traversed");
  state = fixture();
  var nestedProject = node("AXGroup", {}, [node("AXGroup", {}, [state.project])]);
  state.navigation.children[1] = node("AXList", { AXDescription: "Projects" }, [nestedProject]);
  result = probe(state);
  assert(result.ok && result.code === "selected", "project discovery crosses the outer Projects list");
  state = fixture();
  var budgetChecks = 0;
  state.adapter.checkBudget = function () {
    if (++budgetChecks > 8) { var error = new Error("probe_timeout"); error.pocCode = "probe_timeout"; throw error; }
  };
  result = probe(state);
  assert(!result.ok && result.code === "probe_timeout" && result.nodeCount > 0 && result.scanPhase &&
    result.roleCounts.AXWebArea === 1 && result.rendererExposed && !result.pressSent,
    "timeout retains partial scan phase and node counts");
  refused("project label substring", function (s) { s.project.attrs.AXDescription += " extra"; }, "project_list_missing");
  refused("nested action identity", function (s) {
    delete s.row.attrs.AXDOMIdentifier; delete s.sibling.attrs.AXDOMIdentifier;
    s.row.children.push(node("AXButton", { AXIdentifier: thread }));
  }, "thread_id_not_exposed");
  refused("nested list identity", function (s) {
    delete s.row.attrs.AXDOMIdentifier; delete s.sibling.attrs.AXDOMIdentifier;
    s.project.children.push(node("AXList", {}, [node("AXButton", { AXIdentifier: thread })]));
  }, "thread_id_not_exposed");
  state = fixture();
  delete state.row.attrs.AXDOMIdentifier;
  state.wrapper.attrs.AXIdentifier = thread;
  assert(probe(state).ok && state.presses === 1, "single-row wrapper ID works");
  refused("shared wrapper cannot identify one row", function (s) {
    delete s.row.attrs.AXDOMIdentifier; delete s.sibling.attrs.AXDOMIdentifier;
    s.wrapper.attrs.AXIdentifier = thread; s.wrapper.children.push(s.sibling); s.project.children.pop();
  }, "thread_id_not_exposed");
  refused("conflicting fields", function (s) { s.wrapper.attrs.AXIdentifier = other; }, "thread_identity_conflict");
  refused("disabled row", function (s) { s.row.attrs.AXEnabled = false; }, "row_not_pressable");
  refused("no AXPress action", function (s) { s.row.actions = []; }, "row_not_pressable");
  refused("tree cycle is bounded", function (s) { s.wrapper.children.push(s.wrapper); }, "tree_depth");
  state = fixture();
  var originalRead = state.adapter.attribute;
  state.adapter.attribute = function (ref, name) {
    if (state.snapshots >= 3 && ref === state.row && name === "AXDOMIdentifier") return other;
    return originalRead(ref, name);
  };
  result = probe(state);
  assert(!result.ok && result.code === "thread_identity_changed" && state.presses === 0,
    "identity change immediately before press is refused");
  state = fixture();
  state.adapter.ensureTarget = function () {
    state.snapshots++;
    if (state.snapshots === 3) state.project.attrs.AXDescription = "Different project";
  };
  result = probe(state);
  assert(!result.ok && result.code === "project_list_changed" && state.presses === 0,
    "project change immediately before press is refused");
  state = fixture();
  state.adapter.ensureTarget = function () {
    state.snapshots++;
    if (state.snapshots === 3) state.navigation.attrs.AXSubrole = "AXLandmarkMain";
  };
  result = probe(state);
  assert(!result.ok && result.code === "navigation_changed" && state.presses === 0,
    "navigation change immediately before press is refused");
  state = fixture();
  state.adapter.press = function () { state.presses++; throw { number: -1712 }; };
  result = probe(state);
  assert(!result.ok && result.pressSent === "unknown" && state.presses === 1, "uncertain action is not retried");
  state = fixture();
  state.adapter.press = function () { state.presses++; };
  result = probe(state);
  assert(result.code === "selection_not_verified" && state.presses === 1, "verification never repeats press");
  state = fixture();
  state.adapter.role = function () { throw { number: -1743, message: "private label" }; };
  result = probe(state);
  assert(result.code === "automation_permission_required" && JSON.stringify(result).indexOf("private") < 0,
    "permission errors are redacted");
  [thread, "local:" + thread].forEach(function (value) {
    assert(api.identityID("AXIdentifier", value) === thread, "exact identifier format " + (value === thread ? "uuid" : "local"));
  });
  assert(api.identityID("AXURL", "codex://threads/" + thread) === thread, "exact local thread URL");
  ["codex://threads/" + thread + "?hostId=remote", "https://example.invalid/" + thread,
    "codex://threads/" + thread + "/"].forEach(function (value, index) {
    assert(api.identityID("AXURL", value) === null, "invalid URL format " + index);
  });
  assert(api.identityID("AXIdentifier", "prefix-" + thread) === null, "identifier substring is refused");
  var validArgs = ["--thread-id=" + thread, "--project-list-label=" + label];
  assert(api.parseOptions(validArgs).mode === "diagnose", "read-only default");
  [[], validArgs.concat("--mode=enable-accessibility"), validArgs.concat("--chat-title=anything"),
    validArgs.concat("--thread-id=" + thread)].forEach(function (values, index) {
    var rejected = false;
    try { api.parseOptions(values); } catch (_) { rejected = true; }
    assert(rejected, "invalid arguments " + index);
  });
  result = probe(fixture());
  assert(JSON.stringify(result).indexOf(thread) < 0 && JSON.stringify(result).indexOf(label) < 0,
    "report omits identity and label values");

  var clock = 0;
  var updates = [];
  var progress = api.progressReporter(function (line) { updates.push(JSON.parse(line)); }, function () { return clock; });
  progress({ stage: "inspect", nodeCount: 0 });
  clock = 500;
  progress({ nodeCount: 5 });
  clock = 1000;
  progress({ nodeCount: 10 });
  progress({ stage: "press", pressSent: "unknown" });
  assert(updates.length === 3 && updates[1].nodeCount === 10 && updates[2].pressSent === "unknown",
    "progress reports phases immediately and rate-limits scan updates");

  deadlineCases.forEach(function (testCase) {
    var child = $.NSTask.alloc.init;
    child.launchPath = "/usr/bin/osascript";
    child.arguments = ["-l", "JavaScript", root + "/scripts/test-poc-codex-system-events.js", testCase];
    var output = $.NSPipe.pipe;
    var errors = $.NSPipe.pipe;
    child.standardOutput = output;
    child.standardError = errors;
    var started = Date.now();
    child.launch;
    while (child.running && Date.now() - started < 5000) $.NSThread.sleepForTimeInterval(0.02);
    if (child.running) {
      // Only the NSTask owned by this test can be killed.
      ObjC.import("signal");
      $.kill(Number(child.processIdentifier), 9);
      throw new Error("deadline_did_not_stop_fixture");
    }
    var stdout = ObjC.unwrap($.NSString.alloc.initWithDataEncoding(output.fileHandleForReading.readDataToEndOfFile,
      $.NSUTF8StringEncoding));
    var stderr = ObjC.unwrap($.NSString.alloc.initWithDataEncoding(errors.fileHandleForReading.readDataToEndOfFile,
      $.NSUTF8StringEncoding));
    if (testCase === "--deadline-cancel") {
      assert(Number(child.terminationStatus) === 0 && JSON.parse(stdout).completed,
        "hard deadline is cancelled after completion");
      return;
    }
    var messages = stderr.split("\n").filter(function (line) { return line.charAt(0) === "{"; })
      .map(function (line) { return JSON.parse(line); });
    var last = messages[messages.length - 1];
    assert(Number(child.terminationStatus) === 14 && Number(child.terminationReason) === 2 &&
      Date.now() - started < 4000 && !stdout.trim(), "OS timer stops native blocking call " + testCase);
    assert(last && last.stage === (testCase === "--blocked-read" ? "inspect" : "press") &&
      last.pressSent === (testCase === "--blocked-read" ? false : "unknown"),
      "progress preserves action state on hard timeout " + testCase);
  });

  var nativeResults = [];
  if (args[0] === "--native") {
    var se = Application("/System/Library/CoreServices/System Events.app");
    if (!se.uiElementsEnabled({ timeout: 2 })) throw new Error("fixture_accessibility_permission_required");
    var directory = ObjC.unwrap($.NSTemporaryDirectory()) + "mallard-system-events-" + ObjC.unwrap($.NSUUID.UUID.UUIDString);
    if (!fm.createDirectoryAtPathWithIntermediateDirectoriesAttributesError(directory, true, $(), Ref())) {
      throw new Error("fixture_directory_failed");
    }
    ["data-only", "exact-id", "region-wrapper", "duplicate-id", "duplicate-project", "large-content"].forEach(function (scenario) {
      var reportPath = directory + "/" + scenario + ".json";
      var task = $.NSTask.alloc.init;
      task.launchPath = "/usr/bin/osascript";
      task.arguments = ["-l", "JavaScript", root + "/tests/fixtures/codex-sidebar-system-events.js", reportPath, scenario];
      task.standardOutput = $.NSPipe.pipe;
      task.standardError = $.NSPipe.pipe;
      task.launch;
      try {
        var until = Date.now() + 8000;
        var observed;
        do {
          delay(0.1);
          var raw = read(reportPath);
          observed = raw ? JSON.parse(raw) : null;
        } while ((!observed || !observed.ready) && task.running && Date.now() < until);
        if (!observed || !observed.ready) throw new Error("native_fixture_not_ready");
        var pid = Number(task.processIdentifier);
        var processIds;
        until = Date.now() + 3000;
        do {
          processIds = se.processes.whose({ unixId: pid }).id({ timeout: 2 });
          if (processIds.length === 1) break;
          delay(0.1);
        } while (Date.now() < until && task.running);
        var process = processIds.length === 1 ? se.processes.byId(processIds[0]) : null;
        if (!process || process.unixId() !== pid || process.bundleIdentifier() === "com.openai.codex") {
          throw new Error("fixture_process_identity_failed");
        }
        until = Date.now() + 3000;
        while (process.windows({ timeout: 2 }).length === 0 && Date.now() < until) delay(0.1);
        var nativeOptions = Object.assign({}, options, { windowTitle: "Mallard System Events Fixture" });
        var selectable = ["exact-id", "region-wrapper", "large-content"].indexOf(scenario) >= 0;
        if (selectable) {
          var diagnostic = api.runProbe(Object.assign({}, nativeOptions, { mode: "diagnose" }), function (_, trace) {
            return api.systemEventsAdapter(se, process, nativeOptions, trace);
          });
          assert(diagnostic.ok && diagnostic.selectorResult === "matched" && diagnostic.exactThreadIdentityProven &&
            JSON.parse(read(reportPath)).clicks === 0, "native diagnose matches without a press");
          if (scenario === "region-wrapper") {
            assert(diagnostic.navigationOutline.some(function (entry) {
              return entry.subrole === "AXLandmarkRegion" && entry.decision === "search_children";
            }), "native region wrapper is exposed and traversed");
          }
        }
        var outcome = api.runProbe(nativeOptions, function (_, trace) {
          return api.systemEventsAdapter(se, process, nativeOptions, trace);
        });
        delay(0.1);
        var after = JSON.parse(read(reportPath));
        var expected = { "data-only": "thread_id_not_exposed", "exact-id": "selected", "region-wrapper": "selected", "large-content": "selected",
          "duplicate-id": "thread_id_ambiguous", "duplicate-project": "project_list_ambiguous" }[scenario];
        nativeResults.push({ scenario: scenario, result: outcome, clicks: after.clicks,
          selectedProject: after.selectedPlace === "project", recentsPresent: after.recentsPresent });
        if (outcome.code !== expected) throw new Error("native_result_mismatch:" + JSON.stringify(nativeResults));
        assert(after.clicks === (selectable ? 1 : 0) && after.recentsPresent,
          "native " + scenario + " click count and Recents");
        if (selectable) {
          assert(after.selectedPlace === "project" && after.selectedThreadId === thread && outcome.projectRowSelected,
            "native press selects requested project thread");
          if (scenario === "large-content") {
            assert(after.elementCount > 8000 && outcome.nodeCount < 300 && outcome.elapsedMs < 15000,
              "native large content stays below the unchanged scan limit");
          }
          var repeated = api.runProbe(nativeOptions, function (_, trace) {
            return api.systemEventsAdapter(se, process, nativeOptions, trace);
          });
          delay(0.1);
          assert(repeated.code === "already_selected" && JSON.parse(read(reportPath)).clicks === 1,
            "native repeated request sends no click");
        }
      } finally {
        if (task.running) task.terminate;
        task.waitUntilExit;
      }
    });
  }
  return JSON.stringify({ ok: true, checksPassed: passed.length, checks: passed, nativeResults: nativeResults,
    realCodexTested: false, thirdPartyDependencies: 0 });
}
