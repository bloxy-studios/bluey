import AppKit
import CoreGraphics
import Foundation

/// Picks the display the user is working on for "Display with focus" captures
/// (a capture request without a displayId).
public enum FocusDisplay {
    /// Pure resolver: the display containing the midpoint of the frontmost app's
    /// front window, else the display under the mouse, else the main display.
    /// All geometry is in global top-left-origin (Quartz) points, the space of
    /// SCDisplay.frame, kCGWindowBounds and CGEvent.location.
    public static func resolve(
        displays: [(id: UInt32, frame: CGRect)],
        focusedWindow: CGRect?,
        mouse: CGPoint?,
        mainDisplayID: UInt32
    ) -> UInt32? {
        if let window = focusedWindow {
            let mid = CGPoint(x: window.midX, y: window.midY)
            if let hit = displays.first(where: { $0.frame.contains(mid) }) { return hit.id }
        }
        if let mouse, let hit = displays.first(where: { $0.frame.contains(mouse) }) {
            return hit.id
        }
        if displays.contains(where: { $0.id == mainDisplayID }) { return mainDisplayID }
        return displays.first?.id
    }

    /// Bounds of the frontmost application's front layer-0 window, if any.
    static func focusedWindowBounds() -> CGRect? {
        // https://developer.apple.com/documentation/appkit/nsworkspace/frontmostapplication
        guard let pid = NSWorkspace.shared.frontmostApplication?.processIdentifier,
            let bounds = FrontmostAppService.frontmostWindowInfo(pid: pid)?.bounds
        else { return nil }
        return CGRect(x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height)
    }

    /// Mouse position in Quartz coordinates (NSEvent.mouseLocation is
    /// bottom-left-origin Cocoa space, so a null-source CGEvent is used instead).
    static func mouseLocation() -> CGPoint? {
        CGEvent(source: nil)?.location
    }
}
