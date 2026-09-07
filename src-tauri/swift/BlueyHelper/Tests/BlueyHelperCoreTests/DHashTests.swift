import XCTest

@testable import BlueyHelperCore

final class DHashTests: XCTestCase {
    /// Grid where each row increases left→right → no "left > right" pairs → hash 0.
    private func increasingGrid() -> [Double] {
        var g = [Double]()
        for _ in 0..<DHash.gridHeight {
            for col in 0..<DHash.gridWidth {
                g.append(Double(col))
            }
        }
        return g
    }

    /// Decreasing rows → every pair sets its bit → all 64 bits set.
    private func decreasingGrid() -> [Double] {
        var g = [Double]()
        for _ in 0..<DHash.gridHeight {
            for col in 0..<DHash.gridWidth {
                g.append(Double(DHash.gridWidth - col))
            }
        }
        return g
    }

    func testAllZeroAndAllOneHashes() {
        XCTAssertEqual(DHash.compute(gray: increasingGrid()), 0)
        XCTAssertEqual(DHash.compute(gray: decreasingGrid()), UInt64.max)
    }

    func testHammingAndDelta() {
        XCTAssertEqual(DHash.hamming(0, 0), 0)
        XCTAssertEqual(DHash.hamming(0, UInt64.max), 64)
        XCTAssertEqual(DHash.delta(0, UInt64.max), 1.0, accuracy: 1e-12)
        XCTAssertEqual(DHash.delta(0b1111, 0b0000), 4.0 / 64.0, accuracy: 1e-12)
        // Symmetry
        XCTAssertEqual(DHash.hamming(0b1010, 0b0101), DHash.hamming(0b0101, 0b1010))
    }

    func testSingleBitFlipChangesExactlyOneBit() {
        var grid = increasingGrid()
        // Flip the relationship of the first pair in row 0.
        grid[0] = 100
        let h = DHash.compute(gray: grid)
        XCTAssertEqual(h.nonzeroBitCount, 1)
        XCTAssertEqual(h, 1) // bit 0
    }

    func testHexFormat() {
        XCTAssertEqual(DHash.hex(0), "0000000000000000")
        XCTAssertEqual(DHash.hex(UInt64.max), "ffffffffffffffff")
        XCTAssertEqual(DHash.hex(0xdead_beef), "00000000deadbeef")
    }

    func testChangeDetectorFirstFrameIsChanged() {
        let detector = ChangeDetector()
        let eval = detector.evaluate(target: "display:1", hash: 42, minDelta: 0.04)
        XCTAssertTrue(eval.changed)
        XCTAssertEqual(eval.delta, 1.0)
    }

    func testChangeDetectorStableFrameIsUnchanged() {
        let detector = ChangeDetector()
        _ = detector.evaluate(target: "display:1", hash: 42, minDelta: 0.04)
        let eval = detector.evaluate(target: "display:1", hash: 42, minDelta: 0.04)
        XCTAssertFalse(eval.changed)
        XCTAssertEqual(eval.delta, 0.0)
    }

    func testChangeDetectorThreshold() {
        let detector = ChangeDetector()
        _ = detector.evaluate(target: "t", hash: 0, minDelta: 0.04)
        // 2 flipped bits = 2/64 = 0.03125 < 0.04 → unchanged
        let small = detector.evaluate(target: "t", hash: 0b11, minDelta: 0.04)
        XCTAssertFalse(small.changed)
        XCTAssertEqual(small.delta, 2.0 / 64.0, accuracy: 1e-12)
        // 3 more flipped bits vs previous (0b11): delta 3/64 = 0.046875 ≥ 0.04 → changed
        let big = detector.evaluate(target: "t", hash: 0b11111, minDelta: 0.04)
        XCTAssertTrue(big.changed)
    }

    func testChangeDetectorPerTargetMemory() {
        let detector = ChangeDetector()
        _ = detector.evaluate(target: "a", hash: 0, minDelta: 0.04)
        // Different target has its own memory → first frame changed.
        let other = detector.evaluate(target: "b", hash: 0, minDelta: 0.04)
        XCTAssertTrue(other.changed)
        // Original target unaffected.
        let same = detector.evaluate(target: "a", hash: 0, minDelta: 0.04)
        XCTAssertFalse(same.changed)
    }
}
