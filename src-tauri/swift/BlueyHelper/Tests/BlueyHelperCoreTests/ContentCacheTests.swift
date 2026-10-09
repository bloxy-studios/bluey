import XCTest

@testable import BlueyHelperCore

/// PERF-015: ⌘↵ display captures reuse a recent shareable-content enumeration
/// instead of re-enumerating every window on each press.
final class ContentCacheTests: XCTestCase {
    private var nowMs: Double = 0
    private var displayKey = "1:0,0,1440,900"
    private var fetches = 0

    private func makeCache(shouldCache: @escaping (Int) -> Bool = { _ in true }) -> ContentCache<Int> {
        ContentCache<Int>(
            ttlMs: 1500, now: { self.nowMs }, key: { self.displayKey }, shouldCache: shouldCache,
            fetch: { done in
                self.fetches += 1
                done(.success(self.fetches))
            })
    }

    private func get(_ cache: ContentCache<Int>) -> Int? {
        var value: Int?
        cache.get { value = try? $0.get() }
        return value
    }

    func testTwoCapturesWithinTheTtlFetchOnce() {
        let cache = makeCache()
        XCTAssertEqual(get(cache), 1)
        nowMs = 1_000
        XCTAssertEqual(get(cache), 1)
        XCTAssertEqual(fetches, 1)
    }

    func testRefetchesAfterTheTtl() {
        let cache = makeCache()
        _ = get(cache)
        nowMs = 1_500
        XCTAssertEqual(get(cache), 2)
    }

    func testRefetchesWhenTheDisplayConfigurationChanges() {
        let cache = makeCache()
        _ = get(cache)
        displayKey = "1:0,0,1440,900;2:1440,0,1920,1080"
        XCTAssertEqual(get(cache), 2)
    }

    func testInvalidateAndRejectedContentForceAFetch() {
        let cache = makeCache()
        _ = get(cache)
        cache.invalidate()
        XCTAssertEqual(get(cache), 2)

        let picky = makeCache(shouldCache: { _ in false })
        _ = get(picky)
        _ = get(picky)
        XCTAssertEqual(fetches, 4)
    }

    func testFailuresAreNotCached() {
        var calls = 0
        let cache = ContentCache<Int>(
            ttlMs: 1500, now: { 0 }, key: { "k" },
            fetch: { done in
                calls += 1
                done(.failure(.capture("shareable_content_failed", "boom")))
            })
        cache.get { _ in }
        cache.get { _ in }
        XCTAssertEqual(calls, 2)
    }
}
