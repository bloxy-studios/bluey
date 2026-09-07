import XCTest

@testable import BlueyHelperCore

final class EnvelopeTests: XCTestCase {
    func testDecodeRequestWithParams() throws {
        let line = #"{"id":"r-42","method":"capture.display","params":{"displayId":"69733382","quality":0.5,"inline":true}}"#
        let request = try JSONCoding.decoder.decode(
            RequestEnvelope.self, from: Data(line.utf8))
        XCTAssertEqual(request.id, "r-42")
        XCTAssertEqual(request.method, "capture.display")
        let params = try XCTUnwrap(request.params).decode(CaptureParams.self)
        XCTAssertEqual(params.displayId, "69733382")
        XCTAssertEqual(params.quality, 0.5)
        XCTAssertEqual(params.inline, true)
        // Documented defaults kick in for the rest.
        XCTAssertEqual(params.resolvedFormat, .jpeg)
        XCTAssertEqual(params.resolvedMaxDimension, 1600)
        XCTAssertTrue(params.resolvedChangeDetection)
        XCTAssertTrue(params.resolvedExcludeSelf)
    }

    func testDecodeRequestWithoutParams() throws {
        let line = #"{"id":"1","method":"helper.ping"}"#
        let request = try JSONCoding.decoder.decode(RequestEnvelope.self, from: Data(line.utf8))
        XCTAssertEqual(request.method, "helper.ping")
        XCTAssertNil(request.params)
    }

    func testDecodeRequestWithNumericIdToleration() throws {
        let line = #"{"id":7,"method":"helper.ping"}"#
        let request = try JSONCoding.decoder.decode(RequestEnvelope.self, from: Data(line.utf8))
        XCTAssertEqual(request.id, "7")
    }

    func testMissingIdFails() {
        let line = #"{"method":"helper.ping"}"#
        XCTAssertThrowsError(
            try JSONCoding.decoder.decode(RequestEnvelope.self, from: Data(line.utf8)))
    }

    func testEncodeSuccessResponse() throws {
        struct Pong: Encodable {
            let pong: Bool
        }
        let data = try JSONCoding.encoder.encode(SuccessResponse(id: "r-1", result: Pong(pong: true)))
        let json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(json["id"] as? String, "r-1")
        let result = try XCTUnwrap(json["result"] as? [String: Any])
        XCTAssertEqual(result["pong"] as? Bool, true)
    }

    func testEncodeErrorResponseShape() throws {
        let error = HelperError.permissionDenied("screenRecording", message: "Screen Recording not granted")
        let data = try JSONCoding.encoder.encode(ErrorResponse(id: "r-9", error: error))
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(json["id"] as? String, "r-9")
        let payload = try XCTUnwrap(json["error"] as? [String: Any])
        XCTAssertEqual(payload["code"] as? String, "permission_denied")
        XCTAssertEqual(payload["kind"] as? String, "permission")
        XCTAssertEqual(payload["message"] as? String, "Screen Recording not granted")
        let details = try XCTUnwrap(payload["details"] as? [String: Any])
        XCTAssertEqual(details["permission"] as? String, "screenRecording")
    }

    func testEncodeEventEnvelope() throws {
        let event = EventEnvelope(
            event: "screen.changed",
            data: ScreenChangedEvent(
                hash: "00000000deadbeef", delta: 0.125, displayId: "1",
                at: "2026-09-07T12:00:00.000Z"))
        let data = try JSONCoding.encoder.encode(event)
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(json["event"] as? String, "screen.changed")
        XCTAssertNil(json["id"], "events must not carry an id")
        let payload = try XCTUnwrap(json["data"] as? [String: Any])
        XCTAssertEqual(payload["hash"] as? String, "00000000deadbeef")
        XCTAssertEqual(payload["delta"] as? Double, 0.125)
    }

    func testErrorKindRawValues() {
        XCTAssertEqual(ErrorKind.notSupported.rawValue, "not_supported")
        XCTAssertEqual(ErrorKind.internalError.rawValue, "internal")
        XCTAssertEqual(ErrorKind.invalidParams.rawValue, "invalid_params")
    }

    func testJSONValueRoundTrip() throws {
        let line = #"{"a":[1,2.5,"x",true,null],"b":{"c":"d"}}"#
        let value = try JSONCoding.decoder.decode(JSONValue.self, from: Data(line.utf8))
        guard case .object(let obj) = value else {
            return XCTFail("expected object")
        }
        guard case .array(let arr)? = obj["a"] else {
            return XCTFail("expected array")
        }
        XCTAssertEqual(arr.count, 5)
        XCTAssertEqual(arr[0], .number(1))
        XCTAssertEqual(arr[2], .string("x"))
        XCTAssertEqual(arr[3], .bool(true))
        XCTAssertEqual(arr[4], .null)
        // Round trip: encode → decode → equal.
        let reencoded = try JSONCoding.encoder.encode(value)
        let decoded = try JSONCoding.decoder.decode(JSONValue.self, from: reencoded)
        XCTAssertEqual(decoded, value)
    }

    func testAudioStartParamsDecoding() throws {
        let line = """
            {"microphone":{"enabled":true,"deviceId":"uid-1"},"systemAudio":{"enabled":true},
             "sampleRate":16000,"vad":{"enabled":true,"sensitivity":"high"},"emitPcm":false,
             "chunkMs":200,
             "transcription":{"enabled":true,"locale":"en-US","onDevice":true,"sources":["microphone","system"]},
             "levels":{"enabled":true,"intervalMs":100}}
            """.replacingOccurrences(of: "\n", with: "")
        let params = try JSONCoding.decoder.decode(AudioStartParams.self, from: Data(line.utf8))
        XCTAssertEqual(params.microphone?.enabled, true)
        XCTAssertEqual(params.microphone?.deviceId, "uid-1")
        XCTAssertEqual(params.vad?.sensitivity, .high)
        XCTAssertEqual(params.transcription?.sources, ["microphone", "system"])
        XCTAssertEqual(params.levels?.intervalMs, 100)
    }
}
