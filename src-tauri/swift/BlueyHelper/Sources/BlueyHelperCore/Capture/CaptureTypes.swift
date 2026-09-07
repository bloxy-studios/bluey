import Foundation

/// Params/result payloads shared by capture.display/window/region/activeWindow
/// (docs/HELPER_PROTOCOL.md "capture").
public struct CaptureParams: Decodable {
    // Common
    public var format: ImageFormat?
    public var quality: Double?
    public var maxDimension: Int?
    public var inline: Bool?
    public var changeDetection: Bool?
    public var excludeSelf: Bool?
    // Per-method
    public var displayId: String?
    public var windowId: Int?
    public var rect: RectJSON?

    public init() {}

    // Documented defaults.
    public var resolvedFormat: ImageFormat { format ?? .jpeg }
    public var resolvedQuality: Double { quality ?? 0.8 }
    public var resolvedMaxDimension: Int { maxDimension ?? 1600 }
    public var resolvedInline: Bool { inline ?? false }
    public var resolvedChangeDetection: Bool { changeDetection ?? true }
    public var resolvedExcludeSelf: Bool { excludeSelf ?? true }
}

/// The `Frame` result object.
public struct Frame: Encodable {
    public let id: String
    public let path: String?
    public let image: String? // base64 when inline
    public let mimeType: String
    public let width: Int
    public let height: Int
    public let displayId: String?
    public let scaleFactor: Double
    public let capturedAt: String
    public let hash: String
    public let changed: Bool
    public let durationMs: Int

    public init(
        id: String, path: String?, image: String?, mimeType: String,
        width: Int, height: Int, displayId: String?, scaleFactor: Double,
        capturedAt: String, hash: String, changed: Bool, durationMs: Int
    ) {
        self.id = id
        self.path = path
        self.image = image
        self.mimeType = mimeType
        self.width = width
        self.height = height
        self.displayId = displayId
        self.scaleFactor = scaleFactor
        self.capturedAt = capturedAt
        self.hash = hash
        self.changed = changed
        self.durationMs = durationMs
    }
}

/// `observe.start` params.
public struct ObserveParams: Decodable {
    public var intervalMs: Int?
    public var displayId: String?
    public var minDelta: Double?

    public var resolvedIntervalMs: Int { max(100, intervalMs ?? 1500) }
    public var resolvedMinDelta: Double { min(1.0, max(0.0, minDelta ?? 0.04)) }
}

/// `screen.changed` event payload.
public struct ScreenChangedEvent: Encodable {
    public let hash: String
    public let delta: Double
    public let displayId: String?
    public let at: String
}
