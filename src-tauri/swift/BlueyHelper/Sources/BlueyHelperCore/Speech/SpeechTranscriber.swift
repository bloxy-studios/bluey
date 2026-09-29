import AVFoundation
import Foundation
import Speech

/// One SFSpeechRecognizer + SFSpeechAudioBufferRecognitionRequest per audio
/// source ("microphone" / "system").
///
/// SFSpeech buffer requests are limited to ~1 minute, so the request is
/// restarted every ~55 s of appended audio and after every final result;
/// `epochMs` bookkeeping keeps startMs/endMs relative to `audio.start`.
/// Utterance boundaries and ids come from `UtteranceTracker`, which also
/// keeps a retired request's callbacks from rotating the live one.
/// https://developer.apple.com/documentation/speech/sfspeechaudiobufferrecognitionrequest
///
/// NOTE (macOS 26+): the newer SpeechAnalyzer API removes the 1-minute limit;
/// it could be adopted behind `#available(macOS 26, *)`. SFSpeechRecognizer
/// remains the default path for macOS 14/15.
public final class SpeechTranscriber {
    public typealias Emit = (_ event: String, _ data: AnyEncodable) -> Void

    public struct TranscriptEvent: Encodable {
        public let source: String
        public let text: String
        public let startMs: Int
        public let endMs: Int
        public let confidence: Double?
        public let locale: String
        /// `<request generation>-<counter>`, shared by an utterance's
        /// partials and its final.
        public let utteranceId: String
    }

    /// What `start()` settled on (reported in `audio.started`): the
    /// recognizer's locale and whether audio stays on the Mac.
    public struct Route: Encodable, Equatable {
        public let locale: String
        public let onDevice: Bool
    }

    struct SpeechErrorEvent: Encodable {
        let code: String
        let message: String
        let kind: String
    }

    /// Restart the request after this much appended audio (SFSpeech ~1 min cap).
    private static let maxRequestSeconds: Double = 55

    static let fallbackLocale = "en-US"

    /// The locale for `language: auto`: the user's own when Apple Speech
    /// supports it, else en-US.
    static func defaultLocale(
        current: Locale = .current,
        supported: Set<Locale> = SFSpeechRecognizer.supportedLocales()
    ) -> Locale {
        let wanted = speechIdentifier(current)
        return supported.first { speechIdentifier($0) == wanted }
            ?? Locale(identifier: fallbackLocale)
    }

    /// "en-GB" for en_GB, en-GB@rg=… and en-GB alike.
    private static func speechIdentifier(_ locale: Locale) -> String {
        let language = locale.language.languageCode?.identifier ?? ""
        // The dialect's region (en_US@rg=gbzzzz speaks en-US), else the locale's.
        guard let region = (locale.language.region ?? locale.region)?.identifier else {
            return language
        }
        return "\(language)-\(region)"
    }

    private let source: String
    private let localeId: String
    private let onDevice: Bool
    private let sampleRate: Double
    private let emit: Emit
    private let queue: DispatchQueue

    private var recognizer: SFSpeechRecognizer?
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var bufferFormat: AVAudioFormat?

    /// Total frames appended since audio.start / frames at current request start.
    private var totalFrames: Int64 = 0
    private var epochFrames: Int64 = 0
    private var stopped = false
    private var unavailableReported = false
    private var utterances = UtteranceTracker()
    /// Set by `start()`; read after it returns.
    public private(set) var route: Route?
    /// Non-routine task errors in a row (reset by any transcript); past
    /// `maxConsecutiveErrors` the recognizer is reported unavailable instead
    /// of being restarted in a hot loop.
    private var consecutiveErrors = 0
    private static let maxConsecutiveErrors = 5

    public init(
        source: String, locale: String, onDevice: Bool, sampleRate: Double = 16000,
        emit: @escaping Emit
    ) {
        self.source = source
        self.localeId = locale
        self.onDevice = onDevice
        self.sampleRate = sampleRate
        self.emit = emit
        self.queue = DispatchQueue(label: "com.codewithabdul.bluey.helper.speech.\(source)")
    }

    // MARK: lifecycle

    /// Returns false when the recognizer cannot run (also emits audio.error).
    public func start() -> Bool {
        var ok = false
        queue.sync {
            guard SFSpeechRecognizer.authorizationStatus() == .authorized else {
                reportUnavailable("speech recognition not authorized")
                return
            }
            // https://developer.apple.com/documentation/speech/sfspeechrecognizer/init(locale:)
            guard let recognizer = SFSpeechRecognizer(locale: Locale(identifier: localeId)),
                recognizer.isAvailable
            else {
                reportUnavailable("speech recognizer unavailable for locale \(localeId)")
                return
            }
            self.recognizer = recognizer
            self.bufferFormat = AVAudioFormat(
                commonFormat: .pcmFormatFloat32, sampleRate: sampleRate, channels: 1,
                interleaved: false)
            self.stopped = false
            self.startRequestLocked()
            if let request = self.request {
                // Without on-device assets for the locale, SFSpeech uses
                // Apple's servers: say so rather than claim on-device.
                self.route = Route(
                    locale: localeId, onDevice: request.requiresOnDeviceRecognition)
            }
            ok = self.request != nil
        }
        return ok
    }

    public func finish() {
        queue.async { [weak self] in
            guard let self else { return }
            self.stopped = true
            self.request?.endAudio()
            self.request = nil
            // Give the final callback a moment, then cancel outright.
            self.queue.asyncAfter(deadline: .now() + 1.5) { [weak self] in
                self?.task?.cancel()
                self?.task = nil
            }
        }
    }

    // MARK: audio input

    public func append(samples: [Int16]) {
        guard !samples.isEmpty else { return }
        queue.async { [weak self] in
            guard let self, !self.stopped else { return }
            guard let request = self.request, let format = self.bufferFormat else { return }
            guard
                let buffer = AVAudioPCMBuffer(
                    pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count))
            else { return }
            buffer.frameLength = AVAudioFrameCount(samples.count)
            if let channel = buffer.floatChannelData?[0] {
                for (i, s) in samples.enumerated() {
                    channel[i] = Float(s) / 32768.0
                }
            }
            request.append(buffer)
            self.totalFrames += Int64(samples.count)

            // Rotate the request before hitting the ~1 min SFSpeech ceiling.
            let requestSeconds = Double(self.totalFrames - self.epochFrames) / self.sampleRate
            if requestSeconds >= Self.maxRequestSeconds {
                self.rotateRequestLocked("cap")
            }
        }
    }

    // MARK: request management (on `queue`)

    private func startRequestLocked() {
        guard let recognizer else { return }
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        // https://developer.apple.com/documentation/speech/sfspeechrecognitionrequest/requiresondevicerecognition
        request.requiresOnDeviceRecognition = onDevice && recognizer.supportsOnDeviceRecognition
        // Question detection keys off "?" (macOS 13+; off by default).
        // https://developer.apple.com/documentation/speech/sfspeechrecognitionrequest/addspunctuation
        request.addsPunctuation = true
        self.request = request
        self.epochFrames = totalFrames
        let epochMs = Double(epochFrames) * 1000.0 / sampleRate
        let generation = utterances.beginRequest()

        // https://developer.apple.com/documentation/speech/sfspeechrecognizer/recognitiontask(with:resulthandler:)
        self.task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            guard let self else { return }
            self.queue.async {
                self.handleLocked(
                    result: result, error: error, epochMs: epochMs, generation: generation)
            }
        }
    }

    private func rotateRequestLocked(_ reason: String) {
        Log.shared.debug("speech(\(source)) rotating request \(utterances.generation): \(reason)")
        request?.endAudio()
        request = nil
        startRequestLocked()
    }

    private func handleLocked(
        result: SFSpeechRecognitionResult?, error: Error?, epochMs: Double, generation: Int
    ) {
        if let result {
            for event in utterances.observe(observation(result, epochMs, generation)) {
                consecutiveErrors = 0
                emit(
                    event.isFinal ? "transcript.final" : "transcript.partial",
                    AnyEncodable(
                        TranscriptEvent(
                            source: source, text: event.text, startMs: event.startMs,
                            endMs: event.endMs, confidence: event.confidence, locale: localeId,
                            utteranceId: event.utteranceId)))
            }
        }
        // Only the live request may rotate or restart: a retired task's final
        // or error arrives after its successor started (MAC-003).
        guard !stopped, utterances.isCurrent(generation) else { return }
        if let result, result.isFinal {
            rotateRequestLocked("final")
            return
        }
        guard let error else { return }
        let ns = error as NSError
        // kAFAssistantErrorDomain 1110 ("no speech detected") and 216/301
        // (request cancelled/retired) are routine — restart quietly.
        let routine =
            ["kAFAssistantErrorDomain", "kLSRErrorDomain"].contains(ns.domain)
            && [1110, 1101, 216, 203, 301].contains(ns.code)
        if routine {
            rotateRequestLocked("routine \(ns.code)")
        } else if recognizer?.isAvailable == false {
            reportUnavailable("speech recognizer became unavailable")
        } else {
            consecutiveErrors += 1
            Log.shared.warn("speech(\(source)) task error: \(ns.domain) \(ns.code)")
            guard consecutiveErrors < Self.maxConsecutiveErrors else {
                reportUnavailable("speech recognition keeps failing (\(ns.domain) \(ns.code))")
                return
            }
            rotateRequestLocked("error")
        }
    }

    /// One callback, with times relative to `audio.start`.
    private func observation(
        _ result: SFSpeechRecognitionResult, _ epochMs: Double, _ generation: Int
    ) -> UtteranceTracker.Result {
        let transcription = result.bestTranscription
        let segments = transcription.segments
        let (startMs, endMs) = UtteranceTracker.times(
            segments: segments.map { (timestamp: $0.timestamp, duration: $0.duration) },
            epochMs: epochMs, nowMs: Double(totalFrames) * 1000.0 / sampleRate)
        var confidence: Double?
        if result.isFinal, !segments.isEmpty {
            confidence = segments.reduce(0.0) { $0 + Double($1.confidence) } / Double(segments.count)
        }
        return UtteranceTracker.Result(
            generation: generation, text: transcription.formattedString, isFinal: result.isFinal,
            // Set when the recognizer closes a stretch of speech (a pause).
            hasMetadata: result.speechRecognitionMetadata != nil,
            startMs: startMs, endMs: endMs, confidence: confidence)
    }

    private func reportUnavailable(_ message: String) {
        guard !unavailableReported else { return }
        unavailableReported = true
        emit(
            "audio.error",
            AnyEncodable(
                SpeechErrorEvent(code: "speech_unavailable", message: message, kind: "audio")))
    }
}
