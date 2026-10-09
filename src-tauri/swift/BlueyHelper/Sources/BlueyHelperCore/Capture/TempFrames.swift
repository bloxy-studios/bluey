import Foundation

/// Temp frame files: ~/Library/Caches/com.codewithabdul.bluey/frames/<id>.<ext>
/// * created lazily; the app deletes a frame once it is used (DATA-001);
/// * stale files are swept at helper startup and every `sweepInterval`;
/// * `capture.discard` deletes a single frame (path must live inside the dir).
public final class TempFrames {
    /// Older frames are no longer referenced by the app (it keeps the last few).
    public static let staleAge: TimeInterval = 600
    public static let sweepInterval: TimeInterval = 300

    public let directory: URL
    private let fm = FileManager.default
    private var sweepTimer: DispatchSourceTimer?

    public convenience init() {
        let caches =
            FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        self.init(
            directory:
                caches
                .appendingPathComponent(ShareableContent.blueyBundleId, isDirectory: true)
                .appendingPathComponent("frames", isDirectory: true))
    }

    /// Test seam: frames in `directory`.
    public init(directory: URL) {
        self.directory = directory
        try? fm.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    /// Sweep stale frames now and every `sweepInterval` for the helper's lifetime.
    public func startPeriodicSweep(queue: DispatchQueue = .global(qos: .utility)) {
        cleanupStale()
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + Self.sweepInterval, repeating: Self.sweepInterval)
        timer.setEventHandler { [weak self] in self?.cleanupStale() }
        timer.resume()
        sweepTimer = timer
    }

    public struct DiscardParams: Decodable {
        public let path: String
    }

    public func url(id: String, fileExtension: String) -> URL {
        directory.appendingPathComponent("\(id).\(fileExtension)")
    }

    /// Delete frames older than `maxAge` seconds. Returns how many went.
    @discardableResult
    public func cleanupStale(maxAge: TimeInterval = TempFrames.staleAge, now: Date = Date()) -> Int {
        let cutoff = now.addingTimeInterval(-maxAge)
        guard
            let items = try? fm.contentsOfDirectory(
                at: directory,
                includingPropertiesForKeys: [.contentModificationDateKey],
                options: [.skipsHiddenFiles])
        else { return 0 }
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
        return removed
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
