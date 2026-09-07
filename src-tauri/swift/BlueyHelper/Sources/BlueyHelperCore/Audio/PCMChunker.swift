import Foundation

/// Slices a continuous 16 kHz mono Int16 stream into fixed-length chunks.
/// `startMs`/`endMs` are derived from the **sample count**, so they are exact
/// positions on the "milliseconds since audio.start" clock regardless of
/// callback jitter. Pure Swift, unit-tested.
public struct PCMChunk {
    public let samples: [Int16]
    public let startMs: Double
    public let endMs: Double
}

public final class PCMChunker {
    public let sampleRate: Int
    public let samplesPerChunk: Int

    private var pending: [Int16] = []
    /// Absolute sample index (since start) of pending[0].
    private var pendingStartSample: Int64 = 0

    public init(sampleRate: Int, chunkMs: Int) {
        self.sampleRate = sampleRate
        self.samplesPerChunk = max(1, sampleRate * max(10, chunkMs) / 1000)
    }

    /// Current stream position in ms (start of not-yet-emitted audio + pending).
    public var positionMs: Double {
        Double(pendingStartSample + Int64(pending.count)) * 1000.0 / Double(sampleRate)
    }

    public func append(_ samples: [Int16]) -> [PCMChunk] {
        guard !samples.isEmpty else { return [] }
        pending.append(contentsOf: samples)
        var chunks: [PCMChunk] = []
        while pending.count >= samplesPerChunk {
            let slice = Array(pending.prefix(samplesPerChunk))
            pending.removeFirst(samplesPerChunk)
            let startSample = pendingStartSample
            pendingStartSample += Int64(samplesPerChunk)
            chunks.append(makeChunk(slice, startSample: startSample))
        }
        return chunks
    }

    /// Emit whatever is left (used on stop).
    public func flush() -> PCMChunk? {
        guard !pending.isEmpty else { return nil }
        let slice = pending
        let startSample = pendingStartSample
        pendingStartSample += Int64(slice.count)
        pending.removeAll()
        return makeChunk(slice, startSample: startSample)
    }

    private func makeChunk(_ samples: [Int16], startSample: Int64) -> PCMChunk {
        let rate = Double(sampleRate)
        return PCMChunk(
            samples: samples,
            startMs: Double(startSample) * 1000.0 / rate,
            endMs: Double(startSample + Int64(samples.count)) * 1000.0 / rate)
    }
}
