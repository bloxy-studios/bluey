import XCTest

@testable import BlueyHelperCore

/// DATA-001: temp frames must not pile up in ~/Library/Caches.
final class TempFramesTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("bluey-temp-frames-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    private func write(_ name: String, modified: Date, in frames: TempFrames) throws -> URL {
        let url = frames.directory.appendingPathComponent(name)
        try Data("jpeg".utf8).write(to: url)
        try FileManager.default.setAttributes([.modificationDate: modified], ofItemAtPath: url.path)
        return url
    }

    func testSweepRemovesOnlyFramesOlderThanTheStaleAge() throws {
        let frames = TempFrames(directory: directory)
        let now = Date()
        let old = try write("f-old.jpg", modified: now.addingTimeInterval(-(TempFrames.staleAge + 5)), in: frames)
        let fresh = try write("f-new.jpg", modified: now.addingTimeInterval(-5), in: frames)

        XCTAssertEqual(frames.cleanupStale(now: now), 1)
        XCTAssertFalse(FileManager.default.fileExists(atPath: old.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: fresh.path))
    }

    func testTheSweepRunsWellBeforeAnHourAndRepeats() {
        // Startup-only cleanup of hour-old files let a day's captures pile up.
        XCTAssertLessThanOrEqual(TempFrames.staleAge, 900)
        XCTAssertLessThanOrEqual(TempFrames.sweepInterval, TempFrames.staleAge)
    }

    func testDiscardRefusesPathsOutsideTheFramesDirectory() throws {
        let frames = TempFrames(directory: directory)
        let inside = try write("f-1.jpg", modified: Date(), in: frames)
        guard case .success = frames.discard(path: inside.path) else {
            return XCTFail("a frame inside the directory is discarded")
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: inside.path))
        let outside = directory.appendingPathComponent("../elsewhere.jpg").path
        guard case .failure = frames.discard(path: outside) else {
            return XCTFail("a path outside the directory is refused")
        }
    }
}
