// Separate macOS-only fixture. Uses built-in AppKit and WebKit through JXA.
// It has no connection to Codex and reads no provider data.
ObjC.import("AppKit");
ObjC.import("WebKit");

function run(args) {
  var reportPath = args[0];
  var scenario = args[1];
  if (!reportPath || ["data-only", "exact-id", "region-wrapper", "duplicate-id", "duplicate-project", "large-content"].indexOf(scenario) < 0) {
    throw new Error("invalid_fixture_arguments");
  }
  var thread = "11111111-1111-4111-8111-111111111111";
  var other = "22222222-2222-4222-8222-222222222222";
  function row(id, place, active) {
    var identity = scenario === "data-only" ? "" : ' id="' + id + '"';
    return '<div role="listitem"><div role="button" tabindex="0"' + identity + ' data-thread-id="' + id +
      '" data-place="' + place + '"' + (active ? ' aria-current="page"' : '') + '>Same title</div></div>';
  }
  var list = '<div role="list" aria-label="Scheduled tasks in Fixture project">' +
    row(thread, "project", false) + row(scenario === "duplicate-id" ? thread : other, "other", false) + '</div>';
  var mainContent = "";
  var otherProjects = "";
  if (scenario === "large-content") {
    for (var n = 0; n < 3000; n++) mainContent += '<div role="group"><button>Fixture content</button></div>';
    for (var p = 0; p < 40; p++) {
      otherProjects += '<div role="listitem"><div role="group"><div role="list" aria-label="Scheduled tasks in Other ' + p + '">';
      for (var r = 0; r < 25; r++) otherProjects += row(other, "other", false);
      otherProjects += '</div></div></div>';
    }
  }
  var html = '<!doctype html><html><head><meta charset="utf-8"><style>' +
    'body{font:16px system-ui;padding:20px}[role=button]{padding:12px;margin:8px;border:1px solid #888}' +
    '[aria-current=page]{background:#d8e8ff}main{position:absolute;left:1000px}</style></head><body>' +
    (scenario === "region-wrapper" ? '<div role="region" aria-label="Fixture shell">' : '') +
    '<main>' + mainContent + '</main><nav>' +
    '<h2>Recents</h2><div role="list" aria-label="Recents">' + row(thread, "recents", true) + '</div>' +
    '<h2>Fixture project</h2><div role="list" aria-label="Projects"><div role="listitem"><div role="group">' +
    list + (scenario === "duplicate-project" ? list : '') + '</div></div>' + otherProjects + '</div></nav>' +
    (scenario === "region-wrapper" ? '</div>' : '') +
    '<script>window.fixtureClicks=0;window.fixtureSelected=null;document.addEventListener("click",function(e){' +
    'var row=e.target.closest("[role=button]");if(!row)return;fixtureClicks++;fixtureSelected=row.dataset.place;' +
    'document.querySelectorAll("[aria-current]").forEach(function(n){n.removeAttribute("aria-current")});' +
    'row.setAttribute("aria-current","page")});</script></body></html>';
  var app = $.NSApplication.sharedApplication;
  app.setActivationPolicy(1);
  var view = $.WebView.alloc.initWithFrameFrameNameGroupName($.NSMakeRect(0, 0, 660, 480), "fixture", "mallard-system-events-poc");
  view.preferences.privateBrowsingEnabled = true;
  var window = $.NSWindow.alloc.initWithContentRectStyleMaskBackingDefer($.NSMakeRect(220, 220, 660, 480), 15, 2, false);
  window.title = "Mallard System Events Fixture";
  window.contentView = view;
  app.finishLaunching;
  window.orderFrontRegardless;
  view.mainFrame.loadHTMLStringBaseURL(html, $.NSURL.URLWithString("about:blank"));
  var deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(0.05));
    var raw = ObjC.unwrap(view.stringByEvaluatingJavaScriptFromString('JSON.stringify({ready:typeof fixtureClicks==="number",' +
      'clicks:window.fixtureClicks,selectedPlace:window.fixtureSelected,' +
      'elementCount:document.querySelectorAll("*").length,' +
      'recentsPresent:!!document.querySelector("[data-place=recents]"),' +
      'selectedThreadId:(document.querySelector("[aria-current=page]")||{dataset:{}}).dataset.threadId})'));
    if (raw) $(raw).writeToFileAtomicallyEncodingError(reportPath, true, $.NSUTF8StringEncoding, Ref());
  }
  window.close;
  return "fixture_complete";
}
