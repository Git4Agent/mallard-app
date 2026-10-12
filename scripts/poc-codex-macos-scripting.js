// Run with /usr/bin/osascript -l JavaScript. No Node or package dependency.
// Standalone POC. The app must already be running with its sidebar visible.

function parseOptions(args) {
  var options = { mode: "diagnose" };
  args.forEach(function (arg) {
    var match = /^--(mode|project-id|thread-id)=(.+)$/.exec(arg);
    if (!match || Object.prototype.hasOwnProperty.call(options, match[1] + "Seen")) {
      throw new Error("invalid_arguments");
    }
    options[match[1]] = match[2];
    options[match[1] + "Seen"] = true;
  });
  var uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  if ((options.mode !== "diagnose" && options.mode !== "press") ||
      !uuid.test(options["project-id"] || "") || !uuid.test(options["thread-id"] || "")) {
    throw new Error("invalid_arguments");
  }
  return {
    mode: options.mode,
    projectId: options["project-id"].toLowerCase(),
    threadId: options["thread-id"].toLowerCase()
  };
}

// This function runs in the app view. Keep it self-contained and text-free.
function sidebarProbe(options) {
  var prefix = "data-app-action-sidebar-";
  var result = {
    ok: false,
    code: "app_view_missing",
    selectorResult: "app_view_missing",
    exactThreadIdentityProven: false,
    projectRowSelected: false,
    pressSent: false,
    otherOccurrences: 0,
    otherActiveOccurrences: 0,
    labelsRedacted: true,
    identityValuesRedacted: true
  };
  function stop(code) {
    result.code = code;
    result.selectorResult = code;
    return result;
  }
  function active(element) {
    return element.isConnected &&
      !element.closest('[data-app-shell-active-page="false"], [inert], [hidden]');
  }
  function find(root, selector) {
    var nodes = Array.prototype.slice.call(root.querySelectorAll(selector));
    if (nodes.length > 2000) throw new Error("node_limit");
    return nodes.filter(active);
  }
  function current(element) {
    return element.getAttribute(prefix + "thread-active") === "true" &&
      element.getAttribute("aria-current") === "page";
  }
  try {
    var sidebars = find(document, "[" + prefix + "scroll]");
    if (sidebars.length !== 1) return stop(sidebars.length ? "app_view_ambiguous" : "app_view_missing");
    var sidebar = sidebars[0];
    var projects = find(sidebar, "[" + prefix + "project-list-id]").filter(function (element) {
      var section = element.closest("[" + prefix + "section]");
      return element.getAttribute(prefix + "project-list-id") === options.projectId &&
        section && section.getAttribute(prefix + "section-heading") === "Projects";
    });
    if (projects.length !== 1) return stop(projects.length ? "project_list_ambiguous" : "project_list_missing");
    var key = "local:" + options.threadId;
    var rows = find(projects[0], "[" + prefix + "thread-row]").filter(function (element) {
      return element.getAttribute(prefix + "thread-id") === key &&
        element.closest("[" + prefix + "project-list-id]") === projects[0];
    });
    if (rows.length !== 1) return stop(rows.length ? "thread_id_ambiguous" : "thread_id_missing");
    var row = rows[0];
    if (row.getAttribute(prefix + "thread-kind") !== "local" ||
        row.getAttribute(prefix + "thread-host-id") !== "local") return stop("local_thread_required");
    if (row.getAttribute("role") !== "button" || row.getAttribute("aria-disabled") === "true" ||
        row.hasAttribute("disabled")) return stop("thread_row_not_enabled");
    var style = window.getComputedStyle(row);
    if (!row.getClientRects().length || style.visibility !== "visible" || style.pointerEvents === "none") {
      return stop("thread_row_not_visible");
    }
    for (var parent = row; parent; parent = parent.parentElement) {
      if (window.getComputedStyle(parent).opacity === "0") return stop("thread_row_not_visible");
    }
    var others = find(sidebar, "[" + prefix + "thread-row]").filter(function (element) {
      return element !== row && element.getAttribute(prefix + "thread-id") === key &&
        element.getAttribute(prefix + "thread-kind") === "local" &&
        element.getAttribute(prefix + "thread-host-id") === "local";
    });
    result.otherOccurrences = others.length;
    result.otherActiveOccurrences = others.filter(function (element) {
      return element.getAttribute(prefix + "thread-active") === "true" ||
        element.getAttribute("aria-current") === "page";
    }).length;
    result.exactThreadIdentityProven = true;
    result.projectRowSelected = current(row) && result.otherActiveOccurrences === 0;
    result.selectorResult = "matched";
    result.code = result.projectRowSelected ? "already_selected" : "matched";
    result.ok = true;
    if (options.mode === "press" && !result.projectRowSelected) {
      // A normal row click lets Codex supply its own sidebar navigation state.
      // Never write history state, preferences, storage, or app internals.
      result.pressSent = true;
      result.code = "press_sent";
      row.click();
    }
    return result;
  } catch (error) {
    result.ok = false;
    result.code = error.message === "node_limit" ? "node_limit" : "dom_probe_failed";
    return result;
  }
}

function probeSource(options) {
  return "JSON.stringify((" + sidebarProbe.toString() + ")(" + JSON.stringify(options) + "))";
}

function errorCode(error) {
  var number = Number(error.errorNumber || error.number);
  if (number === -1743) return "automation_denied";
  if (number === -1712) return "apple_event_timeout";
  if (number === -1708) return "scripting_command_unavailable";
  if (number === -1728) return "scripting_object_unavailable";
  if (/javascript.*(disabled|not allowed|turned off)/i.test(String(error.message || ""))) {
    return "javascript_from_apple_events_disabled";
  }
  return "apple_event_failed";
}

// Adapter is injected only by the offline test. Production uses Apple events.
function runProbe(options, adapter) {
  var report = {
    ok: false, mode: options.mode, code: "not_started", pressSent: false,
    labelsRedacted: true, identityValuesRedacted: true
  };
  var pressAttempted = false;
  try {
    if (!adapter.running()) {
      report.code = "app_not_running";
      return report;
    }
    var view = adapter.frontView();
    report.scriptableWindowCount = view.windowCount;
    if (!view.tab) {
      report.code = view.windowCount ? "scriptable_tab_missing" : "scriptable_window_missing";
      return report;
    }
    function execute(mode) {
      if (!adapter.sameView(view)) throw new Error("front_view_changed");
      if (mode === "press") pressAttempted = true;
      var raw = adapter.execute(view, probeSource({
        mode: mode, projectId: options.projectId, threadId: options.threadId
      }));
      if (typeof raw !== "string" || raw.length > 4096) throw new Error("invalid_probe_response");
      var value;
      try { value = JSON.parse(raw); } catch (_) { throw new Error("invalid_probe_response"); }
      if (!value || typeof value.ok !== "boolean" || typeof value.pressSent !== "boolean" ||
          value.labelsRedacted !== true || value.identityValuesRedacted !== true) {
        throw new Error("invalid_probe_response");
      }
      var codes = ["app_view_missing", "app_view_ambiguous", "project_list_missing", "project_list_ambiguous",
        "thread_id_missing", "thread_id_ambiguous", "local_thread_required", "thread_row_not_enabled",
        "thread_row_not_visible", "matched", "already_selected", "press_sent", "node_limit", "dom_probe_failed"];
      if (codes.indexOf(value.code) < 0 || codes.indexOf(value.selectorResult) < 0 ||
          typeof value.exactThreadIdentityProven !== "boolean" || typeof value.projectRowSelected !== "boolean" ||
          !Number.isInteger(value.otherOccurrences) || value.otherOccurrences < 0 || value.otherOccurrences > 2000 ||
          !Number.isInteger(value.otherActiveOccurrences) || value.otherActiveOccurrences < 0 ||
          value.otherActiveOccurrences > value.otherOccurrences) throw new Error("invalid_probe_response");
      // Never copy unknown response fields into diagnostic output.
      return {
        ok: value.ok, code: value.code, selectorResult: value.selectorResult,
        exactThreadIdentityProven: value.exactThreadIdentityProven, projectRowSelected: value.projectRowSelected,
        pressSent: value.pressSent, otherOccurrences: value.otherOccurrences,
        otherActiveOccurrences: value.otherActiveOccurrences
      };
    }
    var before = execute("diagnose");
    Object.assign(report, before);
    if (options.mode === "diagnose" || !before.ok || before.projectRowSelected) return report;
    var pressed = execute("press");
    Object.assign(report, pressed);
    if (!pressed.ok || !pressed.pressSent) return report;
    // Verification may repeat. The press never repeats.
    for (var attempt = 0; attempt < 10; attempt++) {
      adapter.pause();
      var after = execute("diagnose");
      if (after.ok && after.projectRowSelected && after.otherOccurrences >= before.otherOccurrences) {
        Object.assign(report, after, { code: "selected", pressSent: true });
        return report;
      }
    }
    report.ok = false;
    report.code = "selection_not_verified";
    return report;
  } catch (error) {
    report.ok = false;
    report.code = /^(front_view_changed|invalid_probe_response)$/.test(error.message) ?
      error.message : errorCode(error);
    // A failed Apple event reply does not prove that the click was not sent.
    if (pressAttempted && !report.pressSent) report.pressSent = "unknown";
    var number = Number(error.errorNumber || error.number);
    if (Number.isFinite(number)) report.appleEventError = number;
    return report;
  }
}

function run(args) {
  var options;
  try { options = parseOptions(args); } catch (_) {
    return JSON.stringify({ ok: false, code: "invalid_arguments", pressSent: false });
  }
  // This block is never called by the fixture suite.
  ObjC.import("Foundation");
  var app;
  try { app = Application("com.openai.codex"); } catch (error) {
    return JSON.stringify({ ok: false, code: errorCode(error), pressSent: false });
  }
  var adapter = {
    running: function () { return app.running(); },
    frontView: function () {
      var windows = app.windows();
      if (!windows.length) return { windowCount: 0 };
      var windowId = windows[0].id();
      var front = app.windows.byId(windowId);
      if (!front.tabs().length) return { windowCount: windows.length };
      var tabId = front.activeTab().id();
      return { windowCount: windows.length, windowId: windowId, tabId: tabId, tab: front.tabs.byId(tabId) };
    },
    sameView: function (view) {
      return app.windows[0].id() === view.windowId && app.windows[0].activeTab().id() === view.tabId;
    },
    execute: function (view, source) { return view.tab.execute({ javascript: source }); },
    pause: function () { $.NSThread.sleepForTimeInterval(0.1); }
  };
  return JSON.stringify(runProbe(options, adapter));
}
