// Standalone POC. No provider-home access, app launch, permission prompt, or sidebar settings writes.
// Only enable-accessibility requests Chromium's runtime AXEnhancedUserInterface flag.
import AppKit
import ApplicationServices
import Darwin
import Foundation

var pressSent = false

struct ProbeError: Error {
    let code: String
    init(_ code: String) { self.code = code }
}

struct Row {
    let element: AXUIElement?
    let parent: Int?
    let role: String
    let names: Set<String>
    var current: String? = nil
    var subrole: String? = nil
    var identity: [String: String] = [:]
}

let identityAttributes = ["AXIdentifier", "AXDOMIdentifier", "AXURL"]

func canonicalThreadID(_ value: String) -> String? {
    guard value.count == 36, let id = UUID(uuidString: value) else { return nil }
    return id.uuidString.lowercased()
}

func identityThreadID(attribute: String, value: String) -> String? {
    if attribute == "AXIdentifier" || attribute == "AXDOMIdentifier" {
        return canonicalThreadID(value)
    }
    // The installed Codex source copies local links in this exact form.
    // Do not accept arbitrary URLs, UUID substrings, or remote/profile query parameters.
    guard attribute == "AXURL", value.hasPrefix("codex://threads/"),
          value.count == "codex://threads/".count + 36 else { return nil }
    return canonicalThreadID(String(value.dropFirst("codex://threads/".count)))
}

struct ProcessStart: Equatable {
    let seconds: UInt64
    let microseconds: UInt64
}

func processStart(_ pid: pid_t) throws -> ProcessStart {
    var info = proc_bsdinfo()
    let size = Int32(MemoryLayout<proc_bsdinfo>.stride)
    guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size else {
        throw ProbeError("target_start_unavailable")
    }
    return ProcessStart(seconds: info.pbi_start_tvsec, microseconds: info.pbi_start_tvusec)
}

func descendants(_ rows: [Row], of ancestor: Int) -> Set<Int> {
    var result: Set<Int> = [ancestor]
    for index in rows.indices where rows[index].parent.map({ result.contains($0) }) == true {
        result.insert(index)
    }
    return result
}

func projectButtons(_ rows: [Row], list: String) throws -> (list: Int, buttons: [Int]) {
    let lists = rows.indices.filter { rows[$0].role == "AXList" && rows[$0].names.contains(list) }
    guard lists.count == 1 else {
        throw ProbeError(lists.isEmpty ? "project_list_missing" : "project_list_ambiguous")
    }
    let scope = descendants(rows, of: lists[0])
    let buttons = rows.indices.filter { index in
        guard scope.contains(index), rows[index].role == "AXButton" else { return false }
        var parent = rows[index].parent
        while let ancestor = parent, ancestor != lists[0] {
            // Nested action buttons and nested lists are not chat rows.
            if ["AXButton", "AXLink", "AXList"].contains(rows[ancestor].role) { return false }
            parent = rows[ancestor].parent
        }
        return parent == lists[0]
    }
    return (lists[0], buttons)
}

func identityOwners(_ rows: [Row], button: Int, scope: (list: Int, buttons: [Int])) -> [Int] {
    var owners = [button]
    var parent = rows[button].parent
    while let ancestor = parent, ancestor != scope.list {
        // A wrapper can identify a row only when it owns exactly one chat button.
        let subtree = descendants(rows, of: ancestor)
        guard scope.buttons.filter({ subtree.contains($0) }).count == 1 else { break }
        if ["AXGroup", "AXRow"].contains(rows[ancestor].role) { owners.append(ancestor) }
        parent = rows[ancestor].parent
    }
    return owners
}

func identifiedThreads(_ rows: [Row], button: Int, scope: (list: Int, buttons: [Int])) -> Set<String> {
    Set(identityOwners(rows, button: button, scope: scope).flatMap { owner in
        rows[owner].identity.compactMap { identityThreadID(attribute: $0.key, value: $0.value) }
    })
}

func select(_ rows: [Row], list: String, threadID: String) throws -> Int {
    let scope = try projectButtons(rows, list: list)
    var exposed = false
    var matches: [Int] = []
    for button in scope.buttons {
        let ids = identifiedThreads(rows, button: button, scope: scope)
        exposed = exposed || !ids.isEmpty
        if ids.contains(threadID) {
            guard ids.count == 1 else { throw ProbeError("thread_identity_conflict") }
            matches.append(button)
        }
    }
    guard matches.count == 1 else {
        throw ProbeError(matches.isEmpty ? (exposed ? "thread_id_not_found" : "thread_id_not_exposed") : "thread_id_ambiguous")
    }
    return matches[0]
}

func identitySources(_ rows: [Row], button: Int, list: String, threadID: String) throws -> [[String: Any]] {
    let scope = try projectButtons(rows, list: list)
    return identityOwners(rows, button: button, scope: scope).flatMap { owner in
        identityAttributes.compactMap { name -> [String: Any]? in
            guard let value = rows[owner].identity[name],
                  identityThreadID(attribute: name, value: value) == threadID else { return nil }
            return ["attribute": name, "ownerRole": rows[owner].role, "onButton": owner == button]
        }
    }
}

func emit(_ value: [String: Any]) {
    let data = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    print(String(decoding: data, as: UTF8.self))
}

func attribute(_ element: AXUIElement, _ name: String) throws -> CFTypeRef? {
    var value: CFTypeRef?
    let result = AXUIElementCopyAttributeValue(element, name as CFString, &value)
    if result == .attributeUnsupported || result == .noValue { return nil }
    guard result == .success else { throw ProbeError("ax_read_\(result.rawValue)") }
    return value
}

func elements(_ element: AXUIElement, _ name: String) throws -> [AXUIElement] {
    var count = 0
    let counted = AXUIElementGetAttributeValueCount(element, name as CFString, &count)
    if counted == .attributeUnsupported || counted == .noValue { return [] }
    guard counted == .success else { throw ProbeError("ax_count_\(counted.rawValue)") }
    guard count <= 4_000 else { throw ProbeError("tree_limit") }
    if count == 0 { return [] }
    var values: CFArray?
    let result = AXUIElementCopyAttributeValues(element, name as CFString, 0, count, &values)
    guard result == .success, let children = values as? [AXUIElement] else {
        throw ProbeError("ax_children_\(result.rawValue)")
    }
    return children
}

func snapshot(_ root: AXUIElement, list: String, diagnose: Bool = false) throws -> [Row] {
    let deadline = ProcessInfo.processInfo.systemUptime + 8
    var pending: [(AXUIElement, Int?, Int)] = [(root, nil, 0)]
    var rows: [Row] = []
    var seen: [CFHashCode: [AXUIElement]] = [:]
    while rows.count < pending.count {
        guard ProcessInfo.processInfo.systemUptime < deadline else { throw ProbeError("tree_timeout") }
        guard pending.count <= 4_000 else { throw ProbeError("tree_limit") }
        let (element, parent, depth) = pending[rows.count]
        guard depth < 64 else { throw ProbeError("tree_depth") }
        let hash = CFHash(element)
        guard !(seen[hash] ?? []).contains(where: { CFEqual($0, element) }) else {
            throw ProbeError("tree_repeated_element")
        }
        seen[hash, default: []].append(element)
        let role = try attribute(element, kAXRoleAttribute) as? String ?? ""
        // Diagnostics read container/control labels, never static text or AXValue.
        var names = Set<String>()
        let diagnosticRoles: Set<String> = ["AXGroup", "AXOutline", "AXScrollArea", "AXWebArea", "AXLink", "AXRow"]
        if role == "AXList" || role == "AXButton" || (diagnose && diagnosticRoles.contains(role)) {
            for name in [kAXTitleAttribute, kAXDescriptionAttribute] {
                if let value = try attribute(element, name) as? String { names.insert(value) }
            }
        }
        let current = role == "AXButton" ? try attribute(element, "AXARIACurrent") as? String : nil
        let subrole = diagnose ? try attribute(element, kAXSubroleAttribute) as? String : nil
        let index = rows.count
        rows.append(Row(element: element, parent: parent, role: role, names: names, current: current, subrole: subrole))
        for child in try elements(element, kAXChildrenAttribute) {
            pending.append((child, index, depth + 1))
        }
    }
    // Read identities only in the requested project list, never in the transcript.
    if let scope = try? projectButtons(rows, list: list) {
        let owners = Set(scope.buttons.flatMap { identityOwners(rows, button: $0, scope: scope) })
        for owner in owners {
            for name in identityAttributes {
                guard ProcessInfo.processInfo.systemUptime < deadline else { throw ProbeError("tree_timeout") }
                var value: CFTypeRef?
                let result = AXUIElementCopyAttributeValue(rows[owner].element!, name as CFString, &value)
                if [.attributeUnsupported, .noValue, .notImplemented].contains(result) { continue }
                guard result == .success else { throw ProbeError("ax_identity_read_\(result.rawValue)") }
                if let text = value as? String { rows[owner].identity[name] = text }
                else if let url = value as? URL { rows[owner].identity[name] = url.absoluteString }
                else if value != nil { rows[owner].identity[name] = "[non-string]" }
            }
        }
    }
    return rows
}

func diagnostics(_ rows: [Row], list: String, title: String, project: String, threadID: String) -> [String: Any] {
    var counts: [String: Int] = [:]
    for row in rows { counts[row.role, default: 0] += 1 }
    let matches = rows.indices.filter {
        rows[$0].names.contains(list) || rows[$0].names.contains(title) ||
        rows[$0].names.contains(where: { $0.localizedCaseInsensitiveContains(project) }) ||
            rows[$0].identity.contains(where: { identityThreadID(attribute: $0.key, value: $0.value) == threadID })
    }
    func detail(_ index: Int) -> [String: Any] {
        let row = rows[index]
        return [
            "index": index, "role": row.role, "subrole": row.subrole ?? "",
            "exactListLabel": row.names.contains(list), "exactChatTitle": row.names.contains(title),
            "projectNameInLabel": row.names.contains(where: { $0.localizedCaseInsensitiveContains(project) }),
        ]
    }
    var selector: String
    do { _ = try select(rows, list: list, threadID: threadID); selector = "matched" }
    catch let error as ProbeError { selector = error.code }
    catch { selector = "unexpected_error" }
    let details = matches.prefix(40).map { index -> [String: Any] in
        var result = detail(index)
        var ancestors: [[String: Any]] = []
        var parent = rows[index].parent
        while let ancestor = parent {
            ancestors.append(detail(ancestor))
            parent = rows[ancestor].parent
        }
        // Nearest ancestors suffice to distinguish a project list from a chat button.
        result["ancestors"] = Array(ancestors.prefix(6))
        return result
    }
    let scope = try? projectButtons(rows, list: list)
    let candidates = scope?.buttons.filter { button in
        title.isEmpty || rows[button].names.contains(title) ||
            identifiedThreads(rows, button: button, scope: scope!).contains(threadID)
    } ?? []
    let identities = candidates.prefix(40).map { button -> [String: Any] in
        let owners = identityOwners(rows, button: button, scope: scope!)
        var fields: [String: Any] = [:]
        for name in identityAttributes {
            let values = owners.compactMap { rows[$0].identity[name] }
            fields[name] = [
                "hasValue": values.contains(where: { !$0.isEmpty }),
                "exactThreadID": values.contains(where: { identityThreadID(attribute: name, value: $0) == threadID }),
                // Diagnostic only. Substrings never authorize selection.
                "containsThreadID": values.contains(where: { $0.lowercased().contains(threadID) }),
            ]
        }
        return ["index": button, "exactChatTitle": rows[button].names.contains(title), "identityFields": fields]
    }
    return [
        "ok": true, "mode": "diagnose", "pressSent": false,
        "nodeCount": rows.count, "roleCounts": counts, "selectorResult": selector,
        "rendererExposed": rows.contains(where: { $0.role == "AXWebArea" }),
        "matchingNodes": details, "matchingNodeCount": matches.count,
        "matchesTruncated": matches.count > 40, "labelsRedacted": true,
        "matchBasis": "project_list_and_native_thread_id", "exactThreadIdentityProven": selector == "matched",
        "identityValuesRedacted": true, "identityCandidates": identities,
        "identityCandidatesTruncated": candidates.count > 40, "projectButtonCount": scope?.buttons.count ?? 0,
    ]
}

func selfTest() throws {
    let list = "Scheduled tasks in Fixture project"
    let title = "Fixture chat"
    let threadID = "11111111-1111-4111-8111-111111111111"
    let otherID = "22222222-2222-4222-8222-222222222222"
    let rows = [
        Row(element: nil, parent: nil, role: "AXWindow", names: []),
        Row(element: nil, parent: 0, role: "AXList", names: ["Recents"]),
        Row(element: nil, parent: 1, role: "AXButton", names: [title], current: "page", identity: ["AXDOMIdentifier": threadID]),
        Row(element: nil, parent: 0, role: "AXList", names: [list]),
        Row(element: nil, parent: 3, role: "AXGroup", names: []),
        Row(element: nil, parent: 4, role: "AXButton", names: [title], identity: ["AXDOMIdentifier": threadID]),
        Row(element: nil, parent: 5, role: "AXButton", names: ["Archive chat"]),
        Row(element: nil, parent: 3, role: "AXButton", names: [title], identity: ["AXDOMIdentifier": otherID]),
    ]
    guard try select(rows, list: list, threadID: threadID) == 5 else { throw ProbeError("selected_wrong_row") }
    var checks = ["thread_id_selects_project_copy_despite_duplicate_titles"]
    func refused(_ name: String, _ expected: String, _ tree: [Row], listName: String? = nil, id: String? = nil) throws {
        do {
            _ = try select(tree, list: listName ?? list, threadID: id ?? threadID)
            throw ProbeError("expected_refusal")
        } catch let error as ProbeError {
            guard error.code == expected else { throw error }
            checks.append(name)
        }
    }
    try refused("missing_project_refused", "project_list_missing", rows, listName: "Missing")
    try refused("wrong_thread_id_refused", "thread_id_not_found", rows, id: "33333333-3333-4333-8333-333333333333")
    try refused("duplicate_thread_id_refused", "thread_id_ambiguous", rows + [
        Row(element: nil, parent: 3, role: "AXButton", names: ["Different title"], identity: ["AXDOMIdentifier": threadID]),
    ])
    try refused("duplicate_project_refused", "project_list_ambiguous", rows + [
        Row(element: nil, parent: 0, role: "AXList", names: [list]),
    ])
    var noIDs = rows
    for index in noIDs.indices { noIDs[index].identity = [:] }
    try refused("title_match_never_substitutes_for_thread_id", "thread_id_not_exposed", noIDs)
    noIDs[2].identity = rows[2].identity
    try refused("recents_id_does_not_identify_project_row", "thread_id_not_exposed", noIDs)
    noIDs[5] = Row(element: nil, parent: 4, role: "AXButton", names: [threadID])
    try refused("title_containing_thread_id_refused", "thread_id_not_exposed", noIDs)
    noIDs[6].identity = rows[5].identity
    try refused("nested_action_id_does_not_identify_chat", "thread_id_not_exposed", noIDs)

    var wrapper = rows
    wrapper[5].identity = [:]
    wrapper[4].identity = ["AXIdentifier": threadID]
    guard try select(wrapper, list: list, threadID: threadID) == 5 else { throw ProbeError("wrapper_id_lost") }
    checks.append("single_row_wrapper_id_accepted")
    wrapper[7] = Row(element: nil, parent: 4, role: "AXButton", names: [title], identity: ["AXDOMIdentifier": otherID])
    try refused("shared_wrapper_id_not_assigned_to_a_row", "thread_id_not_found", wrapper)
    var conflict = rows
    conflict[4].identity = ["AXIdentifier": otherID]
    try refused("conflicting_row_identity_refused", "thread_identity_conflict", conflict)

    var renamed = rows
    renamed[5] = Row(element: nil, parent: 4, role: "AXButton", names: ["Renamed chat"], identity: ["AXDOMIdentifier": threadID])
    guard try select(renamed, list: list, threadID: threadID) == 5 else { throw ProbeError("title_used_for_selection") }
    checks.append("selection_does_not_depend_on_title")
    var link = rows
    link[5].identity = ["AXURL": "codex://threads/\(threadID)"]
    guard try select(link, list: list, threadID: threadID) == 5 else { throw ProbeError("local_deeplink_id_lost") }
    checks.append("exact_local_deeplink_id_accepted")
    for (name, value) in [("AXDOMIdentifier", "prefix-\(threadID)"), ("AXIdentifier", "\(threadID)-suffix"),
                          ("AXURL", "https://example.invalid/\(threadID)"),
                          ("AXURL", "codex://threads/\(threadID)?hostId=remote")] {
        guard identityThreadID(attribute: name, value: value) == nil else { throw ProbeError("unverified_id_format_accepted") }
    }
    guard canonicalThreadID("not-a-native-thread-id") == nil,
          canonicalThreadID("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".uppercased()) == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa" else {
        throw ProbeError("uuid_validation_failed")
    }
    checks.append("uuid_substrings_and_unverified_links_refused")
    var groupRows = rows
    groupRows[3] = Row(element: nil, parent: 0, role: "AXGroup", names: [list], subrole: "AXLandmarkRegion")
    let diagnostic = diagnostics(groupRows, list: list, title: title, project: "Fixture project", threadID: threadID)
    let matches = diagnostic["matchingNodes"] as! [[String: Any]]
    guard diagnostic["selectorResult"] as? String == "project_list_missing",
          diagnostic["pressSent"] as? Bool == false,
          matches.contains(where: { $0["role"] as? String == "AXGroup" && $0["exactListLabel"] as? Bool == true }) else {
        throw ProbeError("diagnostic_role_mismatch_missing")
    }
    let encoded = String(decoding: try JSONSerialization.data(withJSONObject: diagnostic), as: UTF8.self)
    guard !encoded.contains(list), !encoded.contains(title), !encoded.contains("Recents"), !encoded.contains(threadID) else {
        throw ProbeError("diagnostic_label_leak")
    }
    checks.append("diagnostics_report_role_mismatch_without_labels")
    let localized = diagnostics(rows, list: "Fixture project中的已安排任务", title: title, project: "Fixture project", threadID: threadID)
    guard localized["selectorResult"] as? String == "project_list_missing",
          (localized["matchingNodes"] as! [[String: Any]]).contains(where: {
              $0["role"] as? String == "AXList" && $0["projectNameInLabel"] as? Bool == true &&
                  $0["exactListLabel"] as? Bool == false
          }) else { throw ProbeError("diagnostic_label_mismatch_missing") }
    checks.append("diagnostics_report_label_mismatch_without_press")
    let matched = diagnostics(rows, list: list, title: title, project: "Fixture project", threadID: threadID)
    let matchedJSON = String(decoding: try JSONSerialization.data(withJSONObject: matched), as: UTF8.self)
    guard matched["exactThreadIdentityProven"] as? Bool == true, !matchedJSON.contains(threadID), !matchedJSON.contains(otherID) else {
        throw ProbeError("identity_diagnostic_leak")
    }
    checks.append("native_identity_values_are_redacted")
    emit(["ok": true, "mode": "self-test", "checks": checks])
}

func run() throws {
    var args: [String: String] = [:]
    for raw in CommandLine.arguments.dropFirst() {
        let parts = raw.split(separator: "=", maxSplits: 1).map(String.init)
        guard parts.count == 2, parts[0].hasPrefix("--"), !parts[1].isEmpty,
              args[String(parts[0].dropFirst(2))] == nil else { throw ProbeError("invalid_arguments") }
        args[String(parts[0].dropFirst(2))] = parts[1]
    }
    let known: Set<String> = ["mode", "pid", "bundle-id", "window-title", "project-list-label", "chat-title", "thread-id", "project-name"]
    guard Set(args.keys).isSubset(of: known) else { throw ProbeError("unknown_argument") }
    let mode = args["mode"] ?? "inspect"
    if mode == "self-test" { try selfTest(); return }
    let trusted = AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: false] as CFDictionary)
    if mode == "permission" { emit(["ok": true, "trusted": trusted]); return }
    let diagnose = mode == "diagnose" || mode == "enable-accessibility"
    guard mode == "inspect" || mode == "press" || diagnose else { throw ProbeError("invalid_mode") }
    guard trusted else { throw ProbeError("accessibility_permission_required") }
    guard let rawPID = args["pid"], let pid = Int32(rawPID), pid > 0,
          let bundleID = args["bundle-id"], let windowTitle = args["window-title"],
          let list = args["project-list-label"], let rawThreadID = args["thread-id"] else {
        throw ProbeError("target_arguments_missing")
    }
    guard let threadID = canonicalThreadID(rawThreadID) else { throw ProbeError("invalid_native_thread_id") }
    let title = args["chat-title"] ?? ""
    if diagnose && args["project-name"] == nil { throw ProbeError("project_name_required") }
    guard let app = NSRunningApplication(processIdentifier: pid) else { throw ProbeError("target_not_running") }
    guard app.bundleIdentifier == bundleID else { throw ProbeError("target_bundle_mismatch") }
    // NSRunningApplication.launchDate can be nil for a directly launched executable.
    let start = try processStart(pid)
    let target = AXUIElementCreateApplication(pid)
    // The system-wide object sets the default timeout for this client process only.
    guard AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), 0.25) == .success else {
        throw ProbeError("ax_timeout_setup")
    }
    let windows = try elements(target, kAXWindowsAttribute).filter {
        try attribute($0, kAXTitleAttribute) as? String == windowTitle
    }
    guard windows.count == 1 else { throw ProbeError("window_missing_or_ambiguous") }
    let window = windows[0]
    var rows = try snapshot(window, list: list, diagnose: diagnose)
    if diagnose {
        let beforeCount = rows.count
        var requestSent = false
        var setResult: AXError? = nil
        if mode == "enable-accessibility" && !rows.contains(where: { $0.role == "AXWebArea" }) {
            guard let sameApp = NSRunningApplication(processIdentifier: pid), !sameApp.isTerminated,
                  sameApp.bundleIdentifier == bundleID, try processStart(pid) == start else {
                throw ProbeError("target_identity_changed")
            }
            // BrowserCrApplication handles this attribute; AXManualAccessibility is Electron-only.
            // Send it once. Repeating it during polling can restart Chromium's two-second delay.
            requestSent = true
            setResult = AXUIElementSetAttributeValue(target, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
            // Chromium handles the request before calling AppKit's superclass setter.
            // AppKit can return notImplemented even when activation is pending.
            // Preserve the return code, and confirm the result by observing the tree.
            let deadline = ProcessInfo.processInfo.systemUptime + 8
            repeat {
                Thread.sleep(forTimeInterval: 0.2)
                rows = try snapshot(window, list: list, diagnose: true)
                if rows.contains(where: { $0.role == "AXWebArea" }) { break }
            } while ProcessInfo.processInfo.systemUptime < deadline
        }
        var result = diagnostics(rows, list: list, title: title, project: args["project-name"]!, threadID: threadID)
        result["mode"] = mode
        result["accessibilityRequestSent"] = requestSent
        if mode == "enable-accessibility" {
            result["accessibilityAttribute"] = "AXEnhancedUserInterface"
            result["nodeCountBefore"] = beforeCount
            if let setResult {
                result["accessibilitySetResult"] = setResult.rawValue
                result["accessibilitySetAcknowledged"] = setResult == .success
            }
            if result["rendererExposed"] as? Bool != true {
                result["ok"] = false
                result["code"] = setResult == .attributeUnsupported
                    ? "accessibility_attribute_unsupported" : "renderer_tree_unavailable"
            }
        }
        emit(result)
        if result["ok"] as? Bool != true { exit(1) }
        return
    }
    let index = try select(rows, list: list, threadID: threadID)
    let row = rows[index].element!
    var actionValues: CFArray?
    let actionResult = AXUIElementCopyActionNames(row, &actionValues)
    guard actionResult == .success, let actions = actionValues as? [String] else {
        throw ProbeError("ax_actions_\(actionResult.rawValue)")
    }
    let enabled = try attribute(row, kAXEnabledAttribute) as? Bool == true
    let hasPress = actions.contains(kAXPressAction)
    let beforeCount = try projectButtons(rows, list: list).buttons.count
    var result: [String: Any] = [
        "ok": true, "mode": mode, "matchBasis": "project_list_and_native_thread_id",
        "exactThreadIdentityProven": true, "projectButtonCount": beforeCount,
        "identitySources": try identitySources(rows, button: index, list: list, threadID: threadID),
        "identityValuesRedacted": true,
        "pressSupported": hasPress, "enabled": enabled, "currentBefore": rows[index].current ?? "",
        "pressSent": false,
    ]
    if mode == "inspect" { emit(result); return }
    guard enabled && hasPress else { throw ProbeError("row_not_pressable") }
    // Re-read immediately before the one press. Never retry an action after a timeout.
    let fresh = try snapshot(window, list: list)
    let freshIndex = try select(fresh, list: list, threadID: threadID)
    guard CFEqual(row, fresh[freshIndex].element!) else { throw ProbeError("row_changed") }
    guard let sameApp = NSRunningApplication(processIdentifier: pid), !sameApp.isTerminated,
          sameApp.bundleIdentifier == bundleID, try processStart(pid) == start else {
        throw ProbeError("target_identity_changed")
    }
    if fresh[freshIndex].current == "page" {
        result["currentAfter"] = "page"
        result["alreadyCurrent"] = true
        emit(result)
        return
    }
    pressSent = true
    let pressed = AXUIElementPerformAction(row, kAXPressAction as CFString)
    result["pressResult"] = pressed.rawValue
    result["pressSent"] = true
    guard pressed == .success || pressed == .cannotComplete else {
        throw ProbeError("ax_press_\(pressed.rawValue)")
    }
    let deadline = ProcessInfo.processInfo.systemUptime + 3
    repeat {
        let after = try snapshot(window, list: list)
        let selected = try select(after, list: list, threadID: threadID)
        if after[selected].current == "page" {
            let afterCount = try projectButtons(after, list: list).buttons.count
            guard afterCount == beforeCount else { throw ProbeError("project_row_count_changed") }
            result["currentAfter"] = "page"
            result["projectButtonCountAfter"] = afterCount
            emit(result)
            return
        }
        Thread.sleep(forTimeInterval: 0.1)
    } while ProcessInfo.processInfo.systemUptime < deadline
    throw ProbeError("project_selection_unconfirmed")
}

do { try run() }
catch let error as ProbeError { emit(["ok": false, "code": error.code, "pressSent": pressSent]); exit(1) }
catch { emit(["ok": false, "code": "unexpected_error", "pressSent": pressSent]); exit(1) }
