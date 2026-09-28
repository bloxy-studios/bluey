import XCTest

@testable import BlueyHelperCore

final class MicrophoneCaptureTests: XCTestCase {
    /// MAC-005: a rebuild that fails after a device change leaves the
    /// microphone wanted, so the next device change brings it back, and the
    /// host hears about both transitions.
    func testAFailedRebuildIsRetriedOnTheNextDeviceChange() {
        let mic = MicrophoneCapture(deviceService: AudioDeviceService())
        var attempts: [HelperError?] = [nil, .audio("engine_start_failed", "busy"), nil]
        mic.startEngineOverride = { attempts.removeFirst() }
        var changes: [Bool] = []
        var errors: [String] = []
        let recovered = expectation(description: "microphone back")
        mic.onError = { errors.append($0.code) }
        mic.onLiveChange = { live in
            changes.append(live)
            if live { recovered.fulfill() }
        }

        let started = expectation(description: "started")
        mic.start(deviceUID: nil) { error in
            XCTAssertNil(error)
            started.fulfill()
        }
        wait(for: [started], timeout: 2)
        mic.restart()  // AVAudioEngineConfigurationChange: the rebuild fails
        mic.retryIfDegraded()  // audio.deviceChanged: the retry succeeds
        wait(for: [recovered], timeout: 2)

        XCTAssertEqual(changes, [false, true])
        XCTAssertEqual(errors, ["engine_start_failed"])
        XCTAssertTrue(attempts.isEmpty)
    }

    func testALiveMicrophoneIsNotRebuiltOnAnUnrelatedDeviceChange() {
        let mic = MicrophoneCapture(deviceService: AudioDeviceService())
        var builds = 0
        mic.startEngineOverride = {
            builds += 1
            return nil
        }
        let started = expectation(description: "started")
        mic.start(deviceUID: nil) { _ in started.fulfill() }
        wait(for: [started], timeout: 2)

        mic.retryIfDegraded()
        let settled = expectation(description: "queue drained")
        mic.start(deviceUID: nil) { _ in settled.fulfill() }  // already running: no-op
        wait(for: [settled], timeout: 2)
        XCTAssertEqual(builds, 1)
    }
}
