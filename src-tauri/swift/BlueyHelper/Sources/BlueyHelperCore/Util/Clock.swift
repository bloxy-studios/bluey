import Foundation

/// Time helpers: RFC 3339 timestamps with millisecond precision, plus a
/// monotonic millisecond clock for uptime / duration measurement.
public enum Clock {
    /// ISO8601DateFormatter is thread-safe (unlike pre-configured DateFormatter caveats):
    /// https://developer.apple.com/documentation/foundation/iso8601dateformatter
    /// `.withFractionalSeconds` yields "2026-09-07T12:00:00.000Z".
    private static let iso: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    public static func isoNow() -> String { iso.string(from: Date()) }

    public static func isoString(from date: Date) -> String { iso.string(from: date) }

    /// Monotonic milliseconds since boot; unaffected by wall-clock changes.
    /// https://developer.apple.com/documentation/dispatch/dispatchtime
    public static func monotonicMs() -> Double {
        Double(DispatchTime.now().uptimeNanoseconds) / 1_000_000.0
    }
}
