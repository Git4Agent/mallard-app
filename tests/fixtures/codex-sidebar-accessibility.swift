// Separate WebKit fixture. It does not launch or contact Codex.
import AppKit
import WebKit

final class Fixture: NSObject, NSApplicationDelegate, WKScriptMessageHandler {
    var window: NSWindow!
    var webView: WKWebView!
    let report: URL
    let scenario: String

    init(report: URL, scenario: String) {
        self.report = report
        self.scenario = scenario
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.userContentController.add(self, name: "fixture")
        webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 620, height: 440), configuration: config)
        window = NSWindow(contentRect: webView.frame, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "Mallard Accessibility Fixture"
        window.contentView = webView
        window.center()
        window.makeKeyAndOrderFront(nil)
        // Match the relevant Codex semantics: labelled lists, role=button, and aria-current.
        // Positive controls deliberately expose an HTML id. The data-only case models Codex's data attribute.
        let duplicate = ["exact-id", "ancestor-id", "duplicate-id"].contains(scenario) ? row(location: "other-project-row", current: false) : ""
        let project = "<div role='list' aria-label='Scheduled tasks in Fixture project'>\(row(location: "project", current: false))\(duplicate)</div>"
        let projects = scenario == "duplicate-project" ? project + project : project
        let content = """
        <!doctype html><html><head><meta charset='utf-8'><style>
        body { font: 16px system-ui; padding: 24px; }
        [role=button] { margin: 8px; padding: 12px; border: 1px solid #aaa; }
        [aria-current=page] { background: #d9eaff; }
        </style></head><body>
        <h2>Recents</h2><div role='list' aria-label='Recents'>\(row(location: "recents", current: true))</div>
        <h2>Fixture project</h2>\(projects)
        <script>
        let presses = 0;
        let locationId = "chats";
        let selectedThreadId = null;
        function report() {
          window.webkit.messageHandlers.fixture.postMessage({
            ready: true, presses, locationId, selectedThreadId,
            recentsPresent: !!document.querySelector('[data-location="recents"]'),
            buttonCount: document.querySelectorAll('[role="button"]').length
          });
        }
        document.addEventListener("click", event => {
          const row = event.target.closest('[role="button"]');
          if (!row) return;
          presses++;
          selectedThreadId = row.dataset.threadId;
          document.querySelectorAll('[aria-current]').forEach(item => item.removeAttribute('aria-current'));
          row.setAttribute('aria-current', 'page');
          locationId = row.dataset.location === 'project' ? 'unified-project:fixture-project' : row.dataset.location;
          report();
        });
        report();
        </script></body></html>
        """
        webView.loadHTMLString(content, baseURL: nil)
    }

    func row(location: String, current: Bool) -> String {
        let threadID = location == "other-project-row" && scenario != "duplicate-id"
            ? "22222222-2222-4222-8222-222222222222" : "11111111-1111-4111-8111-111111111111"
        let identifier = scenario == "data-only" ? "" : "id='\(threadID)'"
        return "<div role='listitem'><div role='group' aria-label='Fixture row' \(scenario == "ancestor-id" ? identifier : "")><div role='button' tabindex='0' \(scenario == "ancestor-id" ? "" : identifier) aria-label='Fixture chat' data-thread-id='\(threadID)' data-location='\(location)' \(current ? "aria-current='page'" : "")>Fixture chat</div></div></div>"
    }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        guard var body = message.body as? [String: Any] else { exit(2) }
        let app = NSRunningApplication(processIdentifier: ProcessInfo.processInfo.processIdentifier)
        body["bundleIdentityMatches"] = Bundle.main.bundleIdentifier == "io.mallard.accessibility-poc.fixture"
        body["workspaceIdentityMatches"] = app?.bundleIdentifier == "io.mallard.accessibility-poc.fixture"
        guard let data = try? JSONSerialization.data(withJSONObject: body, options: [.sortedKeys]) else { exit(2) }
        do { try data.write(to: report, options: .atomic) } catch { exit(2) }
    }
}

guard CommandLine.arguments.count == 3,
      ["data-only", "exact-id", "ancestor-id", "duplicate-id", "duplicate-project"].contains(CommandLine.arguments[2]) else { exit(2) }
let application = NSApplication.shared
application.setActivationPolicy(.accessory)
let fixture = Fixture(report: URL(fileURLWithPath: CommandLine.arguments[1]), scenario: CommandLine.arguments[2])
application.delegate = fixture
application.run()
