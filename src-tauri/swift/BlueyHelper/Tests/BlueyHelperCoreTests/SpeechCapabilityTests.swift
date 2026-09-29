import XCTest

@testable import BlueyHelperCore

final class SpeechCapabilityTests: XCTestCase {
    /// MAC-011: a slow probe (speechd on a cold boot) never blocks the reader.
    func testASlowProbeNeverBlocksTheHandshake() {
        let release = DispatchSemaphore(value: 0)
        let probed = expectation(description: "probe finished")
        let capability = CachedCapability {
            release.wait()
            probed.fulfill()
            return true
        }

        let started = Date()
        XCTAssertNil(capability.current, "unknown while the probe runs")
        XCTAssertLessThan(Date().timeIntervalSince(started), 0.05)

        release.signal()
        wait(for: [probed], timeout: 2)
        let settled = expectation(description: "value stored")
        DispatchQueue.global().async {
            while capability.current == nil { usleep(1000) }
            settled.fulfill()
        }
        wait(for: [settled], timeout: 2)
        XCTAssertEqual(capability.current, true)
    }

    func testDefaultLocaleIsTheUsersWhenSpeechSupportsIt() {
        let supported: Set<Locale> = [Locale(identifier: "en-US"), Locale(identifier: "en-GB")]
        let british = SpeechTranscriber.defaultLocale(
            current: Locale(identifier: "en_GB"), supported: supported)
        XCTAssertEqual(british.identifier, "en-GB")
    }

    func testDefaultLocaleFallsBackToEnglishUS() {
        let supported: Set<Locale> = [Locale(identifier: "en-US")]
        let klingon = SpeechTranscriber.defaultLocale(
            current: Locale(identifier: "tlh_001"), supported: supported)
        XCTAssertEqual(klingon.identifier, "en-US")
    }
}
