import Foundation

/// Turns one source's SFSpeech result callbacks into transcript events with a
/// stable `utteranceId` per utterance. Pure (no Speech framework), so the
/// rules below are unit-tested without a recognizer.
///
/// - Every recognition request is a *generation*. Callbacks from a retired
///   request (rotated at the ~1 min cap or after a final) may only deliver
///   that request's final; they never rotate or restart the live one
///   (MAC-003) and their late partials are dropped.
/// - On-device recognition often *resets* after a pause instead of sending
///   `isFinal`: the next partial starts over with new words. The previous
///   partial is then committed as the utterance's final (MAC-002). After a
///   pause (the previous result carried speech metadata) anything that does
///   not continue the previous words is a reset, so a recognizer that keeps
///   the words across the pause never duplicates text. Without that signal
///   only a sharp drop in the word count counts as one.
/// - Partials often carry all-zero segment timing; they are stamped with the
///   audio position instead, and every event of an utterance keeps the start
///   of its first partial, so a committed final spans its own speech.
struct UtteranceTracker {
    /// One recognizer callback, reduced to what the rules need.
    struct Result {
        var generation: Int
        var text: String
        var isFinal: Bool
        var hasMetadata = false
        var startMs = 0
        var endMs = 0
        var confidence: Double?
    }

    /// A transcript event to send (`transcript.final` when `isFinal`).
    struct Emission: Equatable {
        let isFinal: Bool
        let utteranceId: String
        let text: String
        let startMs: Int
        let endMs: Int
        let confidence: Double?
    }

    private struct Open {
        let id: String
        /// When the utterance's first event began.
        let startMs: Int
        var last: Emission?
        var hadMetadata = false
    }

    /// Retired generations whose final may still arrive, kept this long.
    private static let retiredKept = 2

    private(set) var generation = 0
    private var counter = 0
    private var open: Open?
    /// Retired generation → the utterance id its final belongs to.
    private var retired: [Int: String] = [:]

    /// A new request starts; returns its generation.
    mutating func beginRequest() -> Int {
        if let open, open.last != nil { retired[generation] = open.id }
        open = nil
        generation += 1
        retired = retired.filter { $0.key >= generation - Self.retiredKept }
        return generation
    }

    /// Only the live request's callbacks may rotate or restart it.
    func isCurrent(_ generation: Int) -> Bool { generation == self.generation }

    /// The events one callback produces (in order).
    mutating func observe(_ result: Result) -> [Emission] {
        guard !result.text.isEmpty else { return [] }
        guard isCurrent(result.generation) else {
            // A retired request: only its final counts, under the id its
            // partials used.
            guard result.isFinal else { return [] }
            let id = retired.removeValue(forKey: result.generation) ?? nextId()
            return [emission(result, id: id)]
        }
        var out: [Emission] = []
        if let current = open, let last = current.last,
            Self.isReset(from: last.text, hadMetadata: current.hadMetadata, to: result.text)
        {
            out.append(last.committed)
            open = nil
        }
        let id = open?.id ?? nextId()
        let startMs = min(open?.startMs ?? result.startMs, result.startMs)
        let event = emission(result, id: id, startMs: startMs)
        out.append(event)
        if result.isFinal {
            open = nil
        } else {
            open = Open(id: id, startMs: startMs, last: event, hadMetadata: result.hasMetadata)
        }
        return out
    }

    private mutating func nextId() -> String {
        counter += 1
        return "\(generation)-\(counter)"
    }

    private func emission(_ result: Result, id: String, startMs: Int? = nil) -> Emission {
        Emission(
            isFinal: result.isFinal, utteranceId: id, text: result.text,
            startMs: startMs ?? result.startMs, endMs: result.endMs, confidence: result.confidence)
    }

    /// The recognizer started a new utterance instead of revising this one.
    static func isReset(from previous: String, hadMetadata: Bool, to next: String) -> Bool {
        let before = words(previous)
        let after = words(next)
        guard !before.isEmpty, !after.isEmpty else { return false }
        if hadMetadata { return !after.starts(with: before) }
        return before.count >= 4 && after.count * 2 < before.count
    }

    /// One callback's times relative to `audio.start`. Segment timestamps are
    /// seconds within the request that began at `epochMs`
    /// (https://developer.apple.com/documentation/speech/sftranscriptionsegment);
    /// without real timing the result is stamped with the audio position.
    static func times(
        segments: [(timestamp: Double, duration: Double)], epochMs: Double, nowMs: Double
    ) -> (startMs: Int, endMs: Int) {
        guard let first = segments.first, let last = segments.last,
            last.timestamp + last.duration > 0
        else { return (Int(nowMs.rounded()), Int(nowMs.rounded())) }
        let startMs = epochMs + first.timestamp * 1000.0
        let endMs = epochMs + (last.timestamp + last.duration) * 1000.0
        return (Int(startMs.rounded()), Int(endMs.rounded()))
    }

    /// Lower-cased words without punctuation (partials rewrite both).
    private static func words(_ text: String) -> [String] {
        text.lowercased()
            .components(separatedBy: CharacterSet.alphanumerics.inverted)
            .filter { !$0.isEmpty }
    }
}

extension UtteranceTracker.Emission {
    /// This partial, sent as its utterance's final.
    fileprivate var committed: Self {
        Self(
            isFinal: true, utteranceId: utteranceId, text: text, startMs: startMs,
            endMs: endMs, confidence: confidence)
    }
}
