import Foundation

/// `audio.start` params (docs/HELPER_PROTOCOL.md "AudioStartParams").
public struct AudioStartParams: Decodable {
    public struct Microphone: Decodable {
        public var enabled: Bool?
        public var deviceId: String?
    }
    public struct SystemAudio: Decodable {
        public var enabled: Bool?
    }
    public struct VAD: Decodable {
        public var enabled: Bool?
        public var sensitivity: VoiceActivityDetector.Sensitivity?
    }
    public struct Transcription: Decodable {
        public var enabled: Bool?
        public var locale: String?
        public var onDevice: Bool?
        public var sources: [String]?
    }
    public struct Levels: Decodable {
        public var enabled: Bool?
        public var intervalMs: Int?
    }

    public var microphone: Microphone?
    public var systemAudio: SystemAudio?
    public var sampleRate: Int?
    public var vad: VAD?
    public var emitPcm: Bool?
    public var chunkMs: Int?
    public var transcription: Transcription?
    public var levels: Levels?
}

public struct AudioStartResult: Encodable {
    public let ok: Bool
    public let microphone: Bool
    public let systemAudio: Bool
    public let sampleRate: Int
}

struct AudioChunkEvent: Encodable {
    let source: String
    let pcm16: String?
    let sampleRate: Int
    let startMs: Int
    let endMs: Int
    let isSpeech: Bool
    let rms: Double
}

struct AudioStartedEvent: Encodable {
    let microphone: Bool
    let systemAudio: Bool
    let device: AudioDeviceService.Device?
}

struct AudioStoppedEvent: Encodable {
    let reason: String // requested | device_lost | error
}

struct AudioDeviceChangedEvent: Encodable {
    let devices: [AudioDeviceService.Device]
    let currentInput: AudioDeviceService.Device?
}

struct AudioErrorEvent: Encodable {
    let code: String
    let message: String
    let kind: String
}

/// Orchestrates microphone + system audio capture, VAD, chunking, level
/// metering and on-device transcription. Raw audio only ever lives in the
/// in-flight chunk buffers — it is never written to disk.
public final class AudioSession {
    public typealias Emit = (_ event: String, _ data: AnyEncodable) -> Void

    private enum State { case idle, running, paused }

    private let queue = DispatchQueue(label: "com.codewithabdul.bluey.helper.audiosession")
    private let deviceService: AudioDeviceService
    private let emit: Emit

    private var state: State = .idle
    private var sampleRate = 16000
    private var emitPcm = false
    private var mic: MicrophoneCapture?
    private var system: SystemAudioCapture?
    private var chunkers: [String: PCMChunker] = [:]
    private var vads: [String: VoiceActivityDetector] = [:]
    private var transcribers: [String: SpeechTranscriber] = [:]
    private var levelMeter: LevelMeter?
    private var usingDefaultDevice = true

    public init(deviceService: AudioDeviceService, emit: @escaping Emit) {
        self.deviceService = deviceService
        self.emit = emit
        // Device listener runs for the whole helper lifetime so
        // audio.deviceChanged also fires while idle.
        deviceService.startListening { [weak self] in
            self?.handleDeviceChange()
        }
    }

    // MARK: audio.start / stop / pause / resume

    public func start(
        _ params: AudioStartParams, completion: @escaping (Result<AudioStartResult, HelperError>) -> Void
    ) {
        queue.async { [weak self] in
            guard let self else { return }
            guard self.state == .idle else {
                completion(.failure(.audio("audio_already_running", "audio session already active")))
                return
            }
            let wantMic = params.microphone?.enabled ?? false
            let wantSystem = params.systemAudio?.enabled ?? false
            guard wantMic || wantSystem else {
                completion(.failure(.invalidParams("enable microphone and/or systemAudio")))
                return
            }
            self.sampleRate = params.sampleRate ?? 16000
            self.emitPcm = params.emitPcm ?? false
            let chunkMs = params.chunkMs ?? 200

            // Per-source pipelines.
            for source in ["microphone", "system"] {
                self.chunkers[source] = PCMChunker(sampleRate: self.sampleRate, chunkMs: chunkMs)
                if params.vad?.enabled ?? true {
                    self.vads[source] = VoiceActivityDetector(
                        sensitivity: params.vad?.sensitivity ?? .medium)
                }
            }
            if params.levels?.enabled ?? true {
                let meter = LevelMeter(emit: self.emit)
                meter.start(intervalMs: params.levels?.intervalMs ?? 100)
                self.levelMeter = meter
            }
            if params.transcription?.enabled ?? false {
                var sources = params.transcription?.sources ?? ["microphone", "system"]
                // Only transcribe sources that are actually being captured.
                sources = sources.filter {
                    ($0 == "microphone" && wantMic) || ($0 == "system" && wantSystem)
                }
                for source in sources {
                    let transcriber = SpeechTranscriber(
                        source: source,
                        locale: params.transcription?.locale ?? "en-US",
                        onDevice: params.transcription?.onDevice ?? true,
                        sampleRate: Double(self.sampleRate),
                        emit: self.emit)
                    if transcriber.start() {
                        self.transcribers[source] = transcriber
                    }
                }
            }

            self.state = .running
            self.startCaptures(
                wantMic: wantMic, micDeviceId: params.microphone?.deviceId,
                wantSystem: wantSystem, completion: completion)
        }
    }

    /// Runs on `queue`.
    private func startCaptures(
        wantMic: Bool, micDeviceId: String?, wantSystem: Bool,
        completion: @escaping (Result<AudioStartResult, HelperError>) -> Void
    ) {
        let group = DispatchGroup()
        let queue = self.queue
        // Mutated ONLY on `queue` (start callbacks hop first) → no data race.
        var micOk = false
        var systemOk = false
        var firstError: HelperError?

        if wantMic {
            usingDefaultDevice = (micDeviceId == nil)
            let mic = MicrophoneCapture(deviceService: deviceService, targetSampleRate: Double(sampleRate))
            mic.onSamples = { [weak self] samples in
                self?.ingest(source: "microphone", samples: samples)
            }
            mic.onError = { [weak self] error in
                self?.emitError(error)
            }
            self.mic = mic
            group.enter()
            mic.start(deviceUID: micDeviceId) { error in
                queue.async {
                    if let error { firstError = firstError ?? error } else { micOk = true }
                    group.leave()
                }
            }
        }

        if wantSystem {
            let system = SystemAudioCapture(targetSampleRate: Double(sampleRate))
            system.onSamples = { [weak self] samples in
                self?.ingest(source: "system", samples: samples)
            }
            system.onStopped = { [weak self] error in
                guard let self, let error else { return }
                self.emitError(error)
                self.stopInternal(reason: "error")
            }
            self.system = system
            group.enter()
            system.start { error in
                queue.async {
                    if let error { firstError = firstError ?? error } else { systemOk = true }
                    group.leave()
                }
            }
        }

        group.notify(queue: queue) { [weak self] in
            guard let self else { return }
            if !micOk && !systemOk {
                self.teardown()
                self.state = .idle
                completion(.failure(firstError ?? .audio("audio_start_failed", "no source started")))
                return
            }
            if let firstError {
                // Partial start: report the failed source but keep running.
                self.emitError(firstError)
            }
            self.emit(
                "audio.started",
                AnyEncodable(
                    AudioStartedEvent(
                        microphone: micOk, systemAudio: systemOk,
                        device: micOk ? self.deviceService.defaultInputDevice() : nil)))
            completion(
                .success(
                    AudioStartResult(
                        ok: true, microphone: micOk, systemAudio: systemOk,
                        sampleRate: self.sampleRate)))
        }
    }

    public func stop(completion: @escaping (Result<OkResult, HelperError>) -> Void) {
        queue.async { [weak self] in
            guard let self else { return }
            guard self.state != .idle else {
                completion(.success(OkResult()))
                return
            }
            self.stopLockedEmitting(reason: "requested")
            completion(.success(OkResult()))
        }
    }

    /// Stop triggered internally (device loss / stream error).
    private func stopInternal(reason: String) {
        queue.async { [weak self] in
            guard let self, self.state != .idle else { return }
            self.stopLockedEmitting(reason: reason)
        }
    }

    /// Must run on `queue`.
    private func stopLockedEmitting(reason: String) {
        // Flush trailing partial chunks before teardown.
        for (source, chunker) in chunkers {
            if let chunk = chunker.flush() {
                deliver(chunk: chunk, source: source)
            }
        }
        teardown()
        state = .idle
        emit("audio.stopped", AnyEncodable(AudioStoppedEvent(reason: reason)))
    }

    /// Must run on `queue`.
    private func teardown() {
        mic?.stop()
        mic = nil
        system?.stop()
        system = nil
        for transcriber in transcribers.values {
            transcriber.finish()
        }
        transcribers.removeAll()
        levelMeter?.stop()
        levelMeter = nil
        chunkers.removeAll()
        vads.removeAll()
    }

    public func pause(completion: @escaping (Result<OkResult, HelperError>) -> Void) {
        queue.async { [weak self] in
            guard let self else { return }
            if self.state == .running { self.state = .paused }
            completion(.success(OkResult()))
        }
    }

    public func resume(completion: @escaping (Result<OkResult, HelperError>) -> Void) {
        queue.async { [weak self] in
            guard let self else { return }
            if self.state == .paused { self.state = .running }
            completion(.success(OkResult()))
        }
    }

    // MARK: sample ingestion

    private func ingest(source: String, samples: [Int16]) {
        queue.async { [weak self] in
            guard let self, self.state == .running else { return }
            guard let chunker = self.chunkers[source] else { return }
            for chunk in chunker.append(samples) {
                self.deliver(chunk: chunk, source: source)
            }
        }
    }

    /// Must run on `queue`.
    private func deliver(chunk: PCMChunk, source: String) {
        let rms = VoiceActivityDetector.rms(of: chunk.samples)
        let isSpeech = vads[source]?.process(rms: rms, atMs: chunk.endMs) ?? true
        levelMeter?.update(source: source, rms: min(1.0, rms * 4)) // perceptual boost for UI meters
        transcribers[source]?.append(samples: chunk.samples)

        var pcm16: String?
        if emitPcm {
            pcm16 = chunk.samples.withUnsafeBufferPointer { ptr in
                Data(buffer: ptr).base64EncodedString()
            }
        }
        emit(
            "audio.chunk",
            AnyEncodable(
                AudioChunkEvent(
                    source: source, pcm16: pcm16, sampleRate: sampleRate,
                    startMs: Int(chunk.startMs.rounded()), endMs: Int(chunk.endMs.rounded()),
                    isSpeech: isSpeech, rms: (rms * 1000).rounded() / 1000)))
    }

    // MARK: devices

    public struct TestMicrophoneParams: Decodable {
        public var deviceId: String?
        public var durationMs: Int?
    }

    public struct TestMicrophoneResult: Encodable {
        public let peakLevel: Double
        public let ok: Bool
    }

    public func testMicrophone(
        _ params: TestMicrophoneParams,
        completion: @escaping (Result<TestMicrophoneResult, HelperError>) -> Void
    ) {
        MicrophoneCapture.measurePeak(
            deviceUID: params.deviceId, durationMs: params.durationMs ?? 1500,
            deviceService: deviceService
        ) { result in
            switch result {
            case .failure(let error):
                completion(.failure(error))
            case .success(let peak):
                completion(
                    .success(TestMicrophoneResult(peakLevel: (peak * 1000).rounded() / 1000, ok: true)))
            }
        }
    }

    private func handleDeviceChange() {
        queue.async { [weak self] in
            guard let self else { return }
            let devices = self.deviceService.listInputDevices()
            let current = devices.first { $0.isDefault }
            self.emit(
                "audio.deviceChanged",
                AnyEncodable(AudioDeviceChangedEvent(devices: devices, currentInput: current)))
            if self.state != .idle, self.usingDefaultDevice {
                if devices.isEmpty {
                    self.emitError(.audio("device_lost", "no input devices available"))
                    self.stopLockedEmitting(reason: "device_lost")
                } else {
                    // MicrophoneCapture also reacts to AVAudioEngineConfigurationChange;
                    // restart() is idempotent and cheap.
                    self.mic?.restart()
                }
            }
        }
    }

    private func emitError(_ error: HelperError) {
        emit(
            "audio.error",
            AnyEncodable(AudioErrorEvent(code: error.code, message: error.message, kind: "audio")))
    }
}
