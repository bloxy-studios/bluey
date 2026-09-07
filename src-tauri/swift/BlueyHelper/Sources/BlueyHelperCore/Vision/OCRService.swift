import CoreGraphics
import Foundation
import Vision

/// `ocr.recognize` via VNRecognizeTextRequest.
/// https://developer.apple.com/documentation/vision/vnrecognizetextrequest
public final class OCRService {
    public init() {}

    public struct Params: Decodable {
        public var path: String?
        public var image: String? // base64
        public var level: String? // "fast" | "accurate"
        public var languages: [String]?
        public var minConfidence: Double?
    }

    public struct Block: Codable, Equatable {
        public let text: String
        public let confidence: Double
        /// Normalized 0–1, **top-left** origin (converted from Vision's
        /// bottom-left-origin boundingBox).
        public let boundingBox: RectJSON

        public init(text: String, confidence: Double, boundingBox: RectJSON) {
            self.text = text
            self.confidence = confidence
            self.boundingBox = boundingBox
        }
    }

    public struct RecognizeResult: Encodable {
        public let blocks: [Block]
        public let text: String
        public let width: Int
        public let height: Int
        public let durationMs: Int
    }

    public func recognize(
        _ params: Params, completion: @escaping (Result<RecognizeResult, HelperError>) -> Void
    ) {
        let started = Clock.monotonicMs()
        guard params.path != nil || params.image != nil else {
            completion(.failure(.invalidParams("ocr.recognize requires path or image")))
            return
        }
        guard let cgImage = ImageEncoder.decode(path: params.path, base64: params.image) else {
            completion(.failure(.invalidParams("could not decode input image")))
            return
        }

        let request = VNRecognizeTextRequest()
        let accurate = (params.level ?? "accurate") != "fast"
        // https://developer.apple.com/documentation/vision/vnrecognizetextrequest/recognitionlevel
        request.recognitionLevel = accurate ? .accurate : .fast
        // https://developer.apple.com/documentation/vision/vnrecognizetextrequest/recognitionlanguages
        request.recognitionLanguages = params.languages ?? ["en-US"]
        // Language correction only helps the accurate path; it hurts fast-path latency.
        request.usesLanguageCorrection = accurate
        // Catch small UI text: minimum height relative to image height.
        // https://developer.apple.com/documentation/vision/vnrecognizetextrequest/minimumtextheight
        request.minimumTextHeight = 0.008

        let minConfidence = params.minConfidence ?? 0.3
        let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
        do {
            // perform(_:) is synchronous; the router already runs us on a
            // background queue.
            try handler.perform([request])
        } catch {
            completion(.failure(HelperError(
                kind: .internalError, code: "ocr_failed", message: error.localizedDescription)))
            return
        }

        var blocks: [Block] = []
        for observation in request.results ?? [] {
            // https://developer.apple.com/documentation/vision/vnrecognizedtextobservation
            guard let candidate = observation.topCandidates(1).first else { continue }
            guard Double(candidate.confidence) >= minConfidence else { continue }
            let bb = observation.boundingBox // normalized, bottom-left origin
            let topLeft = RectJSON(
                x: Double(bb.origin.x),
                y: 1.0 - Double(bb.origin.y) - Double(bb.height), // flip to top-left origin
                width: Double(bb.width),
                height: Double(bb.height))
            blocks.append(
                Block(
                    text: candidate.string,
                    confidence: Double(candidate.confidence),
                    boundingBox: topLeft))
        }

        let ordered = OCRSorter.sortIntoReadingOrder(blocks)
        completion(
            .success(
                RecognizeResult(
                    blocks: ordered,
                    text: OCRSorter.joinedText(ordered),
                    width: cgImage.width,
                    height: cgImage.height,
                    durationMs: Int((Clock.monotonicMs() - started).rounded()))))
    }
}

/// Pure reading-order logic (unit-tested): group blocks into visual lines by
/// vertical overlap, order lines top→bottom, order blocks in a line left→right.
public enum OCRSorter {
    /// Two blocks share a line when their vertical centers are within 60 % of
    /// the smaller block height.
    static func sameLine(_ a: OCRService.Block, _ b: OCRService.Block) -> Bool {
        let centerA = a.boundingBox.y + a.boundingBox.height / 2
        let centerB = b.boundingBox.y + b.boundingBox.height / 2
        let tolerance = 0.6 * min(a.boundingBox.height, b.boundingBox.height)
        return abs(centerA - centerB) <= max(tolerance, 0.004)
    }

    public static func sortIntoReadingOrder(_ blocks: [OCRService.Block]) -> [OCRService.Block] {
        guard blocks.count > 1 else { return blocks }
        // Seed order: top→bottom, then left→right.
        let seeded = blocks.sorted {
            if abs($0.boundingBox.y - $1.boundingBox.y) > 0.0001 {
                return $0.boundingBox.y < $1.boundingBox.y
            }
            return $0.boundingBox.x < $1.boundingBox.x
        }
        // Greedy line grouping over the seeded order.
        var lines: [[OCRService.Block]] = []
        for block in seeded {
            if var line = lines.last, let anchor = line.first, sameLine(anchor, block) {
                line.append(block)
                lines[lines.count - 1] = line
            } else {
                lines.append([block])
            }
        }
        return lines.flatMap { line in
            line.sorted { $0.boundingBox.x < $1.boundingBox.x }
        }
    }

    /// Join already-ordered blocks: spaces within a line, newlines between lines.
    public static func joinedText(_ ordered: [OCRService.Block]) -> String {
        guard !ordered.isEmpty else { return "" }
        var out = ""
        var previous: OCRService.Block?
        for block in ordered {
            if let previous {
                out += sameLine(previous, block) ? " " : "\n"
            }
            out += block.text
            previous = block
        }
        return out
    }
}
