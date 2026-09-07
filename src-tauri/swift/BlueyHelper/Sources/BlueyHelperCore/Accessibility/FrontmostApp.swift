import AppKit
import CoreGraphics
import Foundation

/// `app.frontmost` — NSWorkspace for the application, CGWindowList for the
/// focused window (works without Accessibility permission; window *titles*
/// require Screen Recording on macOS 10.15+, otherwise title is null).
public enum FrontmostAppService {
    public struct AppJSON: Encodable {
        public let name: String?
        public let bundleId: String?
        public let pid: Int
    }

    public struct WindowJSON: Encodable {
        public let title: String?
        public let windowId: Int?
        public let bounds: RectJSON?
    }

    public struct FrontmostResult: Encodable {
        public let application: AppJSON
        public let window: WindowJSON?
    }

    public static func frontmost(completion: @escaping (Result<FrontmostResult, HelperError>) -> Void) {
        DispatchQueue.main.async {
            // https://developer.apple.com/documentation/appkit/nsworkspace/frontmostapplication
            guard let app = NSWorkspace.shared.frontmostApplication else {
                completion(.failure(.capture("no_frontmost_app", "no frontmost application")))
                return
            }
            let pid = app.processIdentifier
            let window = frontmostWindowInfo(pid: pid)
            completion(
                .success(
                    FrontmostResult(
                        application: AppJSON(
                            name: app.localizedName,
                            bundleId: app.bundleIdentifier,
                            pid: Int(pid)),
                        window: window)))
        }
    }

    /// First (frontmost) layer-0 window of `pid`.
    /// CGWindowListCopyWindowInfo returns windows in front-to-back order:
    /// https://developer.apple.com/documentation/coregraphics/cgwindowlistcopywindowinfo(_:_:)
    static func windowDictionaries(pid: pid_t) -> [[String: Any]] {
        let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
        guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]]
        else { return [] }
        return list.filter { info in
            guard let ownerPid = info[kCGWindowOwnerPID as String] as? Int, ownerPid == Int(pid)
            else { return false }
            let layer = info[kCGWindowLayer as String] as? Int ?? -1
            return layer == 0
        }
    }

    public static func frontmostWindowID(pid: pid_t) -> UInt32? {
        guard let info = windowDictionaries(pid: pid).first,
            let number = info[kCGWindowNumber as String] as? Int
        else { return nil }
        return UInt32(number)
    }

    static func frontmostWindowInfo(pid: pid_t) -> WindowJSON? {
        guard let info = windowDictionaries(pid: pid).first else { return nil }
        let windowId = info[kCGWindowNumber as String] as? Int
        // kCGWindowName requires Screen Recording permission on 10.15+.
        let title = info[kCGWindowName as String] as? String
        var bounds: RectJSON?
        if let dict = info[kCGWindowBounds as String] as? NSDictionary,
            // kCGWindowBounds is already in global top-left-origin (Quartz) points:
            // https://developer.apple.com/documentation/coregraphics/kcgwindowbounds
            let rect = CGRect(dictionaryRepresentation: dict)
        {
            bounds = RectJSON(rect)
        }
        return WindowJSON(title: title, windowId: windowId, bounds: bounds)
    }
}
