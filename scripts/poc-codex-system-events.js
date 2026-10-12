// Standalone JXA POC. Run with /usr/bin/osascript -l JavaScript.
// All UI access goes through System Events. No direct Codex scripting or AX setters.

function withHardDeadline(seconds, work) {
  // This kernel timer also stops a synchronous Apple event or JXA bridge call.
  // A JavaScript deadline alone cannot interrupt either one.
  ObjC.import("unistd");
  $.alarm(seconds);
  try { return work(); } finally { $.alarm(0); }
}

function progressReporter(write, now) {
  var start = now();
  var lastWrite = -Infinity;
  var lastStage = "";
  var lastScanPhase = "";
  var state = { event: "progress", mode: "diagnose", stage: "starting", operation: "starting",
    nodeCount: 0, pressSent: false, labelsRedacted: true, identityValuesRedacted: true };
  return function (update) {
    Object.keys(update).forEach(function (key) { state[key] = update[key]; });
    var time = now();
    if (state.stage !== lastStage || state.scanPhase !== lastScanPhase || time - lastWrite >= 1000) {
      state.elapsedMs = time - start;
      write(JSON.stringify(state));
      lastStage = state.stage;
      lastScanPhase = state.scanPhase;
      lastWrite = time;
    }
  };
}

function fail(code) {
  var error = new Error(code);
  error.pocCode = code;
  throw error;
}

function parseOptions(args) {
  var values = {};
  args.forEach(function (arg) {
    var match = /^--(mode|thread-id|project-list-label|window-title)=(.+)$/.exec(arg);
    if (!match || Object.prototype.hasOwnProperty.call(values, match[1])) fail("invalid_arguments");
    values[match[1]] = match[2];
  });
  var mode = values.mode || "diagnose";
  var uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  if (["diagnose", "press"].indexOf(mode) < 0 || !uuid.test(values["thread-id"] || "") ||
      !values["project-list-label"] || values["project-list-label"].length > 300 ||
      (values["window-title"] || "").length > 300) fail("invalid_arguments");
  return { mode: mode, threadId: values["thread-id"].toLowerCase(),
    projectListLabel: values["project-list-label"], windowTitle: values["window-title"] };
}

function identityID(attribute, value) {
  if (typeof value !== "string") return null;
  var uuid = "([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})";
  var pattern = attribute === "AXURL" ? "^codex://threads/" + uuid + "$" :
    (attribute === "AXIdentifier" || attribute === "AXDOMIdentifier") ? "^(?:local:)?" + uuid + "$" : "(?!)";
  var match = new RegExp(pattern, "i").exec(value);
  return match ? match[1].toLowerCase() : null;
}

function inside(rows, index, ancestor) {
  while (index !== null) {
    if (index === ancestor) return true;
    index = rows[index].parent;
  }
  return false;
}

function projectScope(rows, label) {
  var lists = [];
  rows.forEach(function (row, index) {
    if (row.role === "AXList" && row.names.indexOf(label) >= 0) lists.push(index);
  });
  if (lists.length !== 1) fail(lists.length ? "project_list_ambiguous" : "project_list_missing");
  var list = lists[0];
  var buttons = [];
  rows.forEach(function (row, index) {
    if (row.role !== "AXButton") return;
    var parent = row.parent;
    while (parent !== null && parent !== list) {
      if (["AXButton", "AXLink", "AXList"].indexOf(rows[parent].role) >= 0) return;
      parent = rows[parent].parent;
    }
    if (parent === list) buttons.push(index);
  });
  return { list: list, buttons: buttons };
}

function identityOwners(rows, button, scope) {
  var owners = [button];
  var parent = rows[button].parent;
  while (parent !== null && parent !== scope.list) {
    if (scope.buttons.filter(function (index) { return inside(rows, index, parent); }).length !== 1) break;
    if (["AXGroup", "AXRow"].indexOf(rows[parent].role) >= 0) owners.push(parent);
    parent = rows[parent].parent;
  }
  return owners;
}

function selectRow(rows, scope, threadId) {
  var candidates = [];
  var anyIdentity = false;
  scope.buttons.forEach(function (button) {
    var ids = [];
    var sources = [];
    identityOwners(rows, button, scope).forEach(function (index) {
      Object.keys(rows[index].identity).forEach(function (attribute) {
        var id = identityID(attribute, rows[index].identity[attribute]);
        if (id && ids.indexOf(id) < 0) ids.push(id);
        if (id === threadId) sources.push({ attribute: attribute, onButton: index === button });
      });
    });
    anyIdentity = anyIdentity || ids.length > 0;
    if (ids.indexOf(threadId) < 0) return;
    if (ids.length !== 1) fail("thread_identity_conflict");
    candidates.push({ index: button, sources: sources });
  });
  if (candidates.length !== 1) {
    fail(candidates.length ? "thread_id_ambiguous" : anyIdentity ? "thread_id_not_found" : "thread_id_not_exposed");
  }
  return candidates[0];
}

function listLabels(adapter, ref) {
  return adapter.attributeValues(ref, ["AXTitle", "AXDescription"])
    .filter(function (value) { return typeof value === "string"; });
}

function snapshot(adapter, options, progress, report, located) {
  adapter.ensureTarget();
  if (!located) {
    report.nodeCount = 0;
    report.roleCounts = {};
    report.rendererExposed = false;
    report.navigationCount = 0;
    report.projectListCount = 0;
    report.navigationOutline = [];
    report.navigationOutlineTruncated = false;
  }
  var branchRoles = ["AXWindow", "AXScrollArea", "AXWebArea", "AXGroup", "AXSplitGroup", "AXTabGroup",
    "AXLayoutArea", "AXList", "AXRow", "AXOutline"];
  function branch(item) { return branchRoles.indexOf(item.role) >= 0; }
  function labels(ref) {
    // Only list labels are read. Never read chat titles or transcript AXValue.
    return listLabels(adapter, ref);
  }
  function walk(root, visit, rootAlreadyCounted, readSubroles) {
    var first = Object.assign({}, root, { parent: null, depth: 0 });
    var pending = [first];
    while (pending.length) {
      adapter.checkBudget();
      var item = pending.pop();
      if (item.depth > 60) fail("tree_depth");
      if (item !== first || !rootAlreadyCounted) {
        report.nodeCount++;
        var key = /^AX[A-Za-z]{1,40}$/.test(item.role) ? item.role : "other";
        report.roleCounts[key] = (report.roleCounts[key] || 0) + 1;
        report.rendererExposed = report.rendererExposed || item.role === "AXWebArea";
      }
      if (report.nodeCount + pending.length > 2500) fail("tree_limit");
      progress({ nodeCount: report.nodeCount, scanPhase: report.scanPhase });
      if (!visit(item)) continue;
      var children = adapter.childInfo(item.ref, readSubroles);
      for (var i = children.length - 1; i >= 0; i--) {
        pending.push(Object.assign({}, children[i], { parent: item, depth: item.depth + 1 }));
      }
    }
  }

  var navigation = [];
  var lists = [];
  if (located) {
    report.scanPhase = "recheck_project";
    progress({ scanPhase: report.scanPhase });
    if (adapter.subrole(located.navigation) !== "AXLandmarkNavigation") fail("navigation_changed");
    if (adapter.role(located.project) !== "AXList") fail("project_list_changed");
    var currentNames = labels(located.project);
    if (currentNames.indexOf(options.projectListLabel) < 0) fail("project_list_changed");
    navigation.push({ ref: located.navigation });
    lists.push({ ref: located.project, role: "AXList", names: currentNames });
  } else {
    // Container landmarks can enclose navigation. Only the main-content
    // landmark and control contents are excluded from this discovery pass.
    report.scanPhase = "find_navigation";
    progress({ scanPhase: report.scanPhase });
    walk({ ref: adapter.window, role: adapter.role(adapter.window), subrole: adapter.subrole(adapter.window) }, function (item) {
      var decision = item.subrole === "AXLandmarkNavigation" ? "navigation" :
        item.subrole === "AXLandmarkMain" ? "skip_main" : branch(item) ? "search_children" : "leaf";
      if (report.navigationOutline.length < 80) {
        report.navigationOutline.push({ role: /^AX[A-Za-z]{1,40}$/.test(item.role) ? item.role : "other",
          subrole: !item.subrole ? "" : /^AX[A-Za-z]{1,40}$/.test(item.subrole) ? item.subrole : "other",
          depth: item.depth, decision: decision });
      } else report.navigationOutlineTruncated = true;
      if (item.subrole === "AXLandmarkNavigation") {
        navigation.push(item);
        report.navigationCount = navigation.length;
        return false;
      }
      return decision === "search_children";
    }, false, true);
    if (navigation.length !== 1) fail(navigation.length ? "navigation_ambiguous" : "navigation_missing");

    report.scanPhase = "find_project_list";
    progress({ scanPhase: report.scanPhase });
    walk(navigation[0], function (item) {
      if (item.role === "AXList") {
        var names = labels(item.ref);
        if (names.indexOf(options.projectListLabel) >= 0) {
          item.names = names;
          lists.push(item);
          report.projectListCount = lists.length;
          return false;
        }
        // Current Codex English project thread lists use this exact label family.
        // Their chat rows cannot contain another project's sidebar list.
        if (names.some(function (name) { return name.indexOf("Scheduled tasks in ") === 0; })) return false;
      }
      return branch(item);
    }, true, false);
    if (lists.length !== 1) fail(lists.length ? "project_list_ambiguous" : "project_list_missing");
  }

  report.scanPhase = "read_project_rows";
  progress({ scanPhase: report.scanPhase });
  var rows = [];
  walk(lists[0], function (item) {
    item.rowIndex = rows.length;
    rows.push({ ref: item.ref, parent: item.parent ? item.parent.rowIndex : null,
      role: item.role, names: item.parent ? [] : lists[0].names, identity: {} });
    // A chat button is a leaf here. Its nested actions cannot identify it.
    return branch(item) && (item.parent === null || item.role !== "AXList");
  }, true, false);
  var result = { rows: rows, navigation: navigation[0].ref, project: lists[0].ref,
    scope: projectScope(rows, options.projectListLabel) };
  report.scanPhase = "read_identity";
  progress({ scanPhase: report.scanPhase });
  var owners = [];
  result.scope.buttons.forEach(function (button) {
    identityOwners(rows, button, result.scope).forEach(function (index) {
      if (owners.indexOf(index) < 0) owners.push(index);
    });
  });
  owners.forEach(function (index) {
    adapter.checkBudget();
    rows[index].identity = adapter.attributes(rows[index].ref, ["AXIdentifier", "AXDOMIdentifier", "AXURL"]);
  });
  return result;
}

function inspect(adapter, options, report, progress, located) {
  var tree = snapshot(adapter, options, progress, report, located);
  report.projectButtonCount = tree.scope.buttons.length;
  report.identityFields = ["AXIdentifier", "AXDOMIdentifier", "AXURL"].map(function (name) {
    var owners = [];
    tree.scope.buttons.forEach(function (button) {
      identityOwners(tree.rows, button, tree.scope).forEach(function (index) {
        if (owners.indexOf(index) < 0) owners.push(index);
      });
    });
    return { attribute: name, hasValue: owners.some(function (index) { return !!tree.rows[index].identity[name]; }),
      exactThreadID: owners.some(function (index) { return identityID(name, tree.rows[index].identity[name]) === options.threadId; }) };
  });
  var selected = selectRow(tree.rows, tree.scope, options.threadId);
  var ref = tree.rows[selected.index].ref;
  report.selectorResult = "matched";
  report.exactThreadIdentityProven = true;
  report.identitySources = selected.sources;
  var state = adapter.attributes(ref, ["AXARIACurrent", "AXEnabled"]);
  report.projectRowSelected = state.AXARIACurrent === "page";
  report.enabled = state.AXEnabled === true;
  report.pressSupported = adapter.actionNames(ref).indexOf("AXPress") >= 0;
  if (!report.enabled || !report.pressSupported) fail("row_not_pressable");
  return { tree: tree, ref: ref, selected: selected };
}

function errorCode(error) {
  if (error.pocCode) return error.pocCode;
  var number = Number(error.errorNumber || error.number);
  if (number === -1743) return "automation_permission_required";
  if (number === -1719 || /assistive access|accessibility.*not allowed/i.test(String(error.message || ""))) {
    return "accessibility_access_failed";
  }
  if (number === -1712) return "system_events_timeout";
  if (number === -1728) return "ui_element_unavailable";
  return "system_events_failed";
}

function runProbe(options, makeAdapter, progress) {
  var emit = progress || function () {};
  var started = Date.now();
  var report = { ok: false, mode: options.mode, transport: "system_events", code: "not_started",
    stage: "locate_target", selectorResult: "not_checked", exactThreadIdentityProven: false,
    projectRowSelected: false, pressSent: false, labelsRedacted: true, identityValuesRedacted: true };
  progress = function (update) {
    if (update.operation) report.lastOperation = update.operation;
    emit(update);
  };
  var pressAttempted = false;
  function stage(value) {
    report.stage = value;
    progress({ mode: options.mode, stage: value, operation: value, pressSent: report.pressSent });
  }
  try {
    stage("locate_target");
    var adapter = makeAdapter(options, progress);
    report.windowCount = adapter.windowCount;
    stage("inspect");
    var initial = inspect(adapter, options, report, progress);
    report.ok = true;
    report.code = report.projectRowSelected ? "already_selected" : "matched";
    if (options.mode === "diagnose" || report.projectRowSelected) return report;
    stage("recheck");
    report.exactThreadIdentityProven = false;
    var fresh = inspect(adapter, options, report, progress, initial.tree);
    if (report.projectRowSelected) { report.code = "already_selected"; return report; }
    var buttonCount = report.projectButtonCount;
    adapter.ensureTarget();
    if (adapter.subrole(fresh.tree.navigation) !== "AXLandmarkNavigation") fail("navigation_changed");
    var projectRef = fresh.tree.rows[fresh.tree.scope.list].ref;
    if (adapter.role(projectRef) !== "AXList" ||
        listLabels(adapter, projectRef).indexOf(options.projectListLabel) < 0) fail("project_list_changed");
    // Recheck each contributing identity immediately before the sole action.
    var finalIds = [];
    identityOwners(fresh.tree.rows, fresh.selected.index, fresh.tree.scope).forEach(function (index) {
      var values = adapter.attributes(fresh.tree.rows[index].ref, ["AXIdentifier", "AXDOMIdentifier", "AXURL"]);
      Object.keys(values).forEach(function (name) {
        var id = identityID(name, values[name]);
        if (id && finalIds.indexOf(id) < 0) finalIds.push(id);
      });
    });
    if (finalIds.length !== 1 || finalIds[0] !== options.threadId) fail("thread_identity_changed");
    if (adapter.attribute(fresh.ref, "AXEnabled") !== true) fail("row_not_pressable");
    pressAttempted = true;
    // Persist uncertainty in progress before an action can block or time out.
    report.pressSent = "unknown";
    stage("press");
    adapter.press(fresh.ref);
    report.pressSent = true;
    stage("verify");
    for (var attempt = 0; attempt < 3; attempt++) {
      adapter.pause();
      inspect(adapter, options, report, progress, fresh.tree);
      if (report.projectRowSelected) {
        if (report.projectButtonCount !== buttonCount) fail("project_row_count_changed");
        report.code = "selected";
        return report;
      }
    }
    fail("selection_not_verified");
  } catch (error) {
    report.ok = false;
    report.code = errorCode(error);
    if (report.stage === "inspect" || report.stage === "recheck") report.selectorResult = report.code;
    if (pressAttempted && !report.pressSent) report.pressSent = "unknown";
    var number = Number(error.errorNumber || error.number);
    if (Number.isFinite(number)) report.appleEventError = number;
    return report;
  } finally {
    report.elapsedMs = Date.now() - started;
  }
}

// Tests pass only their own fixture process to this adapter.
function systemEventsAdapter(se, process, options, progress) {
  progress = progress || function () {};
  var timeout = { timeout: 2 };
  var deadline = Date.now() + 15000;
  progress({ stage: "process_identity", operation: "read_process_id" });
  var processId = process.id(timeout);
  progress({ operation: "read_process_pid" });
  var pid = process.unixId(timeout);
  progress({ operation: "read_process_bundle" });
  var bundle = process.bundleIdentifier(timeout);
  var target = se.processes.byId(processId);
  function checkBudget() { if (Date.now() > deadline) fail("probe_timeout"); }
  function attribute(ref, name) {
    checkBudget();
    progress({ operation: "read_" + name });
    try { return ref.attributes.byName(name).value(timeout); } catch (error) {
      // System Events reports an absent optional AX attribute as a missing object.
      if (Number(error.errorNumber || error.number) === -1728) return null;
      throw error;
    }
  }
  function positions(collection, operation) {
    checkBudget();
    progress({ operation: operation });
    var count = collection(timeout).length;
    if (count > 2500) fail("tree_limit");
    var refs = [];
    // Materialized System Events references can revert to ambiguous names.
    // Keep the original process-ID path and explicit child positions instead.
    for (var index = 0; index < count; index++) refs.push(collection.at(index));
    return refs;
  }
  progress({ stage: "list_windows", operation: "list_windows" });
  var windows = positions(target.windows, "list_windows");
  if (!windows.length) fail("accessibility_window_missing");
  if (windows.length > 12) fail("window_limit");
  var candidates = windows;
  if (options.windowTitle) candidates = windows.filter(function (window) { return attribute(window, "AXTitle") === options.windowTitle; });
  else if (windows.length > 1) candidates = windows.filter(function (window) { return attribute(window, "AXMain") === true; });
  if (candidates.length !== 1) fail("window_missing_or_ambiguous");
  var window = candidates[0];
  var windowKey = Automation.getDisplayString(window);
  function selectedAttributes(ref, names) {
    return ref.attributes.whose({ _or: names.map(function (name) { return { name: name }; }) });
  }
  return {
    window: window, windowCount: windows.length, checkBudget: checkBudget, attribute: attribute,
    attributeValues: function (ref, names) {
      checkBudget();
      progress({ operation: "read_selected_attribute_values" });
      return selectedAttributes(ref, names).value(timeout);
    },
    attributes: function (ref, names) {
      checkBudget();
      // System Events supports filtered name/value lists, not attribute
      // property records. Only explicitly requested AX values are read.
      var selected = selectedAttributes(ref, names);
      progress({ operation: "read_selected_attribute_names" });
      var keys = selected.name(timeout);
      checkBudget();
      progress({ operation: "read_selected_attribute_values" });
      var values = selected.value(timeout);
      if (keys.length !== values.length || keys.length > names.length) fail("attribute_result_invalid");
      var result = {};
      keys.forEach(function (name, index) {
        if (names.indexOf(name) < 0 || Object.prototype.hasOwnProperty.call(result, name)) {
          fail("attribute_result_invalid");
        }
        result[name] = values[index];
      });
      return result;
    },
    ensureTarget: function () {
      checkBudget();
      progress({ operation: "verify_process" });
      if (target.unixId(timeout) !== pid || target.bundleIdentifier(timeout) !== bundle ||
          !positions(target.windows, "verify_windows").some(function (ref) { return Automation.getDisplayString(ref) === windowKey; })) {
        fail("target_changed");
      }
    },
    role: function (ref) { checkBudget(); progress({ operation: "read_role" }); return ref.role(timeout); },
    subrole: function (ref) { checkBudget(); progress({ operation: "read_subrole" }); return ref.subrole(timeout); },
    childInfo: function (ref, readSubroles) {
      checkBudget();
      progress({ operation: "read_child_roles" });
      var roles = ref.uiElements.role(timeout);
      if (roles.length > 2500) fail("tree_limit");
      var subroles = roles.map(function () { return ""; });
      if (readSubroles && roles.length) {
        checkBudget();
        progress({ operation: "read_child_subroles" });
        subroles = ref.uiElements.subrole(timeout);
      }
      if (roles.length !== subroles.length) fail("tree_changed");
      // Read metadata in batches. Keep positional references, not the named
      // references returned by materializing an entire UI element collection.
      return roles.map(function (role, index) {
        return { ref: ref.uiElements.at(index), role: role, subrole: subroles[index] };
      });
    },
    actionNames: function (ref) { checkBudget(); progress({ operation: "list_actions" }); return ref.actions.name(timeout); },
    press: function (ref) { checkBudget(); ref.actions.byName("AXPress").perform(timeout); },
    pause: function () { delay(0.2); }
  };
}

function run(args) {
  var options;
  try { options = parseOptions(args); } catch (_) {
    return JSON.stringify({ ok: false, code: "invalid_arguments", pressSent: false });
  }
  return withHardDeadline(20, function () {
    var progress = progressReporter(function (line) { console.log(line); }, Date.now);
    progress({ mode: options.mode, stage: "starting", hardTimeoutSeconds: 20 });
    var result = runProbe(options, function (_, trace) {
      trace({ stage: "connect_system_events", operation: "load_dictionary" });
      var se = Application("/System/Library/CoreServices/System Events.app");
      trace({ stage: "check_accessibility", operation: "read_permission" });
      if (se.uiElementsEnabled({ timeout: 2 }) !== true) fail("accessibility_permission_required");
      trace({ stage: "find_process", operation: "find_bundle" });
      var ids = se.applicationProcesses.whose({ bundleIdentifier: "com.openai.codex" }).id({ timeout: 2 });
      if (ids.length !== 1) fail(ids.length ? "target_process_ambiguous" : "target_not_running");
      return systemEventsAdapter(se, se.processes.byId(ids[0]), options, trace);
    }, progress);
    progress({ stage: "complete", operation: "complete", code: result.code, pressSent: result.pressSent });
    return JSON.stringify(result);
  });
}
