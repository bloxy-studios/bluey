import Foundation

/// Tracks smoothed input levels per source and periodically emits
/// `audio.level` → `{ "microphone": 0..1, "system": 0..1 }`.
public final class LevelMeter {
    public typealias Emit = (_ event: String, _ data: AnyEncodable) -> Void

    private struct LevelEvent: Encodable {
        let microphone: Double
        let system: Double
    }

    private let queue = DispatchQueue(label: "com.codewithabdul.bluey.helper.levels")
    private var timer: DispatchSourceTimer?
    private var microphone: Double = 0
    private var system: Double = 0
    private let emit: Emit

    public init(emit: @escaping Emit) {
        self.emit = emit
    }

    /// Fast attack, slow release: the meter jumps up instantly and decays ~30 %
    /// per update, which reads naturally in a UI meter.
    public func update(source: String, rms: Double) {
        let clamped = min(max(rms, 0), 1)
        queue.async { [weak self] in
            guard let self else { return }
            switch source {
            case "microphone": self.microphone = max(clamped, self.microphone * 0.7)
            case "system": self.system = max(clamped, self.system * 0.7)
            default: break
            }
        }
    }

    public func start(intervalMs: Int) {
        queue.async { [weak self] in
            guard let self, self.timer == nil else { return }
            let timer = DispatchSource.makeTimerSource(queue: self.queue)
            timer.schedule(
                deadline: .now() + .milliseconds(intervalMs),
                repeating: .milliseconds(max(16, intervalMs)))
            timer.setEventHandler { [weak self] in
                guard let self else { return }
                self.emit(
                    "audio.level",
                    AnyEncodable(
                        LevelEvent(
                            microphone: (self.microphone * 1000).rounded() / 1000,
                            system: (self.system * 1000).rounded() / 1000)))
                // Decay between updates so silence falls back to 0.
                self.microphone *= 0.7
                self.system *= 0.7
                if self.microphone < 0.001 { self.microphone = 0 }
                if self.system < 0.001 { self.system = 0 }
            }
            timer.resume()
            self.timer = timer
        }
    }

    public func stop() {
        queue.async { [weak self] in
            self?.timer?.cancel()
            self?.timer = nil
            self?.microphone = 0
            self?.system = 0
        }
    }
}
