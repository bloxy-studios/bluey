import Foundation

/// PERF-015: a short-lived cache for the shareable-content enumeration (every
/// window and app on the system), which ⌘↵ display captures otherwise redo on
/// each press. An entry is reused only while it is younger than `ttlMs` and the
/// display configuration (`key`) is unchanged, so a plugged/unplugged or
/// re-arranged display always refetches. Generic so tests can drive it without
/// ScreenCaptureKit.
public final class ContentCache<Content> {
    public typealias Fetch = (@escaping (Result<Content, HelperError>) -> Void) -> Void

    private let ttlMs: Double
    private let now: () -> Double
    private let key: () -> String
    private let fetchContent: Fetch
    private let shouldCache: (Content) -> Bool
    private let lock = NSLock()
    private var entry: (content: Content, atMs: Double, key: String)?

    public init(
        ttlMs: Double, now: @escaping () -> Double, key: @escaping () -> String,
        shouldCache: @escaping (Content) -> Bool = { _ in true }, fetch: @escaping Fetch
    ) {
        self.ttlMs = ttlMs
        self.now = now
        self.key = key
        self.fetchContent = fetch
        self.shouldCache = shouldCache
    }

    /// The cached content when fresh, else a new fetch (kept when it succeeds
    /// and `shouldCache` accepts it).
    public func get(completion: @escaping (Result<Content, HelperError>) -> Void) {
        let currentKey = key()
        let startedMs = now()
        lock.lock()
        let hit = entry.flatMap { cached in
            cached.key == currentKey && startedMs - cached.atMs < ttlMs ? cached.content : nil
        }
        lock.unlock()
        if let hit {
            completion(.success(hit))
            return
        }
        fetchContent { [weak self] result in
            if let self, case .success(let content) = result, self.shouldCache(content) {
                self.lock.lock()
                self.entry = (content, startedMs, currentKey)
                self.lock.unlock()
            }
            completion(result)
        }
    }

    public func invalidate() {
        lock.lock()
        entry = nil
        lock.unlock()
    }
}
