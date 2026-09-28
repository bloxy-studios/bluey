import AVFoundation
import AudioToolbox
import CoreAudio
import Foundation

/// Microphone capture: AVAudioEngine input tap → AVAudioConverter →
/// 16 kHz mono Int16 delivered to `onSamples`.
///
/// Device selection: on macOS the engine's inputNode wraps an AUHAL audio unit;
/// setting kAudioOutputUnitProperty_CurrentDevice on it (global scope, element 0)
/// routes a specific input device. Must happen before engine start.
/// https://developer.apple.com/documentation/audiotoolbox/kaudiooutputunitproperty_currentdevice
public final class MicrophoneCapture {
    public var onSamples: (([Int16]) -> Void)?
    public var onError: ((HelperError) -> Void)?
    /// Capture stopped (a rebuild after a device change failed) or resumed
    /// mid-session. Called on the mic queue.
    public var onLiveChange: ((Bool) -> Void)?
    /// Test seam: replaces building and starting the AVAudioEngine.
    var startEngineOverride: (() -> HelperError?)?

    private let queue = DispatchQueue(label: "com.codewithabdul.bluey.helper.mic")
    private let deviceService: AudioDeviceService
    private let targetSampleRate: Double

    private var engine: AVAudioEngine?
    private var converter: AVAudioConverter?
    private var converterInputFormat: AVAudioFormat?
    private var configChangeObserver: NSObjectProtocol?
    private var requestedDeviceUID: String?
    /// Between a successful `start` and `stop`: a failed rebuild keeps it set
    /// so the next device change retries (MAC-005).
    private var wantRunning = false
    /// The engine is capturing.
    private var live = false
    /// The requested device vanished and capture uses the default input.
    private var onFallbackDevice = false

    public init(deviceService: AudioDeviceService, targetSampleRate: Double = 16000) {
        self.deviceService = deviceService
        self.targetSampleRate = targetSampleRate
    }

    // MARK: lifecycle

    public func start(deviceUID: String?, completion: @escaping (HelperError?) -> Void) {
        queue.async { [weak self] in
            guard let self else { return }
            guard !self.wantRunning else {
                completion(nil)
                return
            }
            self.requestedDeviceUID = deviceUID
            let error = self.startEngine(fallBackToDefault: false)
            if error == nil {
                self.wantRunning = true
                self.live = true
                self.observeConfigurationChanges()
            }
            completion(error)
        }
    }

    public func stop() {
        queue.async { [weak self] in
            guard let self else { return }
            self.wantRunning = false
            self.live = false
            self.teardownEngine()
            if let observer = self.configChangeObserver {
                NotificationCenter.default.removeObserver(observer)
                self.configChangeObserver = nil
            }
        }
    }

    /// Rebuild after a device change (e.g. default input switched).
    public func restart() {
        queue.async { [weak self] in
            self?.restartLocked()
        }
    }

    /// After a device change: retry when a previous rebuild failed, or when
    /// the requested device is back after capture fell back to the default.
    public func retryIfDegraded() {
        queue.async { [weak self] in
            guard let self, !self.live || self.requestedDeviceReturned else { return }
            self.restartLocked()
        }
    }

    private var requestedDeviceReturned: Bool {
        guard onFallbackDevice, let uid = requestedDeviceUID else { return false }
        return deviceService.deviceID(forUID: uid) != nil
    }

    /// Must run on `queue`. A vanished requested device falls back to the
    /// default input rather than leaving the microphone dead.
    private func restartLocked() {
        guard wantRunning else { return }
        teardownEngine()
        let error = startEngine(fallBackToDefault: true)
        let wasLive = live
        live = error == nil
        if let error, wasLive { onError?(error) }
        if live != wasLive { onLiveChange?(live) }
    }

    private func startEngine(fallBackToDefault: Bool) -> HelperError? {
        if let startEngineOverride { return startEngineOverride() }
        return buildAndStartEngine(fallBackToDefault: fallBackToDefault)
    }

    // MARK: engine plumbing (all on `queue`)

    private func teardownEngine() {
        guard let engine else { return }
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        self.engine = nil
        converter = nil
        converterInputFormat = nil
    }

    private func buildAndStartEngine(fallBackToDefault: Bool) -> HelperError? {
        guard AVCaptureDevice.authorizationStatus(for: .audio) == .authorized else {
            return .permissionDenied("microphone", message: "Microphone not granted")
        }
        let engine = AVAudioEngine()
        let input = engine.inputNode

        onFallbackDevice = false
        if let uid = requestedDeviceUID, !uid.isEmpty {
            if let deviceID = deviceService.deviceID(forUID: uid) {
                guard let unit = input.audioUnit else {
                    return .audio("engine_unavailable", "input audio unit unavailable")
                }
                var device = deviceID
                let status = AudioUnitSetProperty(
                    unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0,
                    &device, UInt32(MemoryLayout<AudioDeviceID>.size))
                if status != noErr {
                    return .audio(
                        "device_select_failed", "could not select input device (\(status))")
                }
            } else if !fallBackToDefault {
                return .audio("device_not_found", "input device \(uid) not found")
            } else {
                Log.shared.warn("selected input device is gone; capturing from the default input")
                onFallbackDevice = true
            }
        }

        // Tap in the node's native format; convert per buffer below.
        // https://developer.apple.com/documentation/avfaudio/avaudionode/installtap(onbus:buffersize:format:block:)
        let tapFormat = input.outputFormat(forBus: 0)
        guard tapFormat.sampleRate > 0, tapFormat.channelCount > 0 else {
            return .audio("no_input_format", "input node has no valid format (no input device?)")
        }
        input.installTap(onBus: 0, bufferSize: 4096, format: tapFormat) { [weak self] buffer, _ in
            self?.queue.async {
                self?.convertAndDeliver(buffer)
            }
        }

        engine.prepare()
        do {
            try engine.start()
        } catch {
            engine.inputNode.removeTap(onBus: 0)
            return .audio("engine_start_failed", error.localizedDescription)
        }
        self.engine = engine
        return nil
    }

    /// Rebuild on AVAudioEngineConfigurationChange (sample-rate/device changes):
    /// https://developer.apple.com/documentation/avfaudio/avaudioengineconfigurationchangenotification
    private func observeConfigurationChanges() {
        guard configChangeObserver == nil else { return }
        configChangeObserver = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: nil, queue: nil
        ) { [weak self] notification in
            guard let self else { return }
            self.queue.async {
                // Only react to OUR engine (another short-lived engine, e.g. a
                // mic test, posts the same notification).
                guard let engine = self.engine,
                    (notification.object as? AVAudioEngine) === engine
                else { return }
                Log.shared.info("AVAudioEngineConfigurationChange -> restarting mic engine")
                self.restartLocked()
            }
        }
    }

    // MARK: conversion (on `queue`)

    private func convertAndDeliver(_ buffer: AVAudioPCMBuffer) {
        guard live else { return }
        guard
            let outFormat = AVAudioFormat(
                commonFormat: .pcmFormatInt16, sampleRate: targetSampleRate,
                channels: 1, interleaved: true)
        else { return }

        if converter == nil || converterInputFormat != buffer.format {
            // https://developer.apple.com/documentation/avfaudio/avaudioconverter/init(from:to:)
            converter = AVAudioConverter(from: buffer.format, to: outFormat)
            converterInputFormat = buffer.format
        }
        guard let converter else { return }

        let ratio = targetSampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 64
        guard let out = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: capacity) else {
            return
        }

        var fed = false
        // https://developer.apple.com/documentation/avfaudio/avaudioconverter/convert(to:error:withinputfrom:)
        let inputBlock: AVAudioConverterInputBlock = { _, outStatus in
            if fed {
                outStatus.pointee = .noDataNow
                return nil
            }
            fed = true
            outStatus.pointee = .haveData
            return buffer
        }
        var conversionError: NSError?
        let status = converter.convert(to: out, error: &conversionError, withInputFrom: inputBlock)
        guard status != .error else {
            Log.shared.warn("mic conversion failed: \(conversionError?.localizedDescription ?? "?")")
            return
        }
        guard out.frameLength > 0, let channel = out.int16ChannelData?[0] else { return }
        let samples = Array(UnsafeBufferPointer(start: channel, count: Int(out.frameLength)))
        onSamples?(samples)
    }

    // MARK: audio.testMicrophone

    /// Short standalone engine run that reports the peak level (0…1).
    public static func measurePeak(
        deviceUID: String?, durationMs: Int, deviceService: AudioDeviceService,
        completion: @escaping (Result<Double, HelperError>) -> Void
    ) {
        let capture = MicrophoneCapture(deviceService: deviceService)
        let lock = NSLock()
        var peak: Double = 0
        capture.onSamples = { samples in
            var localMax: Double = 0
            for s in samples {
                localMax = max(localMax, abs(Double(s)) / 32768.0)
            }
            lock.lock()
            peak = max(peak, localMax)
            lock.unlock()
        }
        capture.start(deviceUID: deviceUID) { error in
            if let error {
                completion(.failure(error))
                return
            }
            DispatchQueue.global().asyncAfter(
                deadline: .now() + .milliseconds(max(200, durationMs))
            ) {
                capture.stop()
                lock.lock()
                let value = peak
                lock.unlock()
                completion(.success(value))
            }
        }
    }
}
