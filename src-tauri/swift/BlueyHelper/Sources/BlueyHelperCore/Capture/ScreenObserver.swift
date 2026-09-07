import CoreGraphics
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit

/// `observe.start` / `observe.stop`: a low-FPS SCStream that dHashes each
/// sampled frame and emits `screen.changed` when the normalized Hamming
/// distance ≥ minDelta.
public final class ScreenObserver: NSObject, SCStreamOutput, SCStreamDelegate {
    public typealias Emit = (_ event: String, _ data: AnyEncodable) -> Void

    private let emit: Emit
    private let queue = DispatchQueue(label: "com.codewithabdul.bluey.helper.observe")
    private let sampleQueue = DispatchQueue(label: "com.codewithabdul.bluey.helper.observe.samples")
    private var stream: SCStream?
    private var displayId: String?
    private var minDelta: Double = 0.04
    private var lastHash: UInt64?

    public init(emit: @escaping Emit) {
        self.emit = emit
        super.init()
    }

    public var isRunning: Bool {
        queue.sync { stream != nil }
    }

    // MARK: start / stop

    public func start(_ params: ObserveParams, completion: @escaping (Result<OkResult, HelperError>) -> Void) {
        ShareableContent.fetch { [weak self] result in
            guard let self else { return }
            switch result {
            case .failure(let error):
                completion(.failure(error))
            case .success(let content):
                self.queue.async {
                    self.startLocked(params: params, content: content, completion: completion)
                }
            }
        }
    }

    private func startLocked(
        params: ObserveParams, content: SCShareableContent,
        completion: @escaping (Result<OkResult, HelperError>) -> Void
    ) {
        if stream != nil {
            // Restart with new settings.
            stopStreamLocked()
        }
        guard let display = ShareableContent.display(withId: params.displayId, in: content) else {
            completion(.failure(.capture("display_not_found", "display \(params.displayId ?? "main") not found")))
            return
        }
        minDelta = params.resolvedMinDelta
        displayId = String(display.displayID)
        lastHash = nil

        let filter = SCContentFilter(
            display: display, excludingWindows: ShareableContent.ownWindows(in: content))

        let config = SCStreamConfiguration()
        // Tiny frames: the observer only needs enough pixels for a 9×8 dHash.
        let aspect = Double(display.height) / Double(max(1, display.width))
        config.width = 160
        config.height = max(8, Int((160.0 * aspect).rounded()))
        config.pixelFormat = kCVPixelFormatType_32BGRA
        config.showsCursor = false
        config.queueDepth = 3
        // minimumFrameInterval throttles delivery (CMTime of intervalMs):
        // https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/minimumframeinterval
        config.minimumFrameInterval = CMTime(
            value: CMTimeValue(params.resolvedIntervalMs), timescale: 1000)

        let stream = SCStream(filter: filter, configuration: config, delegate: self)
        do {
            // https://developer.apple.com/documentation/screencapturekit/scstream/addstreamoutput(_:type:samplehandlerqueue:)
            try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: sampleQueue)
        } catch {
            completion(.failure(.capture("observe_output_failed", error.localizedDescription)))
            return
        }
        stream.startCapture { [weak self] error in
            guard let self else { return }
            if let error {
                self.queue.async { self.stream = nil }
                completion(.failure(ScreenCaptureService.mapCaptureError(error)))
            } else {
                completion(.success(OkResult()))
            }
        }
        self.stream = stream
    }

    public func stop(completion: @escaping (Result<OkResult, HelperError>) -> Void) {
        queue.async { [weak self] in
            self?.stopStreamLocked()
            completion(.success(OkResult()))
        }
    }

    /// Must be called on `queue`.
    private func stopStreamLocked() {
        guard let stream else { return }
        self.stream = nil
        lastHash = nil
        stream.stopCapture { error in
            if let error {
                Log.shared.warn("observe stopCapture: \(error.localizedDescription)")
            }
        }
    }

    // MARK: SCStreamOutput

    public func stream(
        _ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of type: SCStreamOutputType
    ) {
        guard type == .screen, sampleBuffer.isValid else { return }
        // Idle/status-only buffers carry no image (SCStreamFrameInfo.status != .complete).
        guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        guard let gray = Self.grayGrid(from: pixelBuffer) else { return }
        let hash = DHash.compute(gray: gray)

        queue.async { [weak self] in
            guard let self, self.stream != nil else { return }
            defer { self.lastHash = hash }
            guard let previous = self.lastHash else { return }  // first frame → baseline only
            let delta = DHash.delta(previous, hash)
            guard delta >= self.minDelta else { return }
            self.emit(
                "screen.changed",
                AnyEncodable(
                    ScreenChangedEvent(
                        hash: DHash.hex(hash), delta: delta,
                        displayId: self.displayId, at: Clock.isoNow())))
        }
    }

    // MARK: SCStreamDelegate

    /// https://developer.apple.com/documentation/screencapturekit/scstreamdelegate/stream(_:didstopwitherror:)
    public func stream(_ stream: SCStream, didStopWithError error: Error) {
        Log.shared.error("observe stream stopped: \(error.localizedDescription)")
        queue.async { [weak self] in
            self?.stream = nil
            self?.lastHash = nil
        }
    }

    // MARK: pixels → 9×8 luminance grid

    /// Nearest-neighbour sample of a BGRA CVPixelBuffer into the dHash grid.
    static func grayGrid(from pixelBuffer: CVPixelBuffer) -> [Double]? {
        guard CVPixelBufferLockBaseAddress(pixelBuffer, .readOnly) == kCVReturnSuccess else {
            return nil
        }
        defer { CVPixelBufferUnlockBaseAddress(pixelBuffer, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(pixelBuffer) else { return nil }
        let width = CVPixelBufferGetWidth(pixelBuffer)
        let height = CVPixelBufferGetHeight(pixelBuffer)
        let stride = CVPixelBufferGetBytesPerRow(pixelBuffer)
        guard width >= DHash.gridWidth, height >= DHash.gridHeight else { return nil }

        let bytes = base.assumingMemoryBound(to: UInt8.self)
        var grid = [Double](repeating: 0, count: DHash.gridWidth * DHash.gridHeight)
        for gy in 0..<DHash.gridHeight {
            let sy = (gy * height) / DHash.gridHeight
            for gx in 0..<DHash.gridWidth {
                let sx = (gx * width) / DHash.gridWidth
                let p = sy * stride + sx * 4  // BGRA
                let b = Double(bytes[p])
                let g = Double(bytes[p + 1])
                let r = Double(bytes[p + 2])
                grid[gy * DHash.gridWidth + gx] = 0.299 * r + 0.587 * g + 0.114 * b
            }
        }
        return grid
    }
}
