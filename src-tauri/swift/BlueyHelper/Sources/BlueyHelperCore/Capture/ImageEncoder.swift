import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// CGImage encode/decode/downscale + 9×8 luminance grid extraction for dHash.
public enum ImageFormat: String, Codable {
    case jpeg
    case png

    public var mimeType: String {
        switch self {
        case .jpeg: return "image/jpeg"
        case .png: return "image/png"
        }
    }

    public var fileExtension: String {
        switch self {
        case .jpeg: return "jpg"
        case .png: return "png"
        }
    }

    /// https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct
    var utType: UTType {
        switch self {
        case .jpeg: return .jpeg
        case .png: return .png
        }
    }
}

public enum ImageEncoder {
    /// Downscale so the longest side is ≤ maxDimension (no-op when already small).
    public static func downscale(_ image: CGImage, maxDimension: Int) -> CGImage {
        let w = image.width
        let h = image.height
        let longest = max(w, h)
        guard maxDimension > 0, longest > maxDimension else { return image }
        let scale = Double(maxDimension) / Double(longest)
        let newW = max(1, Int((Double(w) * scale).rounded()))
        let newH = max(1, Int((Double(h) * scale).rounded()))
        guard
            let ctx = CGContext(
                data: nil, width: newW, height: newH,
                bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return image }
        ctx.interpolationQuality = .medium
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: newW, height: newH))
        return ctx.makeImage() ?? image
    }

    /// Encode via CGImageDestination.
    /// https://developer.apple.com/documentation/imageio/cgimagedestinationcreatewithdata(_:_:_:_:)
    public static func encode(_ image: CGImage, format: ImageFormat, quality: Double) -> Data? {
        let data = NSMutableData()
        guard
            let dest = CGImageDestinationCreateWithData(
                data as CFMutableData, format.utType.identifier as CFString, 1, nil)
        else { return nil }
        var options: [CFString: Any] = [:]
        if format == .jpeg {
            options[kCGImageDestinationLossyCompressionQuality] = max(0.05, min(1.0, quality))
        }
        CGImageDestinationAddImage(dest, image, options as CFDictionary)
        guard CGImageDestinationFinalize(dest) else { return nil }
        return data as Data
    }

    /// Draw the image into a 9×8 8-bit grayscale bitmap and return the 72
    /// luminance values (0…255) row-major, ready for DHash.compute.
    public static func grayGrid(_ image: CGImage) -> [Double]? {
        let w = DHash.gridWidth
        let h = DHash.gridHeight
        guard
            let ctx = CGContext(
                data: nil, width: w, height: h,
                bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceGray(),
                bitmapInfo: CGImageAlphaInfo.none.rawValue)
        else { return nil }
        ctx.interpolationQuality = .medium
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
        guard let base = ctx.data else { return nil }
        let stride = ctx.bytesPerRow
        var values = [Double](repeating: 0, count: w * h)
        let bytes = base.assumingMemoryBound(to: UInt8.self)
        // CGContext rows are bottom-up relative to CG coords, but for hashing
        // orientation only needs to be *consistent*, so read rows as stored.
        for row in 0..<h {
            for col in 0..<w {
                values[row * w + col] = Double(bytes[row * stride + col])
            }
        }
        return values
    }

    /// Decode an image from a temp-file path or inline base64 (OCR input).
    public static func decode(path: String?, base64: String?) -> CGImage? {
        var source: CGImageSource?
        if let path {
            let url = URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
            source = CGImageSourceCreateWithURL(url as CFURL, nil)
        } else if let base64, let data = Data(base64Encoded: base64) {
            source = CGImageSourceCreateWithData(data as CFData, nil)
        }
        guard let source else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }
}
