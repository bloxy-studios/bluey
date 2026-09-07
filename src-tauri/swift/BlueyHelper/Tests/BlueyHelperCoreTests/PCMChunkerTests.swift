import XCTest

@testable import BlueyHelperCore

final class PCMChunkerTests: XCTestCase {
    func testChunkSizeMath() {
        // 16 kHz, 200 ms → 3200 samples per chunk.
        let chunker = PCMChunker(sampleRate: 16000, chunkMs: 200)
        XCTAssertEqual(chunker.samplesPerChunk, 3200)
    }

    func testChunkBoundariesAndTimestamps() {
        let chunker = PCMChunker(sampleRate: 16000, chunkMs: 200)

        // 5000 samples → one 3200-sample chunk (0–200 ms), 1800 pending.
        let first = chunker.append([Int16](repeating: 1, count: 5000))
        XCTAssertEqual(first.count, 1)
        XCTAssertEqual(first[0].samples.count, 3200)
        XCTAssertEqual(first[0].startMs, 0, accuracy: 1e-9)
        XCTAssertEqual(first[0].endMs, 200, accuracy: 1e-9)
        XCTAssertEqual(chunker.positionMs, 5000.0 / 16.0, accuracy: 1e-9)

        // +2000 samples = 3800 pending → one chunk (200–400 ms), 600 left.
        let second = chunker.append([Int16](repeating: 2, count: 2000))
        XCTAssertEqual(second.count, 1)
        XCTAssertEqual(second[0].startMs, 200, accuracy: 1e-9)
        XCTAssertEqual(second[0].endMs, 400, accuracy: 1e-9)

        // Flush emits the 600-sample remainder: 400 ms → 437.5 ms.
        let tail = chunker.flush()
        XCTAssertNotNil(tail)
        XCTAssertEqual(tail!.samples.count, 600)
        XCTAssertEqual(tail!.startMs, 400, accuracy: 1e-9)
        XCTAssertEqual(tail!.endMs, 400 + 600.0 / 16.0, accuracy: 1e-9)

        // Nothing left.
        XCTAssertNil(chunker.flush())
    }

    func testMultipleChunksFromOneAppend() {
        let chunker = PCMChunker(sampleRate: 16000, chunkMs: 100) // 1600 samples/chunk
        let chunks = chunker.append([Int16](repeating: 0, count: 5000))
        XCTAssertEqual(chunks.count, 3)
        XCTAssertEqual(chunks[0].startMs, 0, accuracy: 1e-9)
        XCTAssertEqual(chunks[1].startMs, 100, accuracy: 1e-9)
        XCTAssertEqual(chunks[2].startMs, 200, accuracy: 1e-9)
        XCTAssertEqual(chunks[2].endMs, 300, accuracy: 1e-9)
    }

    func testSampleContinuityAcrossChunks() {
        let chunker = PCMChunker(sampleRate: 16000, chunkMs: 100)
        let input = (0..<3200).map { Int16($0 % 1000) }
        let chunks = chunker.append(input)
        XCTAssertEqual(chunks.count, 2)
        XCTAssertEqual(chunks[0].samples + chunks[1].samples, input, "no samples lost or reordered")
    }

    func testEmptyAppend() {
        let chunker = PCMChunker(sampleRate: 16000, chunkMs: 200)
        XCTAssertTrue(chunker.append([]).isEmpty)
        XCTAssertEqual(chunker.positionMs, 0)
    }

    func testTinyChunkMsIsClampedTo10Ms() {
        // Guard against divide-by-zero / pathological chunk sizes.
        let chunker = PCMChunker(sampleRate: 16000, chunkMs: 1)
        XCTAssertEqual(chunker.samplesPerChunk, 160) // 10 ms floor
    }
}
