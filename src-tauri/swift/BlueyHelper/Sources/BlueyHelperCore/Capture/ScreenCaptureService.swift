import AppKit
import CoreGraphics
import CoreVideo
import Foundation
import ScreenCaptureKit

/// On-demand screenshots via SCScreenshotManager (macOS 14+).
/// https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager
public final class ScreenCaptureService {
    /// Fixed threshold for one-shot capture change detection (observe.start has
    /// its own configurable minDelta).
    static let screenshotMinDelta = 0.04

    private let tempFrames: TempFrames
    private let changeDetector: ChangeDetector

    public init(tempFrames: TempFrames, changeDetector: ChangeDetector) {
        self.tempFrames = tempFrames
        self.changeDetector = changeDetector
    }

    // MARK: - capture.display

    public func captureDisplay(
        _ params: CaptureParams, completion: @escaping (Result<Frame, HelperError>) -> Void
    ) {
        let startedMs = Clock.monotonicMs()
        ShareableContent.fetch { [weak self] result in
            guard let self else { return }
            switch result {
            case .failure(let error):
                completion(.failure(error))
            case .success(let content):
                guard let display = ShareableContent.display(withId: params.displayId, in: content) else {
                    completion(
                        .failure(
                            .capture("display_not_found", "display \(params.displayId ?? "main") not found")))
                    return
                }
                let excluded = params.resolvedExcludeSelf ? ShareableContent.ownWindows(in: content) : []
                // https://developer.apple.com/documentation/screencapturekit/sccontentfilter/init(display:excludingwindows:)
                let filter = SCContentFilter(display: display, excludingWindows: excluded)
                let scale = ShareableContent.scaleFactor(forDisplayID: display.displayID)
                // SCDisplay.width/height are in points:
                // https://developer.apple.com/documentation/screencapturekit/scdisplay/width
                let size = Self.fit(
                    widthPx: Double(display.width) * scale,
                    heightPx: Double(display.height) * scale,
                    maxDimension: params.resolvedMaxDimension)
                let config = Self.screenshotConfig(width: size.width, height: size.height)
                self.performScreenshot(
                    filter: filter, config: config, params: params,
                    targetKey: "display:\(display.displayID)",
                    displayId: String(display.displayID),
                    scaleFactor: scale, startedMs: startedMs, completion: completion)
            }
        }
    }

    // MARK: - capture.window

    public func captureWindow(
        _ params: CaptureParams, completion: @escaping (Result<Frame, HelperError>) -> Void
    ) {
        guard let windowId = params.windowId, windowId > 0 else {
            completion(.failure(.invalidParams("capture.window requires windowId")))
            return
        }
        let startedMs = Clock.monotonicMs()
        // onScreenWindowsOnly:false so windows on other Spaces are still resolvable.
        ShareableContent.fetch(onScreenWindowsOnly: false) { [weak self] result in
            guard let self else { return }
            switch result {
            case .failure(let error):
                completion(.failure(error))
            case .success(let content):
                guard let window = ShareableContent.window(withId: UInt32(windowId), in: content) else {
                    completion(.failure(.capture("window_not_found", "window \(windowId) not found")))
                    return
                }
                if params.resolvedExcludeSelf, ShareableContent.isOwnWindow(window) {
                    completion(.failure(.capture("window_is_self", "refusing to capture Bluey's own window")))
                    return
                }
                // https://developer.apple.com/documentation/screencapturekit/sccontentfilter/init(desktopindependentwindow:)
                let filter = SCContentFilter(desktopIndependentWindow: window)
                // pointPixelScale (macOS 14+) is the exact point→pixel factor of
                // the filtered content:
                // https://developer.apple.com/documentation/screencapturekit/sccontentfilter/pointpixelscale
                var scale = Double(filter.pointPixelScale)
                if scale <= 0 {
                    let display = ShareableContent.display(containing: window, in: content)
                    scale = display.map { ShareableContent.scaleFactor(forDisplayID: $0.displayID) } ?? 2.0
                }
                let size = Self.fit(
                    widthPx: Double(window.frame.width) * scale,
                    heightPx: Double(window.frame.height) * scale,
                    maxDimension: params.resolvedMaxDimension)
                let config = Self.screenshotConfig(width: size.width, height: size.height)
                let display = ShareableContent.display(containing: window, in: content)
                self.performScreenshot(
                    filter: filter, config: config, params: params,
                    targetKey: "window:\(windowId)",
                    displayId: display.map { String($0.displayID) },
                    scaleFactor: scale, startedMs: startedMs, completion: completion)
            }
        }
    }

    // MARK: - capture.region

    public func captureRegion(
        _ params: CaptureParams, completion: @escaping (Result<Frame, HelperError>) -> Void
    ) {
        guard let rect = params.rect, rect.width > 0, rect.height > 0 else {
            completion(.failure(.invalidParams("capture.region requires a non-empty rect")))
            return
        }
        let startedMs = Clock.monotonicMs()
        ShareableContent.fetch { [weak self] result in
            guard let self else { return }
            switch result {
            case .failure(let error):
                completion(.failure(error))
            case .success(let content):
                guard let display = ShareableContent.display(withId: params.displayId, in: content) else {
                    completion(
                        .failure(
                            .capture("display_not_found", "display \(params.displayId ?? "main") not found")))
                    return
                }
                let excluded = params.resolvedExcludeSelf ? ShareableContent.ownWindows(in: content) : []
                let filter = SCContentFilter(display: display, excludingWindows: excluded)
                let scale = ShareableContent.scaleFactor(forDisplayID: display.displayID)
                let size = Self.fit(
                    widthPx: rect.width * scale,
                    heightPx: rect.height * scale,
                    maxDimension: params.resolvedMaxDimension)
                let config = Self.screenshotConfig(width: size.width, height: size.height)
                // sourceRect is in points in the filtered display's local
                // (top-left-origin) coordinate system:
                // https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/sourcerect
                config.sourceRect = rect.cgRect
                let key =
                    "region:\(display.displayID):\(Int(rect.x)),\(Int(rect.y)),\(Int(rect.width)),\(Int(rect.height))"
                self.performScreenshot(
                    filter: filter, config: config, params: params,
                    targetKey: key,
                    displayId: String(display.displayID),
                    scaleFactor: scale, startedMs: startedMs, completion: completion)
            }
        }
    }

    // MARK: - capture.activeWindow

    public func captureActiveWindow(
        _ params: CaptureParams, completion: @escaping (Result<Frame, HelperError>) -> Void
    ) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard let app = NSWorkspace.shared.frontmostApplication else {
                completion(.failure(.capture("no_frontmost_app", "no frontmost application")))
                return
            }
            let pid = app.processIdentifier
            guard let windowId = FrontmostAppService.frontmostWindowID(pid: pid) else {
                completion(
                    .failure(
                        .capture(
                            "no_active_window",
                            "no on-screen window found for \(app.localizedName ?? String(pid))")))
                return
            }
            var withWindow = params
            withWindow.windowId = Int(windowId)
            self.captureWindow(withWindow, completion: completion)
        }
    }

    // MARK: - shared pipeline

    static func screenshotConfig(width: Int, height: Int) -> SCStreamConfiguration {
        let config = SCStreamConfiguration()
        config.width = max(1, width)
        config.height = max(1, height)
        config.showsCursor = false
        // BGRA is the default screenshot-friendly pixel format.
        config.pixelFormat = kCVPixelFormatType_32BGRA
        return config
    }

    /// Fit pixel dimensions so the longest side is ≤ maxDimension.
    static func fit(widthPx: Double, heightPx: Double, maxDimension: Int) -> (width: Int, height: Int) {
        let w = max(1.0, widthPx)
        let h = max(1.0, heightPx)
        let longest = max(w, h)
        let scale = longest > Double(maxDimension) ? Double(maxDimension) / longest : 1.0
        return (max(1, Int((w * scale).rounded())), max(1, Int((h * scale).rounded())))
    }

    private func performScreenshot(
        filter: SCContentFilter,
        config: SCStreamConfiguration,
        params: CaptureParams,
        targetKey: String,
        displayId: String?,
        scaleFactor: Double,
        startedMs: Double,
        completion: @escaping (Result<Frame, HelperError>) -> Void
    ) {
        // Completion-handler variant of the macOS 14+ API
        // SCScreenshotManager.captureImage(contentFilter:configuration:completionHandler:)
        // https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager
        SCScreenshotManager.captureImage(contentFilter: filter, configuration: config) {
            [weak self] image, error in
            guard let self else { return }
            if let error {
                completion(.failure(Self.mapCaptureError(error)))
                return
            }
            guard let image else {
                completion(.failure(.capture("empty_frame", "screenshot returned no image")))
                return
            }
            self.finalize(
                image: image, params: params, targetKey: targetKey,
                displayId: displayId, scaleFactor: scaleFactor,
                startedMs: startedMs, completion: completion)
        }
    }

    private func finalize(
        image: CGImage,
        params: CaptureParams,
        targetKey: String,
        displayId: String?,
        scaleFactor: Double,
        startedMs: Double,
        completion: @escaping (Result<Frame, HelperError>) -> Void
    ) {
        // Safety net: SCScreenshotManager already rendered at config size, but
        // downscale defensively in case the OS returned a larger surface.
        let finalImage = ImageEncoder.downscale(image, maxDimension: params.resolvedMaxDimension)

        var hashHex = ""
        var changed = true
        if let grid = ImageEncoder.grayGrid(finalImage) {
            let hash = DHash.compute(gray: grid)
            hashHex = DHash.hex(hash)
            if params.resolvedChangeDetection {
                changed =
                    changeDetector.evaluate(
                        target: targetKey, hash: hash, minDelta: Self.screenshotMinDelta
                    ).changed
            }
        }

        let format = params.resolvedFormat
        guard let data = ImageEncoder.encode(finalImage, format: format, quality: params.resolvedQuality)
        else {
            completion(.failure(.capture("encode_failed", "could not encode frame")))
            return
        }

        let frameId = "f-" + UUID().uuidString.lowercased()
        // The temp file is always written: `ocr.recognize` and `capture.discard`
        // work by path, and Rust caches the path per frame id. `inline` only adds
        // the encoded image to the frame so the caller skips a second read — an
        // inline-only frame used to leave OCR with an unknown frame id.
        let url = tempFrames.url(id: frameId, fileExtension: format.fileExtension)
        var path: String?
        do {
            try data.write(to: url, options: [.atomic])
            path = url.path
        } catch {
            guard params.resolvedInline else {
                completion(
                    .failure(.internalError("failed to write frame: \(error.localizedDescription)")))
                return
            }
            // The inline image still serves the caller (and OCR by `image`).
            Log.shared.warn(
                "failed to write frame \(frameId) to disk; returning it inline only: \(error.localizedDescription)")
        }
        let inlineImage: String? = params.resolvedInline ? data.base64EncodedString() : nil

        completion(
            .success(
                Frame(
                    id: frameId, path: path, image: inlineImage,
                    mimeType: format.mimeType,
                    width: finalImage.width, height: finalImage.height,
                    displayId: displayId, scaleFactor: scaleFactor,
                    capturedAt: Clock.isoNow(), hash: hashHex, changed: changed,
                    durationMs: Int((Clock.monotonicMs() - startedMs).rounded()))))
    }

    static func mapCaptureError(_ error: Error) -> HelperError {
        let ns = error as NSError
        // SCStreamError.Code.userDeclined → the user has not granted (or revoked)
        // Screen Recording: https://developer.apple.com/documentation/screencapturekit/scstreamerror
        if ns.domain == SCStreamErrorDomain,
            ns.code == SCStreamError.Code.userDeclined.rawValue
        {
            return .permissionDenied("screenRecording", message: "Screen Recording not granted")
        }
        return .capture("screenshot_failed", ns.localizedDescription)
    }
}
