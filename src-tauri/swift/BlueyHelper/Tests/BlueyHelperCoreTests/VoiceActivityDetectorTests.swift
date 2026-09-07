import XCTest

@testable import BlueyHelperCore

final class VoiceActivityDetectorTests: XCTestCase {
    func testSilenceIsNotSpeech() {
        let vad = VoiceActivityDetector(sensitivity: .medium)
        for i in 0..<20 {
            XCTAssertFalse(vad.process(rms: 0.002, atMs: Double(i) * 200))
        }
    }

    func testLoudFrameIsSpeech() {
        let vad = VoiceActivityDetector(sensitivity: .medium)
        _ = vad.process(rms: 0.002, atMs: 0)
        XCTAssertTrue(vad.process(rms: 0.3, atMs: 200))
    }

    func testHangoverBridgesShortPauses() {
        let vad = VoiceActivityDetector(sensitivity: .medium, hangoverMs: 300)
        XCTAssertTrue(vad.process(rms: 0.3, atMs: 0))
        // Quiet frames inside the 300 ms hangover stay "speech".
        XCTAssertTrue(vad.process(rms: 0.001, atMs: 100))
        XCTAssertTrue(vad.process(rms: 0.001, atMs: 250))
        // Past the hangover → silence again.
        XCTAssertFalse(vad.process(rms: 0.001, atMs: 400))
        XCTAssertFalse(vad.process(rms: 0.001, atMs: 600))
    }

    func testSpeechExtendsHangover() {
        let vad = VoiceActivityDetector(sensitivity: .medium, hangoverMs: 300)
        XCTAssertTrue(vad.process(rms: 0.3, atMs: 0))
        XCTAssertTrue(vad.process(rms: 0.001, atMs: 200)) // hangover
        XCTAssertTrue(vad.process(rms: 0.3, atMs: 250)) // speech again → resets hangover clock
        XCTAssertTrue(vad.process(rms: 0.001, atMs: 500)) // 250 ms after last speech → still hangover
        XCTAssertFalse(vad.process(rms: 0.001, atMs: 600)) // 350 ms after → silence
    }

    func testNoiseFloorAdaptsUpward() {
        let vad = VoiceActivityDetector(sensitivity: .medium, initialNoiseFloor: 0.001)
        // A constant office hum at 0.03 should eventually stop counting as speech
        // (medium: threshold = max(0.012, floor * 2.8)).
        var lastResult = true
        for i in 0..<200 {
            lastResult = vad.process(rms: 0.03, atMs: Double(i) * 200)
        }
        XCTAssertFalse(lastResult, "sustained constant noise must be absorbed into the floor")
        // Real speech well above the hum still trips it.
        XCTAssertTrue(vad.process(rms: 0.4, atMs: 100_000))
    }

    func testSensitivityOrdering() {
        // The same marginal RMS should trip high before medium before low.
        let quiet = 0.010
        let low = VoiceActivityDetector(sensitivity: .low, initialNoiseFloor: 0.001)
        let medium = VoiceActivityDetector(sensitivity: .medium, initialNoiseFloor: 0.001)
        let high = VoiceActivityDetector(sensitivity: .high, initialNoiseFloor: 0.001)
        XCTAssertFalse(low.process(rms: quiet, atMs: 0))
        XCTAssertFalse(medium.process(rms: quiet, atMs: 0))
        XCTAssertTrue(high.process(rms: quiet, atMs: 0))
    }

    func testRmsOfKnownSignal() {
        // Full-scale square wave → RMS 1.0.
        let fullScale = [Int16](repeating: Int16.min, count: 100)
        XCTAssertEqual(VoiceActivityDetector.rms(of: fullScale), 1.0, accuracy: 1e-9)
        // Silence → 0.
        XCTAssertEqual(VoiceActivityDetector.rms(of: [Int16](repeating: 0, count: 100)), 0.0)
        // Half-scale square wave → 0.5.
        let half = [Int16](repeating: -16384, count: 100)
        XCTAssertEqual(VoiceActivityDetector.rms(of: half), 0.5, accuracy: 1e-9)
        XCTAssertEqual(VoiceActivityDetector.rms(of: []), 0.0)
    }
}
