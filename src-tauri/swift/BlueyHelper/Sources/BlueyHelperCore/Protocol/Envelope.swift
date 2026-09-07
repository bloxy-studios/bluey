import Foundation

// MARK: - JSONValue

/// A dynamically typed JSON value. Requests carry `params` as `JSONValue` and
/// each handler decodes it into its own typed struct via `decode(_:)`.
public enum JSONValue: Codable, Equatable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() {
            self = .null
        } else if let b = try? c.decode(Bool.self) {
            self = .bool(b)
        } else if let n = try? c.decode(Double.self) {
            self = .number(n)
        } else if let s = try? c.decode(String.self) {
            self = .string(s)
        } else if let a = try? c.decode([JSONValue].self) {
            self = .array(a)
        } else if let o = try? c.decode([String: JSONValue].self) {
            self = .object(o)
        } else {
            throw DecodingError.dataCorruptedError(in: c, debugDescription: "unsupported JSON value")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n): try c.encode(n)
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }

    /// Re-encode this JSON fragment and decode it as a typed value.
    public func decode<T: Decodable>(_ type: T.Type) throws -> T {
        let data = try JSONCoding.encoder.encode(self)
        return try JSONCoding.decoder.decode(T.self, from: data)
    }
}

// MARK: - Shared coders

public enum JSONCoding {
    /// Keys in the protocol are camelCase already — no key strategy.
    /// sortedKeys keeps output deterministic (nice for tests and log diffing).
    public static let encoder: JSONEncoder = {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return e
    }()

    public static let decoder = JSONDecoder()
}

// MARK: - Envelopes (docs/HELPER_PROTOCOL.md "Envelope")

/// `{ "id": "r-42", "method": "capture.display", "params": { … } }`
public struct RequestEnvelope: Decodable {
    public let id: String
    public let method: String
    public let params: JSONValue?

    enum CodingKeys: String, CodingKey { case id, method, params }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        // Accept string ids (protocol default) but tolerate numeric ids.
        if let s = try? c.decode(String.self, forKey: .id) {
            id = s
        } else if let n = try? c.decode(Int64.self, forKey: .id) {
            id = String(n)
        } else {
            throw DecodingError.keyNotFound(
                CodingKeys.id,
                .init(codingPath: decoder.codingPath, debugDescription: "missing request id"))
        }
        method = try c.decode(String.self, forKey: .method)
        params = try c.decodeIfPresent(JSONValue.self, forKey: .params)
    }

    public init(id: String, method: String, params: JSONValue?) {
        self.id = id
        self.method = method
        self.params = params
    }
}

public struct SuccessResponse<T: Encodable>: Encodable {
    public let id: String
    public let result: T
    public init(id: String, result: T) {
        self.id = id
        self.result = result
    }
}

public struct ErrorResponse: Encodable {
    public let id: String
    public let error: HelperError
    public init(id: String, error: HelperError) {
        self.id = id
        self.error = error
    }
}

public struct EventEnvelope<T: Encodable>: Encodable {
    public let event: String
    public let data: T
    public init(event: String, data: T) {
        self.event = event
        self.data = data
    }
}

// MARK: - Errors

/// `error.kind` ∈ permission | capture | audio | transcription | not_supported |
/// internal | invalid_params (docs/HELPER_PROTOCOL.md).
public enum ErrorKind: String, Codable {
    case permission
    case capture
    case audio
    case transcription
    case notSupported = "not_supported"
    case internalError = "internal"
    case invalidParams = "invalid_params"
}

public struct HelperError: Error, Codable, Equatable {
    public let code: String
    public let message: String
    public let kind: ErrorKind
    public let details: [String: JSONValue]?

    public init(kind: ErrorKind, code: String, message: String, details: [String: JSONValue]? = nil) {
        self.kind = kind
        self.code = code
        self.message = message
        self.details = details
    }

    // Common factories --------------------------------------------------

    public static func permissionDenied(_ permission: String, message: String? = nil) -> HelperError {
        HelperError(
            kind: .permission,
            code: "permission_denied",
            message: message ?? "\(permission) permission not granted",
            details: ["permission": .string(permission)])
    }

    public static func invalidParams(_ message: String) -> HelperError {
        HelperError(kind: .invalidParams, code: "invalid_params", message: message)
    }

    public static func notSupported(_ code: String, _ message: String) -> HelperError {
        HelperError(kind: .notSupported, code: code, message: message)
    }

    public static func capture(_ code: String, _ message: String) -> HelperError {
        HelperError(kind: .capture, code: code, message: message)
    }

    public static func audio(_ code: String, _ message: String) -> HelperError {
        HelperError(kind: .audio, code: code, message: message)
    }

    public static func internalError(_ message: String) -> HelperError {
        HelperError(kind: .internalError, code: "internal_error", message: message)
    }
}

// MARK: - AnyEncodable

/// Type-erased Encodable so the router can pass heterogeneous results around.
public struct AnyEncodable: Encodable {
    private let encodeClosure: (Encoder) throws -> Void
    public init<T: Encodable>(_ value: T) {
        self.encodeClosure = value.encode(to:)
    }
    public func encode(to encoder: Encoder) throws {
        try encodeClosure(encoder)
    }
}

// MARK: - Shared geometry payloads

/// `{x,y,width,height}` used by windows, capture regions and AX bounds.
public struct RectJSON: Codable, Equatable {
    public var x: Double
    public var y: Double
    public var width: Double
    public var height: Double

    public init(x: Double, y: Double, width: Double, height: Double) {
        self.x = x
        self.y = y
        self.width = width
        self.height = height
    }

    public init(_ rect: CGRect) {
        self.init(
            x: Double(rect.origin.x), y: Double(rect.origin.y),
            width: Double(rect.size.width), height: Double(rect.size.height))
    }

    public var cgRect: CGRect { CGRect(x: x, y: y, width: width, height: height) }
}

public struct PointJSON: Codable, Equatable {
    public var x: Double
    public var y: Double
    public init(x: Double, y: Double) {
        self.x = x
        self.y = y
    }
}

public struct SizeJSON: Codable, Equatable {
    public var width: Double
    public var height: Double
    public init(width: Double, height: Double) {
        self.width = width
        self.height = height
    }
}
