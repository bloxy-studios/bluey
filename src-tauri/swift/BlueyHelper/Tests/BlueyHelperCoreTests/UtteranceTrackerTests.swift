import XCTest

@testable import BlueyHelperCore

final class UtteranceTrackerTests: XCTestCase {
    private typealias R = UtteranceTracker.Result

    private func texts(_ emissions: [UtteranceTracker.Emission]) -> [String] {
        emissions.map { "\($0.isFinal ? "final" : "partial"):\($0.text)" }
    }

    /// MAC-002: the on-device recognizer resets after a pause without isFinal.
    func testAResetAfterAPauseCommitsThePreviousUtterance() {
        var tracker = UtteranceTracker()
        let g = tracker.beginRequest()
        _ = tracker.observe(R(generation: g, text: "a b", isFinal: false))
        _ = tracker.observe(R(generation: g, text: "a b", isFinal: false, hasMetadata: true))
        let out = tracker.observe(R(generation: g, text: "c", isFinal: false))

        XCTAssertEqual(texts(out), ["final:a b", "partial:c"])
        XCTAssertNotEqual(out[0].utteranceId, out[1].utteranceId)
    }

    func testARecognizerThatKeepsTheWordsNeverDuplicatesThem() {
        var tracker = UtteranceTracker()
        let g = tracker.beginRequest()
        _ = tracker.observe(R(generation: g, text: "a b", isFinal: false, hasMetadata: true))
        let more = tracker.observe(R(generation: g, text: "A b, c", isFinal: false))
        let done = tracker.observe(R(generation: g, text: "A b, c.", isFinal: true))

        XCTAssertEqual(texts(more + done), ["partial:A b, c", "final:A b, c."])
        XCTAssertEqual(more[0].utteranceId, done[0].utteranceId)
    }

    func testARevisionOfTheOpeningWordIsNotAReset() {
        var tracker = UtteranceTracker()
        let g = tracker.beginRequest()
        _ = tracker.observe(R(generation: g, text: "the cat sat", isFinal: false))
        let out = tracker.observe(R(generation: g, text: "a cat sat", isFinal: false))
        XCTAssertEqual(texts(out), ["partial:a cat sat"])
    }

    func testTheEndAudioFinalAfterAResetCarriesOnlyTheLastUtterance() {
        var tracker = UtteranceTracker()
        let g = tracker.beginRequest()
        _ = tracker.observe(R(generation: g, text: "one two", isFinal: false, hasMetadata: true))
        let reset = tracker.observe(R(generation: g, text: "three", isFinal: false))
        let done = tracker.observe(R(generation: g, text: "three four", isFinal: true))

        XCTAssertEqual(texts(done), ["final:three four"])
        XCTAssertEqual(reset[1].utteranceId, done[0].utteranceId)
    }

    /// MAC-003: after the 55 s rotation the retired request's final still
    /// lands (under its partials' id) but is not the live generation, so it
    /// cannot rotate the new request.
    func testARetiredRequestsFinalNeverCountsAsTheLiveRequest() {
        var tracker = UtteranceTracker()
        let first = tracker.beginRequest()
        let partial = tracker.observe(R(generation: first, text: "long talk", isFinal: false))
        let second = tracker.beginRequest()

        XCTAssertFalse(tracker.isCurrent(first))
        XCTAssertTrue(tracker.isCurrent(second))
        XCTAssertEqual(
            tracker.observe(R(generation: first, text: "long talk ends", isFinal: false)), [],
            "late partials of a retired request are dropped")
        let late = tracker.observe(R(generation: first, text: "long talk ends.", isFinal: true))
        XCTAssertEqual(texts(late), ["final:long talk ends."])
        XCTAssertEqual(late[0].utteranceId, partial[0].utteranceId)

        let live = tracker.observe(R(generation: second, text: "next", isFinal: false))
        XCTAssertNotEqual(live[0].utteranceId, late[0].utteranceId)
    }

    /// A short utterance followed by one with the same opening word is still
    /// committed when the recognizer marked a pause.
    func testAShortUtteranceIsCommittedBeforeOneWithTheSameOpeningWord() {
        var tracker = UtteranceTracker()
        let g = tracker.beginRequest()
        _ = tracker.observe(R(generation: g, text: "I see.", isFinal: false, hasMetadata: true))
        let out = tracker.observe(R(generation: g, text: "I", isFinal: false))

        XCTAssertEqual(texts(out), ["final:I see.", "partial:I"])
        XCTAssertNotEqual(out[0].utteranceId, out[1].utteranceId)
        XCTAssertTrue(UtteranceTracker.isReset(from: "I see.", hadMetadata: true, to: "I think so"))
        XCTAssertFalse(UtteranceTracker.isReset(from: "a b", hadMetadata: true, to: "A b, c"))
        XCTAssertFalse(UtteranceTracker.isReset(from: "one two", hadMetadata: false, to: "three"))
        XCTAssertTrue(UtteranceTracker.isReset(from: "a b c d", hadMetadata: false, to: "a"))
    }

    /// Partials often carry all-zero segment timing: they are stamped with the
    /// audio position, and a committed final spans its own partials.
    func testACommittedFinalCarriesItsOwnAudioTimes() {
        var tracker = UtteranceTracker()
        let g = tracker.beginRequest()
        _ = tracker.observe(
            R(generation: g, text: "a b", isFinal: false, startMs: 1_000, endMs: 1_000))
        _ = tracker.observe(
            R(
                generation: g, text: "a b", isFinal: false, hasMetadata: true,
                startMs: 2_500, endMs: 2_500))
        let out = tracker.observe(
            R(generation: g, text: "c", isFinal: false, startMs: 30_000, endMs: 30_000))

        XCTAssertEqual(texts(out), ["final:a b", "partial:c"])
        XCTAssertEqual([out[0].startMs, out[0].endMs], [1_000, 2_500])
        XCTAssertEqual([out[1].startMs, out[1].endMs], [30_000, 30_000])
    }

    func testSegmentTimesAreRelativeToTheRequestUnlessAllZero() {
        let untimed = UtteranceTracker.times(
            segments: [(timestamp: 0, duration: 0), (timestamp: 0, duration: 0)],
            epochMs: 55_000, nowMs: 80_250)
        XCTAssertEqual([untimed.startMs, untimed.endMs], [80_250, 80_250])
        let timed = UtteranceTracker.times(
            segments: [(timestamp: 1.5, duration: 0.5), (timestamp: 2, duration: 1)],
            epochMs: 55_000, nowMs: 80_250)
        XCTAssertEqual([timed.startMs, timed.endMs], [56_500, 58_000])
        let empty = UtteranceTracker.times(segments: [], epochMs: 55_000, nowMs: 80_250)
        XCTAssertEqual([empty.startMs, empty.endMs], [80_250, 80_250])
    }

    func testUtteranceIdsNameTheirGeneration() {
        var tracker = UtteranceTracker()
        _ = tracker.beginRequest()
        let g = tracker.beginRequest()
        let out = tracker.observe(R(generation: g, text: "hi", isFinal: false))
        XCTAssertEqual(out.first?.utteranceId, "2-1")
    }
}
