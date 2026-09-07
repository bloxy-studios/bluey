import CoreAudio
import CoreMedia
import Foundation
import ScreenCaptureKit

/// System (loopback) audio via ScreenCaptureKit — SCStream with
/// `capturesAudio = true`, excluding Bluey's own process audio.
///
/// SCStreamConfiguration.sampleRate officially supports 8000/16000/24000/48000
/// (https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/samplerate)
/// so we request 16 kHz mono directly; a defensive downmix/resample path still
/// handles any deviating delivery format.
///
/// ScreenCaptureKit will not run an audio-only stream: a minimal 2×2 @ 1 fps
/// video output is attached and discarded (well-known SCKit pattern).
public final class SystemAudioCapture: NSObject, SCStreamOutput, SCStreamDelegate {
    public var onSamples: (([Int16]) -> Void)?
    public var onStopped: ((HelperError?) -> Void)?

    private let queue = DispatchQueue(label: "com.codewithabdul.bluey.helper.sysaudio")
    private let sampleQueue = DispatchQueue(label: "com.codewithabdul.bluey.helper.sysaudio.samples")
    private let targetSampleRate: Double
    private var stream: SCStream?
    private var running = false

    public init(targetSampleRate: Double = 16000) {
        self.targetSampleRate = targetSampleRate
        super.init()
    }

    // MARK: lifecycle

    public func start(completion: @escaping (HelperError?) -> Void) {
        ShareableContent.fetch { [weak self] result in
            guard let self else { return }
            switch result {
            case .failure(let error):
                completion(error)
            case .success(let content):
                self.queue.async {
                    self.startLocked(content: content, completion: completion)
                }
            }
        }
    }

    private func startLocked(content: SCShareableContent, completion: @escaping (HelperError?) -> Void) {
        guard stream == nil else {
            completion(nil)
            return
        }
        guard let display = ShareableContent.display(withId: nil, in: content) else {
            completion(.capture("display_not_found", "no display for system audio stream"))
            return
        }
        // Whole display minus Bluey's own windows. Audio capture itself is
        // display-wide; excludesCurrentProcessAudio removes our own output.
        let filter = SCContentFilter(
            display: display, excludingWindows: ShareableContent.ownWindows(in: content))

        let config = SCStreamConfiguration()
        config.capturesAudio = true
        // https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/excludescurrentprocessaudio
        config.excludesCurrentProcessAudio = true
        config.sampleRate = Int(targetSampleRate)
        config.channelCount = 1
        // Cheapest possible mandatory video surface (discarded).
        config.width = 2
        config.height = 2
        config.minimumFrameInterval = CMTime(value: 1, timescale: 1)
        config.queueDepth = 3
        config.showsCursor = false

        let stream = SCStream(filter: filter, configuration: config, delegate: self)
        do {
            try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: sampleQueue)
            try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: sampleQueue)
        } catch {
            completion(.audio("system_audio_output_failed", error.localizedDescription))
            return
        }
        stream.startCapture { [weak self] error in
            guard let self else { return }
            if let error {
                self.queue.async {
                    self.stream = nil
                    self.running = false
                }
                completion(Self.mapStartError(error))
            } else {
                completion(nil)
            }
        }
        self.stream = stream
        running = true
    }

    public func stop() {
        queue.async { [weak self] in
            guard let self, let stream = self.stream else { return }
            self.stream = nil
            self.running = false
            stream.stopCapture { error in
                if let error {
                    Log.shared.warn("system audio stopCapture: \(error.localizedDescription)")
                }
            }
        }
    }

    private static func mapStartError(_ error: Error) -> HelperError {
        let ns = error as NSError
        if ns.domain == SCStreamErrorDomain, ns.code == SCStreamError.Code.userDeclined.rawValue {
            return .permissionDenied("screenRecording", message: "Screen Recording not granted")
        }
        return .audio("system_audio_start_failed", ns.localizedDescription)
    }

    // MARK: SCStreamOutput

    public func stream(
        _ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of type: SCStreamOutputType
    ) {
        guard type == .audio, running, sampleBuffer.isValid else { return }
        guard let samples = Self.int16Mono(from: sampleBuffer, targetRate: targetSampleRate) else {
            return
        }
        if !samples.isEmpty {
            onSamples?(samples)
        }
    }

    // MARK: SCStreamDelegate

    public func stream(_ stream: SCStream, didStopWithError error: Error) {
        Log.shared.error("system audio stream stopped: \(error.localizedDescription)")
        queue.async { [weak self] in
            guard let self else { return }
            self.stream = nil
            self.running = false
            self.onStopped?(.audio("system_audio_stopped", error.localizedDescription))
        }
    }

    // MARK: CMSampleBuffer → 16 kHz mono Int16

    /// Extract the AudioBufferList (two-call pattern via
    /// CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer), downmix to
    /// mono, convert Float32→Int16, and linearly resample if the delivered
    /// rate differs from the target.
    /// https://developer.apple.com/documentation/coremedia/cmsamplebuffergetaudiobufferlistwithretainedblockbuffer(_:bufferlistsizeneededout:bufferlistout:bufferlistsize:blockbufferallocator:blockbuffermemoryallocator:flags:blockbufferout:)
    static func int16Mono(from sampleBuffer: CMSampleBuffer, targetRate: Double) -> [Int16]? {
        guard let formatDesc = CMSampleBufferGetFormatDescription(sampleBuffer),
            let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(formatDesc)?.pointee
        else { return nil }

        var ablSize = 0
        CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sampleBuffer, bufferListSizeNeededOut: &ablSize, bufferListOut: nil, bufferListSize: 0,
            blockBufferAllocator: nil, blockBufferMemoryAllocator: nil,
            flags: kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment, blockBufferOut: nil)
        guard ablSize > 0 else { return nil }

        let raw = UnsafeMutableRawPointer.allocate(byteCount: ablSize, alignment: 16)
        defer { raw.deallocate() }
        let ablPtr = raw.bindMemory(to: AudioBufferList.self, capacity: 1)
        var blockBuffer: CMBlockBuffer?
        let status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sampleBuffer, bufferListSizeNeededOut: nil, bufferListOut: ablPtr,
            bufferListSize: ablSize, blockBufferAllocator: nil, blockBufferMemoryAllocator: nil,
            flags: kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            blockBufferOut: &blockBuffer)
        guard status == noErr else { return nil }

        let buffers = UnsafeMutableAudioBufferListPointer(ablPtr)
        let isFloat = (asbd.mFormatFlags & kAudioFormatFlagIsFloat) != 0
        let channelBuffers = Array(buffers)
        guard !channelBuffers.isEmpty else { return nil }

        // Non-interleaved: one AudioBuffer per channel; average channels.
        var mono: [Float] = []
        if isFloat {
            let perChannel: [[Float]] = channelBuffers.compactMap { buf in
                guard let data = buf.mData else { return nil }
                let count = Int(buf.mDataByteSize) / MemoryLayout<Float>.size
                let ptr = data.assumingMemoryBound(to: Float.self)
                return Array(UnsafeBufferPointer(start: ptr, count: count))
            }
            guard let first = perChannel.first else { return nil }
            if perChannel.count == 1 {
                // Interleaved multi-channel in a single buffer.
                let ch = Int(asbd.mChannelsPerFrame)
                if ch > 1 {
                    mono = stride(from: 0, to: first.count - (ch - 1), by: ch).map { i in
                        var sum: Float = 0
                        for c in 0..<ch { sum += first[i + c] }
                        return sum / Float(ch)
                    }
                } else {
                    mono = first
                }
            } else {
                let frames = perChannel.map(\.count).min() ?? 0
                mono = (0..<frames).map { i in
                    var sum: Float = 0
                    for channel in perChannel { sum += channel[i] }
                    return sum / Float(perChannel.count)
                }
            }
        } else {
            // Int16 delivery (defensive; SCStream normally delivers Float32).
            guard let data = channelBuffers[0].mData else { return nil }
            let count = Int(channelBuffers[0].mDataByteSize) / MemoryLayout<Int16>.size
            let ptr = data.assumingMemoryBound(to: Int16.self)
            let ints = Array(UnsafeBufferPointer(start: ptr, count: count))
            let ch = max(1, Int(asbd.mChannelsPerFrame))
            if ch == 1 && channelBuffers.count == 1 && asbd.mSampleRate == targetRate {
                return ints
            }
            mono = stride(from: 0, to: ints.count - (ch - 1), by: ch).map { i in
                var sum: Float = 0
                for c in 0..<ch { sum += Float(ints[i + c]) / 32768.0 }
                return sum / Float(ch)
            }
        }

        // Resample if needed (linear; only a fallback — normally 16 k already).
        if asbd.mSampleRate != targetRate, asbd.mSampleRate > 0, mono.count > 1 {
            let ratio = asbd.mSampleRate / targetRate
            let outCount = max(1, Int(Double(mono.count) / ratio))
            mono = (0..<outCount).map { i in
                let pos = Double(i) * ratio
                let idx = Int(pos)
                let frac = Float(pos - Double(idx))
                let a = mono[min(idx, mono.count - 1)]
                let b = mono[min(idx + 1, mono.count - 1)]
                return a + (b - a) * frac
            }
        }

        return mono.map { sample in
            guard sample.isFinite else { return 0 }
            let scaled = (max(-1.0, min(1.0, sample)) * 32767.0).rounded()
            return Int16(scaled)
        }
    }
}
