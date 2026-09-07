import Foundation

/// Structured JSON logging to **stderr** — one line per entry:
///     {"at":"…","level":"info","message":"…"}
/// stdout is reserved exclusively for protocol responses/events
/// (docs/HELPER_PROTOCOL.md "Transport").
///
/// PRIVACY RULE: never log audio samples, transcript text, OCR text or image
/// bytes. Log only metadata (sizes, durations, codes).
public final class Log {
    public static let shared = Log()

    public enum Level: String { case debug, info, warn, error }

    private let queue = DispatchQueue(label: "com.codewithabdul.bluey.helper.log")

    private init() {}

    public func log(_ level: Level, _ message: String) {
        let entry: [String: String] = [
            "level": level.rawValue,
            "message": message,
            "at": Clock.isoNow(),
        ]
        guard var data = try? JSONSerialization.data(withJSONObject: entry, options: [.sortedKeys]) else {
            return
        }
        data.append(0x0A) // "\n"
        queue.async {
            // FileHandle.write(contentsOf:) issues an unbuffered write(2).
            try? FileHandle.standardError.write(contentsOf: data)
        }
    }

    public func debug(_ m: String) { log(.debug, m) }
    public func info(_ m: String) { log(.info, m) }
    public func warn(_ m: String) { log(.warn, m) }
    public func error(_ m: String) { log(.error, m) }
}
