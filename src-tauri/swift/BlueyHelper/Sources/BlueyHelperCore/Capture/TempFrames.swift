import Foundation

/// Temp frame files: ~/Library/Caches/com.codewithabdul.bluey/frames/<id>.<ext>
/// * created lazily, stale files (> 1 h) removed at helper startup;
/// * `capture.discard` deletes a single frame (path must live inside the dir).
public final class TempFrames {
    public let directory: URL
    private let fm = FileManager.default

    public init() {
        let caches =
            fm.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? fm.temporaryDirectory
        directory =
            caches
            .appendingPathComponent(ShareableContent.blueyBundleId, isDirectory: true)
            .appendingPathComponent("frames", isDirectory: true)
        try? fm.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    public struct DiscardParams: Decodable {
        public let path: String
    }

    public func url(id: String, fileExtension: String) -> URL {
        directory.appendingPathComponent("\(id).\(fileExtension)")
    }

    /// Delete frames older than `maxAge` seconds (default 1 h) — run at startup.
    public func cleanupStale(maxAge: TimeInterval = 3600) {
        let cutoff = Date().addingTimeInterval(-maxAge)
        guard
            let items = try? fm.contentsOfDirectory(
                at: directory,
                includingPropertiesForKeys: [.contentModificationDateKey],
                options: [.skipsHiddenFiles])
        else { return }
        var removed = 0
        for item in items {
            let values = try? item.resourceValues(forKeys: [.contentModificationDateKey])
            if let modified = values?.contentModificationDate, modified < cutoff {
                try? fm.removeItem(at: item)
                removed += 1
            }
        }
        if removed > 0 {
            Log.shared.info("removed \(removed) stale temp frame(s)")
        }
    }

    /// `capture.discard`: remove one frame. The path is validated to be inside
    /// the frames directory so a confused caller can never delete arbitrary files.
    public func discard(path: String) -> Result<OkResult, HelperError> {
        let requested = URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
            .standardizedFileURL.resolvingSymlinksInPath()
        let root = directory.standardizedFileURL.resolvingSymlinksInPath()
        guard requested.path.hasPrefix(root.path + "/") else {
            return .failure(.invalidParams("path is outside the frames directory"))
        }
        if fm.fileExists(atPath: requested.path) {
            do {
                try fm.removeItem(at: requested)
            } catch {
                return .failure(.internalError("failed to delete frame: \(error.localizedDescription)"))
            }
        }
        // Idempotent: discarding an already-deleted frame is OK.
        return .success(OkResult())
    }
}
