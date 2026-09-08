import AppKit
import Foundation
import ScreenCaptureKit

/// Shared SCShareableContent plumbing for capture/observe/window services.
public enum ShareableContent {
    public static let blueyBundleId = "com.codewithabdul.bluey"

    /// Async `SCShareableContent.excludingDesktopWindows(_:onScreenWindowsOnly:)`
    /// (the completion-handler overload was removed from current SDKs).
    public static func fetch(
        onScreenWindowsOnly: Bool = true,
        completion: @escaping (Result<SCShareableContent, HelperError>) -> Void
    ) {
        guard CGPreflightScreenCaptureAccess() else {
            completion(.failure(.permissionDenied("screenRecording", message: "Screen Recording not granted")))
            return
        }
        Task {
            do {
                let content = try await SCShareableContent.excludingDesktopWindows(
                    true,
                    onScreenWindowsOnly: onScreenWindowsOnly
                )
                completion(.success(content))
            } catch {
                completion(.failure(.capture("shareable_content_failed", error.localizedDescription)))
            }
        }
    }

    /// True when the window belongs to Bluey itself (the helper is a child of
    /// the .app: getppid() is the Tauri process; the helper owns no windows but
    /// getpid() is checked defensively).
    public static func isOwnWindow(_ window: SCWindow) -> Bool {
        guard let app = window.owningApplication else { return false }
        if app.processID == getpid() || app.processID == getppid() { return true }
        return app.bundleIdentifier == blueyBundleId
    }

    public static func ownWindows(in content: SCShareableContent) -> [SCWindow] {
        content.windows.filter { isOwnWindow($0) }
    }

    /// Resolve a display by its stringified CGDirectDisplayID; nil → main display.
    public static func display(
        withId id: String?, in content: SCShareableContent
    ) -> SCDisplay? {
        if let id, let numeric = UInt32(id) {
            return content.displays.first { $0.displayID == numeric }
        }
        let main = CGMainDisplayID()
        return content.displays.first { $0.displayID == main } ?? content.displays.first
    }

    public static func window(withId id: UInt32, in content: SCShareableContent) -> SCWindow? {
        content.windows.first { $0.windowID == id }
    }

    /// The display whose bounds contain the window's midpoint (for scale factor
    /// + displayId attribution of window captures).
    public static func display(containing window: SCWindow, in content: SCShareableContent) -> SCDisplay? {
        let mid = CGPoint(x: window.frame.midX, y: window.frame.midY)
        return content.displays.first { $0.frame.contains(mid) } ?? content.displays.first
    }

    /// backingScaleFactor of the NSScreen matching a CGDirectDisplayID.
    /// NSScreen.deviceDescription["NSScreenNumber"] is the documented bridge:
    /// https://developer.apple.com/documentation/appkit/nsscreen/devicedescription
    public static func scaleFactor(forDisplayID displayID: UInt32) -> Double {
        let key = NSDeviceDescriptionKey("NSScreenNumber")
        for screen in NSScreen.screens {
            if let n = screen.deviceDescription[key] as? NSNumber, n.uint32Value == displayID {
                return Double(screen.backingScaleFactor)
            }
        }
        return 2.0 // sensible Retina default
    }
}
