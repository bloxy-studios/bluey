import AppKit
import CoreGraphics
import Foundation

/// `displays.list` — sizes in points, origin in the global (Quartz, top-left)
/// coordinate space, per docs/HELPER_PROTOCOL.md.
public final class DisplayService {
    public init() {}

    public struct DisplayInfo: Encodable {
        public let id: String
        public let name: String
        public let width: Int
        public let height: Int
        public let x: Int
        public let y: Int
        public let scaleFactor: Double
        public let isMain: Bool
    }

    public struct ListResult: Encodable {
        public let displays: [DisplayInfo]
    }

    public func list(completion: @escaping (Result<ListResult, HelperError>) -> Void) {
        // NSScreen is an AppKit class; read it on the main queue for safety.
        DispatchQueue.main.async {
            let mainID = CGMainDisplayID()
            let key = NSDeviceDescriptionKey("NSScreenNumber")
            var infos: [DisplayInfo] = []

            for screen in NSScreen.screens {
                guard let number = screen.deviceDescription[key] as? NSNumber else { continue }
                let displayID = number.uint32Value
                // CGDisplayBounds returns the display rect in the global display
                // (Quartz, top-left-origin) coordinate space, in points:
                // https://developer.apple.com/documentation/coregraphics/cgdisplaybounds(_:)
                let bounds = CGDisplayBounds(displayID)
                // NSScreen.localizedName — macOS 10.15+:
                // https://developer.apple.com/documentation/appkit/nsscreen/localizedname
                infos.append(
                    DisplayInfo(
                        id: String(displayID),
                        name: screen.localizedName,
                        width: Int(bounds.width.rounded()),
                        height: Int(bounds.height.rounded()),
                        x: Int(bounds.origin.x.rounded()),
                        y: Int(bounds.origin.y.rounded()),
                        scaleFactor: Double(screen.backingScaleFactor),
                        isMain: displayID == mainID))
            }

            if infos.isEmpty {
                completion(.failure(.capture("no_displays", "no displays found")))
            } else {
                completion(.success(ListResult(displays: infos)))
            }
        }
    }
}
