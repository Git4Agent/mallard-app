// Offline JXA tests. Uses macOS WebKit with synthetic HTML, never Codex.
ObjC.import("AppKit");
ObjC.import("WebKit");

function fixtureSuite(probe) {
  var prefix = "data-app-action-sidebar-";
  var projectId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  var threadId = "11111111-1111-4111-8111-111111111111";
  var otherId = "22222222-2222-4222-8222-222222222222";
  var options = { mode: "press", projectId: projectId, threadId: threadId };
  var names = [];
  var clicks;
  function assert(value, name) {
    if (!value) throw new Error(name);
    names.push(name);
  }
  function row(id, active) {
    return '<div role="button" ' + prefix + 'thread-row ' + prefix + 'thread-kind="local" ' +
      prefix + 'thread-host-id="local" ' + prefix + 'thread-id="local:' + id + '" ' +
      prefix + 'thread-active="' + active + '" ' + (active ? 'aria-current="page"' : '') +
      '>Same title</div>';
  }
  function reset() {
    clicks = 0;
    document.body.innerHTML = '<div ' + prefix + 'scroll><section ' + prefix + 'section ' +
      prefix + 'section-heading="Recents">' + row(threadId, true) + '</section><section ' +
      prefix + 'section ' + prefix + 'section-heading="Projects"><div ' + prefix +
      'project-list-id="' + projectId + '">' + row(threadId, false) + row(otherId, false) +
      '</div></section></div>';
    document.body.onclick = function (event) {
      var target = event.target.closest("[" + prefix + "thread-row]");
      if (!target) return;
      clicks++;
      document.querySelectorAll("[" + prefix + "thread-row]").forEach(function (element) {
        element.setAttribute(prefix + "thread-active", String(element === target));
        if (element === target) element.setAttribute("aria-current", "page");
        else element.removeAttribute("aria-current");
      });
    };
    return {
      project: document.querySelector("[" + prefix + "project-list-id]"),
      recents: document.querySelector("[" + prefix + "thread-row]"),
      row: document.querySelector("[" + prefix + "project-list-id] [" + prefix + "thread-row]")
    };
  }
  function refused(name, setup, code) {
    var nodes = reset();
    setup(nodes);
    var result = probe(options);
    assert(!result.ok && result.code === code && !result.pressSent && clicks === 0, name);
  }
  var nodes = reset();
  var result = probe(Object.assign({}, options, { mode: "diagnose" }));
  assert(result.ok && result.selectorResult === "matched" && clicks === 0, "diagnosis does not click");
  assert(result.otherOccurrences === 1 && !result.projectRowSelected, "Recents copy is initially active");
  result = probe(options);
  assert(result.ok && result.pressSent && clicks === 1 && nodes.row.getAttribute("aria-current") === "page",
    "exact project row selected among duplicate titles");
  assert(nodes.recents.isConnected && !nodes.recents.hasAttribute("aria-current"), "Recents copy remains");
  result = probe(options);
  assert(result.code === "already_selected" && !result.pressSent && clicks === 1, "repeat request does not click");
  nodes = reset();
  nodes.row.textContent = "Renamed fixture";
  assert(probe(options).pressSent && clicks === 1, "renamed chat uses ID");
  refused("wrong ID", function (n) { n.row.setAttribute(prefix + "thread-id", "local:" + otherId); }, "thread_id_missing");
  refused("ID in title only", function (n) {
    n.row.setAttribute(prefix + "thread-id", "local:" + otherId);
    n.row.textContent = threadId;
  }, "thread_id_missing");
  refused("bare UUID is not a row key", function (n) { n.row.setAttribute(prefix + "thread-id", threadId); }, "thread_id_missing");
  refused("ID only in Recents", function (n) { n.row.remove(); }, "thread_id_missing");
  refused("duplicate thread ID", function (n) { n.project.appendChild(n.row.cloneNode(true)); }, "thread_id_ambiguous");
  refused("duplicate project ID", function (n) { n.project.parentElement.appendChild(n.project.cloneNode(true)); }, "project_list_ambiguous");
  refused("pinned project is not Projects", function (n) {
    n.project.parentElement.setAttribute(prefix + "section-heading", "Pinned");
  }, "project_list_missing");
  refused("remote host", function (n) { n.row.setAttribute(prefix + "thread-host-id", "fixture-ssh"); }, "local_thread_required");
  refused("cloud kind", function (n) { n.row.setAttribute(prefix + "thread-kind", "remote"); }, "local_thread_required");
  refused("disabled row", function (n) { n.row.setAttribute("aria-disabled", "true"); }, "thread_row_not_enabled");
  refused("ID on wrapper", function (n) { n.row.setAttribute("role", "group"); }, "thread_row_not_enabled");
  refused("CSS hidden row", function (n) { n.row.style.display = "none"; }, "thread_row_not_visible");
  refused("transparent parent", function (n) { n.project.style.opacity = "0"; }, "thread_row_not_visible");
  refused("nested different project", function (n) {
    var nested = document.createElement("div");
    nested.setAttribute(prefix + "project-list-id", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
    n.project.appendChild(nested);
    nested.appendChild(n.row);
  }, "thread_id_missing");
  refused("inactive app page", function (n) { n.project.setAttribute("data-app-shell-active-page", "false"); }, "project_list_missing");
  refused("inert project", function (n) { n.project.setAttribute("inert", ""); }, "project_list_missing");
  refused("missing sidebar", function () { document.body.innerHTML = ""; }, "app_view_missing");
  refused("ambiguous sidebar", function () {
    document.body.appendChild(document.body.firstElementChild.cloneNode(true));
  }, "app_view_ambiguous");
  reset();
  result = probe(Object.assign({}, options, { mode: "diagnose" }));
  assert(JSON.stringify(result).indexOf(threadId) < 0 && JSON.stringify(result).indexOf("Same title") < 0,
    "diagnostic output redacts identities and titles");
  // Expose fixture controls only in this separate, in-memory WebKit page.
  window.resetFixture = reset;
  window.fixtureClickCount = function () { return clicks; };
  return names;
}

function run(args) {
  var path = args[0] || "scripts/poc-codex-macos-scripting.js";
  var error = Ref();
  var source = ObjC.unwrap($.NSString.stringWithContentsOfFileEncodingError(path, $.NSUTF8StringEncoding, error));
  if (typeof source !== "string") throw new Error("poc_source_missing");
  // Shadow Application: any accidental real-app call makes the test fail.
  var api = eval("(function () { var Application = function () { throw new Error('real_app_forbidden'); };\n" + source +
    "\nreturn {parseOptions:parseOptions, sidebarProbe:sidebarProbe, runProbe:runProbe}; })()");
  var names = [];
  function assert(value, name) {
    if (!value) throw new Error(name);
    names.push(name);
  }
  $.NSApplication.sharedApplication;
  // Legacy WebKit provides a synchronous, in-process DOM for this fixture only.
  // The shipped POC does not load WebKit or compile a helper.
  var view = $.WebView.alloc.initWithFrameFrameNameGroupName($.NSMakeRect(0, 0, 800, 600), "fixture", "mallard-scripting-poc");
  view.preferences.privateBrowsingEnabled = true;
  view.mainFrame.loadHTMLStringBaseURL("<html><body>Offline fixture</body></html>", $.NSURL.URLWithString("about:blank"));
  var deadline = Date.now() + 5000;
  while (view.loading && Date.now() < deadline) {
    $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(0.01));
  }
  function evaluate(script) { return ObjC.unwrap(view.stringByEvaluatingJavaScriptFromString(script)); }
  var domResult = evaluate("JSON.stringify((" + fixtureSuite.toString() + ")(" + api.sidebarProbe.toString() + "))");
  if (!domResult) throw new Error("dom_fixture_failed");
  names = names.concat(JSON.parse(domResult));
  var projectId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  var threadId = "11111111-1111-4111-8111-111111111111";
  var options = { mode: "press", projectId: projectId, threadId: threadId };
  var validArgs = ["--project-id=" + projectId, "--thread-id=" + threadId];
  assert(api.parseOptions(validArgs).mode === "diagnose", "default mode is read-only");
  [[], validArgs.concat("--mode=unknown"), validArgs.concat("--thread-id=" + threadId),
    validArgs.concat("--chat-title=title"), ["--project-id=" + projectId, "--thread-id=thread-123"]].forEach(function (args, index) {
    var refused = false;
    try { api.parseOptions(args); } catch (_) { refused = true; }
    assert(refused, "invalid arguments " + index);
  });
  function adapter() {
    evaluate("resetFixture(); 'ready'");
    return {
      calls: 0,
      running: function () { return true; },
      frontView: function () { return { windowCount: 1, tab: true }; },
      sameView: function () { return true; },
      execute: function (_, script) { this.calls++; return evaluate(script); },
      pause: function () {}
    };
  }
  function clicks() { return Number(evaluate("String(fixtureClickCount())")); }
  var transport = adapter();
  var result = api.runProbe(options, transport);
  assert(result.ok && result.code === "selected" && result.projectRowSelected && clicks() === 1,
    "driver verifies selected project row with one click");
  result = api.runProbe(options, transport);
  assert(result.code === "already_selected" && clicks() === 1, "driver does not repeat selection");
  transport = adapter();
  result = api.runProbe(Object.assign({}, options, { mode: "diagnose" }), transport);
  assert(result.ok && !result.pressSent && clicks() === 0, "driver diagnosis is read-only");
  transport = adapter();
  transport.running = function () { return false; };
  assert(api.runProbe(options, transport).code === "app_not_running" && transport.calls === 0, "does not launch app");
  transport = adapter();
  transport.frontView = function () { return { windowCount: 0 }; };
  assert(api.runProbe(options, transport).code === "scriptable_window_missing", "missing scriptable window");
  transport.frontView = function () { return { windowCount: 1 }; };
  assert(api.runProbe(options, transport).code === "scriptable_tab_missing", "missing scriptable tab");
  transport = adapter();
  transport.sameView = function () { return this.calls === 0; };
  result = api.runProbe(options, transport);
  assert(result.code === "front_view_changed" && result.pressSent === false && clicks() === 0,
    "view change stops before press");
  transport = adapter();
  transport.execute = function () { throw { number: -1743, message: "private label" }; };
  result = api.runProbe(options, transport);
  assert(result.code === "automation_denied" && result.pressSent === false && JSON.stringify(result).indexOf("private") < 0,
    "permission error is redacted");
  transport = adapter();
  transport.execute = function (_, script) {
    this.calls++;
    var reply = evaluate(script);
    if (this.calls === 2) throw { number: -1712 };
    return reply;
  };
  result = api.runProbe(options, transport);
  assert(result.code === "apple_event_timeout" && result.pressSent === "unknown" && clicks() === 1 && transport.calls === 2,
    "uncertain press is never retried");
  transport = adapter();
  transport.execute = function (_, script) {
    this.calls++;
    if (this.calls === 2) evaluate("document.body.onclick = null; 'ready'");
    return evaluate(script);
  };
  result = api.runProbe(options, transport);
  assert(!result.ok && result.code === "selection_not_verified" && result.pressSent === true && transport.calls === 12,
    "failed selection has one press and ten read-only checks");
  transport = adapter();
  transport.execute = function (_, script) {
    this.calls++;
    if (this.calls === 2) evaluate("document.querySelector('[data-app-action-sidebar-project-list-id]').remove(); 'ready'");
    return evaluate(script);
  };
  result = api.runProbe(options, transport);
  assert(!result.ok && result.code === "project_list_missing" && !result.pressSent && clicks() === 0,
    "press rechecks current DOM");
  transport = adapter();
  transport.execute = function (_, script) {
    var value = JSON.parse(evaluate(script));
    value.privateText = "must not leave fixture";
    return JSON.stringify(value);
  };
  result = api.runProbe(Object.assign({}, options, { mode: "diagnose" }), transport);
  assert(result.ok && JSON.stringify(result).indexOf("privateText") < 0, "unknown response fields are omitted");
  transport = adapter();
  transport.execute = function () { return "private non-JSON data"; };
  result = api.runProbe(options, transport);
  assert(result.code === "invalid_probe_response" && !result.pressSent, "invalid response stops before press");
  return JSON.stringify({ ok: true, testsPassed: names.length, checks: names, runtime: "macOS JXA and offline WebKit",
    realCodexTested: false, thirdPartyDependencies: 0 });
}
