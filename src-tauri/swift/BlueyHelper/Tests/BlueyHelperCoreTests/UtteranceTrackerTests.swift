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

    func testUtteranceIdsNameTheirGeneration() {
        var tracker = UtteranceTracker()
        _ = tracker.beginRequest()
        let g = tracker.beginRequest()
        let out = tracker.observe(R(generation: g, text: "hi", isFinal: false))
        XCTAssertEqual(out.first?.utteranceId, "2-1")
    }
}
