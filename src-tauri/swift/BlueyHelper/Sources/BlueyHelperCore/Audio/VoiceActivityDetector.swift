import Foundation

/// Energy (RMS) voice activity detector with an adaptive noise floor and a
/// hangover window. Pure Swift, deterministic, unit-tested.
///
/// Sensitivities map to (multiplier over noise floor, absolute RMS minimum):
///   low    → speech must be clearly louder than the floor
///   medium → default
///   high   → trips on quiet speech
public final class VoiceActivityDetector {
    public enum Sensitivity: String, Codable {
        case low, medium, high

        var floorMultiplier: Double {
            switch self {
            case .low: return 4.0
            case .medium: return 2.8
            case .high: return 1.9
            }
        }

        /// Never trigger below this RMS (guards against a near-zero floor).
        var minimumThreshold: Double {
            switch self {
            case .low: return 0.020
            case .medium: return 0.012
            case .high: return 0.006
            }
        }
    }

    private let sensitivity: Sensitivity
    private let hangoverMs: Double

    /// Exponential noise-floor tracker; only updated by non-speech frames.
    private var noiseFloor: Double
    private var inSpeech = false
    private var lastActiveMs: Double = -.greatestFiniteMagnitude

    public init(sensitivity: Sensitivity, hangoverMs: Double = 300, initialNoiseFloor: Double = 0.01) {
        self.sensitivity = sensitivity
        self.hangoverMs = hangoverMs
        self.noiseFloor = initialNoiseFloor
    }

    public var currentThreshold: Double {
        max(sensitivity.minimumThreshold, noiseFloor * sensitivity.floorMultiplier)
    }

    /// Feed one frame's RMS (0…1) with its position on the session clock (ms).
    /// Returns whether the frame counts as speech (including hangover frames).
    public func process(rms: Double, atMs: Double) -> Bool {
        let active = rms >= currentThreshold

        if active {
            inSpeech = true
            lastActiveMs = atMs
            // Creep the floor upward very slowly during activity so sustained
            // constant noise (fan hum, HVAC) is eventually absorbed into the
            // floor; bursty real speech barely moves it.
            noiseFloor = 0.995 * noiseFloor + 0.005 * rms
            noiseFloor = min(max(noiseFloor, 0.0005), 0.5)
        } else {
            // Hangover: bridge short pauses so words are not chopped.
            if inSpeech, atMs - lastActiveMs > hangoverMs {
                inSpeech = false
            }
            // Adapt the floor on non-active frames only, quickly downward and
            // slowly upward so a loud period cannot poison the floor.
            if rms < noiseFloor {
                noiseFloor = 0.8 * noiseFloor + 0.2 * rms
            } else if !inSpeech {
                noiseFloor = 0.95 * noiseFloor + 0.05 * rms
            }
            noiseFloor = min(max(noiseFloor, 0.0005), 0.5)
        }

        return active || inSpeech
    }

    /// RMS of 16-bit samples normalized to 0…1.
    public static func rms(of samples: [Int16]) -> Double {
        guard !samples.isEmpty else { return 0 }
        var sum: Double = 0
        for s in samples {
            let v = Double(s) / 32768.0
            sum += v * v
        }
        return (sum / Double(samples.count)).squareRoot()
    }
}
