import Foundation

/// Perceptual difference hash (dHash) + per-target change memory.
/// Pure Swift (no CoreGraphics types) so it is unit-testable on any host.
///
/// Algorithm: downscale to a 9×8 grayscale grid (done by the caller, see
/// ImageEncoder.grayGrid / ScreenObserver), then set one bit per horizontal
/// neighbour pair: bit = left > right → 64-bit hash. Change amount is the
/// Hamming distance divided by 64.
public enum DHash {
    public static let gridWidth = 9
    public static let gridHeight = 8
    public static let bitCount = 64

    /// `gray` must contain exactly 72 (9×8) luminance values, row-major.
    public static func compute(gray: [Double]) -> UInt64 {
        precondition(gray.count == gridWidth * gridHeight, "dHash needs a 9x8 grid")
        var hash: UInt64 = 0
        var bit = 0
        for row in 0..<gridHeight {
            for col in 0..<(gridWidth - 1) {
                let left = gray[row * gridWidth + col]
                let right = gray[row * gridWidth + col + 1]
                if left > right {
                    hash |= (1 << UInt64(bit))
                }
                bit += 1
            }
        }
        return hash
    }

    public static func hamming(_ a: UInt64, _ b: UInt64) -> Int {
        (a ^ b).nonzeroBitCount
    }

    /// Normalized distance in 0…1.
    public static func delta(_ a: UInt64, _ b: UInt64) -> Double {
        Double(hamming(a, b)) / Double(bitCount)
    }

    /// 16 lowercase hex chars, zero-padded (protocol `Frame.hash`).
    public static func hex(_ hash: UInt64) -> String {
        String(format: "%016llx", hash)
    }
}

/// Remembers the last hash per capture target ("display:1", "window:42",
/// "region:1:0,0,800,600", "observe:1") and evaluates whether a new frame
/// counts as changed. Thread-safe.
public final class ChangeDetector {
    public struct Evaluation {
        public let changed: Bool
        /// 1.0 for the first frame of a target.
        public let delta: Double
    }

    private var lastHashes: [String: UInt64] = [:]
    private let lock = NSLock()

    public init() {}

    public func evaluate(target: String, hash: UInt64, minDelta: Double) -> Evaluation {
        lock.lock()
        defer { lock.unlock() }
        let previous = lastHashes[target]
        lastHashes[target] = hash
        guard let previous else {
            return Evaluation(changed: true, delta: 1.0)
        }
        let delta = DHash.delta(previous, hash)
        return Evaluation(changed: delta >= minDelta, delta: delta)
    }

    public func reset(target: String? = nil) {
        lock.lock()
        defer { lock.unlock() }
        if let target {
            lastHashes.removeValue(forKey: target)
        } else {
            lastHashes.removeAll()
        }
    }
}
