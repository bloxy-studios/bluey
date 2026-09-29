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
            current: Locale(identifier: "en_GB"), supported: supported,
            recognizesOnDevice: { _ in true })
        XCTAssertEqual(british.identifier, "en-GB")
    }

    /// `auto` never moves audio off the Mac: without an on-device model for
    /// the user's locale it keeps en-US, unless on-device was not asked for.
    func testDefaultLocaleStaysOnDeviceWhenTheUsersLocaleCannot() {
        let supported: Set<Locale> = [Locale(identifier: "en-US"), Locale(identifier: "nl-NL")]
        let english: (Locale) -> Bool = { $0.identifier == "en-US" }
        let dutch = Locale(identifier: "nl_NL")
        XCTAssertEqual(
            SpeechTranscriber.defaultLocale(
                current: dutch, supported: supported, recognizesOnDevice: english
            ).identifier, "en-US")
        XCTAssertEqual(
            SpeechTranscriber.defaultLocale(
                onDevice: false, current: dutch, supported: supported, recognizesOnDevice: english
            ).identifier, "nl-NL")
    }

    func testDefaultLocaleFallsBackToEnglishUS() {
        let supported: Set<Locale> = [Locale(identifier: "en-US")]
        let klingon = SpeechTranscriber.defaultLocale(
            current: Locale(identifier: "tlh_001"), supported: supported,
            recognizesOnDevice: { _ in true })
        XCTAssertEqual(klingon.identifier, "en-US")
    }
}
