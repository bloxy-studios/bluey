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

    /// PERF-015: display captures reuse an enumeration for up to 1.5 s while the
    /// display configuration is unchanged. Only content that lists Bluey's own
    /// app is kept, so `displayFilter` can exclude Bluey by app and a Bluey
    /// window opened after the fetch is still left out of the frame.
    static let displayCaptureContent = ContentCache<SCShareableContent>(
        ttlMs: 1500, now: Clock.monotonicMs, key: displayConfigurationKey,
        shouldCache: { !ownApplications(in: $0).isEmpty },
        fetch: { fetch(completion: $0) })

    public static func fetchForDisplayCapture(
        completion: @escaping (Result<SCShareableContent, HelperError>) -> Void
    ) {
        guard CGPreflightScreenCaptureAccess() else {
            displayCaptureContent.invalidate()
            completion(.failure(.permissionDenied("screenRecording", message: "Screen Recording not granted")))
            return
        }
        displayCaptureContent.get(completion: completion)
    }

    /// Active displays and their bounds: cheap to read, and it changes whenever
    /// a display is added, removed, re-arranged or changes resolution.
    static func displayConfigurationKey() -> String {
        var count: UInt32 = 0
        // https://developer.apple.com/documentation/coregraphics/1454603-cggetactivedisplaylist
        guard CGGetActiveDisplayList(0, nil, &count) == .success, count > 0 else { return "" }
        var ids = [CGDirectDisplayID](repeating: 0, count: Int(count))
        guard CGGetActiveDisplayList(count, &ids, &count) == .success else { return "" }
        return ids.prefix(Int(count)).map { id -> String in
            let b = CGDisplayBounds(id)
            return "\(id):\(b.origin.x),\(b.origin.y),\(b.width),\(b.height)"
        }.joined(separator: ";")
    }

    public static func ownApplications(in content: SCShareableContent) -> [SCRunningApplication] {
        content.applications.filter { app in
            app.processID == getpid() || app.processID == getppid() || app.bundleIdentifier == blueyBundleId
        }
    }

    /// A display filter without Bluey: by app when Bluey is enumerated (covers
    /// windows created later), else by the windows in `content`.
    public static func displayFilter(
        _ display: SCDisplay, excludingSelf: Bool, in content: SCShareableContent
    ) -> SCContentFilter {
        guard excludingSelf else { return SCContentFilter(display: display, excludingWindows: []) }
        let apps = ownApplications(in: content)
        guard !apps.isEmpty else {
            return SCContentFilter(display: display, excludingWindows: ownWindows(in: content))
        }
        // https://developer.apple.com/documentation/screencapturekit/sccontentfilter/init(display:excludingapplications:exceptingwindows:)
        return SCContentFilter(display: display, excludingApplications: apps, exceptingWindows: [])
    }

    public static func ownWindows(in content: SCShareableContent) -> [SCWindow] {
        content.windows.filter { isOwnWindow($0) }
    }

    /// Resolve a display by its stringified CGDirectDisplayID; nil → the display
    /// with focus (see FocusDisplay), not simply the menu-bar display.
    public static func display(
        withId id: String?, in content: SCShareableContent
    ) -> SCDisplay? {
        if let id, let numeric = UInt32(id) {
            return content.displays.first { $0.displayID == numeric }
        }
        let focused = FocusDisplay.resolve(
            displays: content.displays.map { (id: $0.displayID, frame: $0.frame) },
            focusedWindow: FocusDisplay.focusedWindowBounds(),
            mouse: FocusDisplay.mouseLocation(),
            mainDisplayID: CGMainDisplayID())
        return content.displays.first { $0.displayID == focused } ?? content.displays.first
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
