import Foundation
import Speech

/// A capability probed once, off the request path, and read without blocking.
///
/// `helper.version` is the handshake the host times out on, and building an
/// `SFSpeechRecognizer` can take seconds on a cold boot (an XPC round trip to
/// speechd) — so the probe runs on a background queue at startup and the
/// version answer reports whatever is known by then.
final class CachedCapability {
    private let lock = NSLock()
    private var value: Bool?

    init(queue: DispatchQueue = .global(qos: .utility), probe: @escaping () -> Bool) {
        queue.async { [weak self] in
            let available = probe()
            self?.store(available)
        }
    }

    /// `nil` until the probe has finished.
    var current: Bool? {
        lock.lock()
        defer { lock.unlock() }
        return value
    }

    private func store(_ available: Bool) {
        lock.lock()
        value = available
        lock.unlock()
    }
}

extension CachedCapability {
    /// Whether Apple Speech can recognize the default locale on-device.
    static func speechOnDevice() -> CachedCapability {
        CachedCapability {
            // https://developer.apple.com/documentation/speech/sfspeechrecognizer/supportsondevicerecognition
            SFSpeechRecognizer(locale: SpeechTranscriber.defaultLocale())?.supportsOnDeviceRecognition
                == true
        }
    }
}
