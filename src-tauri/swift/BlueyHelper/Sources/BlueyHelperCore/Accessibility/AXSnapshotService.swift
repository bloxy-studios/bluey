import AppKit
import ApplicationServices
import Foundation

/// `accessibility.snapshot` — bounded traversal of the frontmost app's AX tree.
/// Budgets: maxDepth (default 6), maxElements (default 150), 250 ms deadline.
public final class AXSnapshotService {
    /// Per-message AX timeout so one unresponsive app cannot hang the helper.
    /// https://developer.apple.com/documentation/applicationservices/1462248-axuielementsetmessagingtimeout
    private static let messagingTimeoutSeconds: Float = 0.1
    private static let traversalBudget = DispatchTimeInterval.milliseconds(250)
    private static let focusedValueLimit = 4000
    private static let elementValueLimit = 1000
    private static let visibleTextLimit = 8000

    private let queue = DispatchQueue(label: "com.codewithabdul.bluey.helper.ax", qos: .userInitiated)

    public init() {}

    public struct Params: Decodable {
        public var maxDepth: Int?
        public var maxElements: Int?
        public var includeSelectedText: Bool?
    }

    public struct ElementJSON: Encodable {
        public var role: String
        public var label: String?
        public var value: String?
        public var position: PointJSON?
        public var size: SizeJSON?
        public var actions: [String]?
        public var focused: Bool?
        public var depth: Int
        public var url: String?
    }

    public struct WindowJSON: Encodable {
        public var title: String?
        public var windowId: Int?
        public var bounds: RectJSON?
    }

    public struct Snapshot: Encodable {
        public var application: FrontmostAppService.AppJSON
        public var window: WindowJSON?
        public var focusedElement: ElementJSON?
        public var elements: [ElementJSON]
        public var selectedText: String?
        public var visibleText: String
        public var truncated: Bool
        public var capturedAt: String
    }

    public func snapshot(
        _ params: Params?, completion: @escaping (Result<Snapshot, HelperError>) -> Void
    ) {
        guard AXIsProcessTrusted() else {
            completion(.failure(.permissionDenied("accessibility", message: "Accessibility not granted")))
            return
        }
        // NSWorkspace read on main, AX walking off main.
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard let app = NSWorkspace.shared.frontmostApplication else {
                completion(.failure(.capture("no_frontmost_app", "no frontmost application")))
                return
            }
            let appJSON = FrontmostAppService.AppJSON(
                name: app.localizedName, bundleId: app.bundleIdentifier,
                pid: Int(app.processIdentifier))
            let pid = app.processIdentifier
            self.queue.async {
                let snapshot = self.buildSnapshot(pid: pid, appJSON: appJSON, params: params)
                completion(.success(snapshot))
            }
        }
    }

    // MARK: - snapshot construction (runs on `queue`)

    private func buildSnapshot(
        pid: pid_t, appJSON: FrontmostAppService.AppJSON, params: Params?
    ) -> Snapshot {
        let maxDepth = max(1, params?.maxDepth ?? 6)
        let maxElements = max(1, params?.maxElements ?? 150)
        let includeSelectedText = params?.includeSelectedText ?? true
        let deadline = DispatchTime.now() + Self.traversalBudget

        let appElement = AXUIElementCreateApplication(pid)
        _ = AXUIElementSetMessagingTimeout(appElement, Self.messagingTimeoutSeconds)

        // Focused window --------------------------------------------------
        var windowJSON: WindowJSON?
        let focusedWindow = AX.element(appElement, kAXFocusedWindowAttribute)
        if let focusedWindow {
            let title = AX.string(focusedWindow, kAXTitleAttribute)
            var bounds: RectJSON?
            if let pos = AX.point(focusedWindow, kAXPositionAttribute),
                let size = AX.size(focusedWindow, kAXSizeAttribute)
            {
                bounds = RectJSON(
                    x: Double(pos.x), y: Double(pos.y),
                    width: Double(size.width), height: Double(size.height))
            }
            windowJSON = WindowJSON(
                title: title,
                windowId: Self.matchWindowId(pid: pid, title: title, bounds: bounds),
                bounds: bounds)
        }

        // Focused element --------------------------------------------------
        var focusedJSON: ElementJSON?
        var selectedText: String?
        let focusedElement = AX.element(appElement, kAXFocusedUIElementAttribute)
        if let focusedElement {
            focusedJSON = Self.describe(
                focusedElement, depth: 0, valueLimit: Self.focusedValueLimit,
                includeActions: true, markFocused: true)
            if includeSelectedText {
                // https://developer.apple.com/documentation/applicationservices — kAXSelectedTextAttribute
                selectedText = AX.string(focusedElement, kAXSelectedTextAttribute)
                if let s = selectedText, s.isEmpty { selectedText = nil }
            }
        }

        // Bounded DFS ------------------------------------------------------
        var truncated = false
        let root: AXUIElement = focusedWindow ?? appElement
        let elements = Self.traverse(
            root: root, maxDepth: maxDepth, maxElements: maxElements,
            deadline: deadline, truncated: &truncated)

        // visibleText: deduplicated, order-preserving, ≤ 8000 chars ---------
        var seen = Set<String>()
        var pieces: [String] = []
        var total = 0
        for element in elements {
            guard let text = element.value ?? element.label,
                !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            else { continue }
            guard !seen.contains(text) else { continue }
            seen.insert(text)
            if total + text.count + 1 > Self.visibleTextLimit {
                truncated = true
                break
            }
            pieces.append(text)
            total += text.count + 1
        }

        if let value = focusedJSON?.value, value.count >= Self.focusedValueLimit {
            truncated = true
        }

        return Snapshot(
            application: appJSON,
            window: windowJSON,
            focusedElement: focusedJSON,
            elements: elements,
            selectedText: selectedText,
            visibleText: pieces.joined(separator: "\n"),
            truncated: truncated,
            capturedAt: Clock.isoNow())
    }

    // MARK: - traversal

    private static func traverse(
        root: AXUIElement, maxDepth: Int, maxElements: Int,
        deadline: DispatchTime, truncated: inout Bool
    ) -> [ElementJSON] {
        var collected: [ElementJSON] = []
        // Explicit stack DFS (element, depth); children pushed in reverse to
        // preserve natural order.
        var stack: [(AXUIElement, Int)] = [(root, 0)]
        var visited = 0

        while let (element, depth) = stack.popLast() {
            if DispatchTime.now() >= deadline {
                truncated = true
                break
            }
            visited += 1
            if visited > maxElements * 8 {
                // Hard cap on AX round-trips even if few elements match.
                truncated = true
                break
            }

            let role = AX.role(element)
            if AXRoleFilter.isCollectible(role) {
                collected.append(
                    describe(
                        element, depth: depth, valueLimit: elementValueLimit,
                        includeActions: false, markFocused: false))
                if collected.count >= maxElements {
                    truncated = true
                    break
                }
            }

            if depth < maxDepth, AXRoleFilter.shouldDescend(into: role) {
                let children = AX.children(element)
                for child in children.reversed() {
                    stack.append((child, depth + 1))
                }
            } else if depth >= maxDepth {
                truncated = true
            }
        }
        return collected
    }

    static func describe(
        _ element: AXUIElement, depth: Int, valueLimit: Int,
        includeActions: Bool, markFocused: Bool
    ) -> ElementJSON {
        let role = AX.role(element) ?? "AXUnknown"
        var value = AX.valueString(element)
        if let v = value, v.count > valueLimit {
            value = String(v.prefix(valueLimit))
        }
        var position: PointJSON?
        if let p = AX.point(element, kAXPositionAttribute) {
            position = PointJSON(x: Double(p.x), y: Double(p.y))
        }
        var size: SizeJSON?
        if let s = AX.size(element, kAXSizeAttribute) {
            size = SizeJSON(width: Double(s.width), height: Double(s.height))
        }
        var url: String?
        if role == "AXWebArea" {
            url = AX.string(element, kAXURLAttribute)
        }
        return ElementJSON(
            role: role,
            label: AX.label(element),
            value: value,
            position: position,
            size: size,
            actions: includeActions ? AX.actions(element) : nil,
            focused: markFocused ? true : nil,
            depth: depth,
            url: url)
    }

    /// AX exposes no public window number; match the focused AX window against
    /// CGWindowList entries of the same pid by title, then by closest bounds.
    static func matchWindowId(pid: pid_t, title: String?, bounds: RectJSON?) -> Int? {
        let candidates = FrontmostAppService.windowDictionaries(pid: pid)
        guard !candidates.isEmpty else { return nil }
        if let title, !title.isEmpty {
            for info in candidates
            where (info[kCGWindowName as String] as? String) == title {
                return info[kCGWindowNumber as String] as? Int
            }
        }
        if let bounds {
            var best: (id: Int, distance: Double)?
            for info in candidates {
                guard let dict = info[kCGWindowBounds as String] as? NSDictionary,
                    let rect = CGRect(dictionaryRepresentation: dict),
                    let id = info[kCGWindowNumber as String] as? Int
                else { continue }
                let d =
                    abs(Double(rect.origin.x) - bounds.x) + abs(Double(rect.origin.y) - bounds.y)
                    + abs(Double(rect.width) - bounds.width) + abs(Double(rect.height) - bounds.height)
                if best == nil || d < best!.distance {
                    best = (id, d)
                }
            }
            return best?.id
        }
        return candidates.first?[kCGWindowNumber as String] as? Int
    }
}
