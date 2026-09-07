import AVFoundation
import Foundation
import Speech

/// One SFSpeechRecognizer + SFSpeechAudioBufferRecognitionRequest per audio
/// source ("microphone" / "system").
///
/// SFSpeech buffer requests are limited to ~1 minute, so the request is
/// restarted every ~55 s of appended audio and after every final result;
/// `epochMs` bookkeeping keeps startMs/endMs relative to `audio.start`.
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
    }

    struct SpeechErrorEvent: Encodable {
        let code: String
        let message: String
        let kind: String
    }

    /// Restart the request after this much appended audio (SFSpeech ~1 min cap).
    private static let maxRequestSeconds: Double = 55

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
                self.rotateRequestLocked()
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
        self.request = request
        self.epochFrames = totalFrames
        let epochMs = Double(epochFrames) * 1000.0 / sampleRate

        // https://developer.apple.com/documentation/speech/sfspeechrecognizer/recognitiontask(with:resulthandler:)
        self.task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            guard let self else { return }
            self.queue.async {
                self.handleLocked(result: result, error: error, epochMs: epochMs)
            }
        }
    }

    private func rotateRequestLocked() {
        request?.endAudio()
        request = nil
        startRequestLocked()
    }

    private func handleLocked(result: SFSpeechRecognitionResult?, error: Error?, epochMs: Double) {
        if let result {
            let transcription = result.bestTranscription
            let text = transcription.formattedString
            if !text.isEmpty {
                let segments = transcription.segments
                var startMs = epochMs
                var endMs = Double(totalFrames) * 1000.0 / sampleRate
                if let first = segments.first, let last = segments.last {
                    // SFTranscriptionSegment.timestamp/.duration are seconds
                    // within the current request's audio stream:
                    // https://developer.apple.com/documentation/speech/sftranscriptionsegment
                    startMs = epochMs + first.timestamp * 1000.0
                    endMs = epochMs + (last.timestamp + last.duration) * 1000.0
                }
                var confidence: Double?
                if result.isFinal, !segments.isEmpty {
                    let sum = segments.reduce(0.0) { $0 + Double($1.confidence) }
                    confidence = sum / Double(segments.count)
                }
                self.emit(
                    result.isFinal ? "transcript.final" : "transcript.partial",
                    AnyEncodable(
                        TranscriptEvent(
                            source: source, text: text,
                            startMs: Int(startMs.rounded()), endMs: Int(endMs.rounded()),
                            confidence: confidence, locale: localeId)))
            }
            if result.isFinal, !stopped {
                rotateRequestLocked()
                return
            }
        }

        if let error, !stopped {
            let ns = error as NSError
            // kAFAssistantErrorDomain 1110 ("no speech detected") and 216/301
            // (request cancelled/retired) are routine — restart quietly.
            let routine =
                ns.domain == "kAFAssistantErrorDomain"
                && [1110, 1101, 216, 203, 301].contains(ns.code)
            if routine {
                rotateRequestLocked()
            } else if recognizer?.isAvailable == false {
                reportUnavailable("speech recognizer became unavailable")
            } else {
                Log.shared.warn("speech(\(source)) task error: \(ns.domain) \(ns.code)")
                rotateRequestLocked()
            }
        }
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
