import AppKit
import Foundation
import ScreenCaptureKit

/// `windows.list` — SCShareableContent windows, excluding Bluey's own windows
/// and desktop/menu-bar chrome.
public final class WindowService {
    public init() {}

    public struct Params: Decodable {
        public var onScreenOnly: Bool?
    }

    public struct WindowInfo: Encodable {
        public let windowId: Int
        public let title: String?
        public let ownerName: String?
        public let bundleId: String?
        public let pid: Int
        public let bounds: RectJSON
        public let onScreen: Bool
    }

    public struct ListResult: Encodable {
        public let windows: [WindowInfo]
    }

    public func list(params: Params?, completion: @escaping (Result<ListResult, HelperError>) -> Void) {
        let onScreenOnly = params?.onScreenOnly ?? true
        ShareableContent.fetch(onScreenWindowsOnly: onScreenOnly) { result in
            switch result {
            case .failure(let error):
                completion(.failure(error))
            case .success(let content):
                let windows = content.windows.compactMap { Self.map($0) }
                completion(.success(ListResult(windows: windows)))
            }
        }
    }

    static func map(_ window: SCWindow) -> WindowInfo? {
        if ShareableContent.isOwnWindow(window) { return nil }
        // windowLayer == 0 keeps normal app windows; menu bar (layer 24/25),
        // status items and overlays are excluded. Desktop windows are already
        // excluded by excludingDesktopWindows(true, …).
        // https://developer.apple.com/documentation/screencapturekit/scwindow/windowlayer
        guard window.windowLayer == 0 else { return nil }

        let app = window.owningApplication
        // SCRunningApplication.bundleIdentifier can be empty for some processes;
        // fall back to NSRunningApplication.
        var bundleId = app?.bundleIdentifier
        if bundleId?.isEmpty ?? true, let pid = app?.processID {
            bundleId = NSRunningApplication(processIdentifier: pid)?.bundleIdentifier
        }

        return WindowInfo(
            windowId: Int(window.windowID),
            title: window.title,
            ownerName: app?.applicationName,
            bundleId: bundleId,
            pid: Int(app?.processID ?? 0),
            // SCWindow.frame is in screen points, global top-left-origin coords:
            // https://developer.apple.com/documentation/screencapturekit/scwindow/frame
            bounds: RectJSON(window.frame),
            onScreen: window.isOnScreen)
    }
}
