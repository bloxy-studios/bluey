import Foundation

/// Newline-delimited JSON transport over stdio (docs/HELPER_PROTOCOL.md "Transport").
///
/// * Reader: `FileHandle.standardInput.readabilityHandler` feeds a serial parse
///   queue; complete lines are handed to `onLine`. Empty `availableData` means
///   EOF → `onEOF` fires once (Rust closed stdin → helper must exit 0).
///   https://developer.apple.com/documentation/foundation/filehandle/1399558-readabilityhandler
/// * Writer: a serial queue guarantees whole-line atomicity on stdout.
///   `FileHandle.write(contentsOf:)` performs an unbuffered write(2), so every
///   line is flushed as soon as the block runs.
public final class JSONLinesIO {
    public var onLine: ((Data) -> Void)?
    public var onEOF: (() -> Void)?

    private let writeQueue = DispatchQueue(label: "com.codewithabdul.bluey.helper.stdout")
    private let readQueue = DispatchQueue(label: "com.codewithabdul.bluey.helper.stdin")
    private var readBuffer = Data()
    private var eofDelivered = false
    private let stdout = FileHandle.standardOutput
    private let stdin = FileHandle.standardInput

    public init() {}

    // MARK: Reading

    public func start() {
        stdin.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard let self else { return }
            self.readQueue.async {
                if data.isEmpty {
                    self.deliverEOF()
                } else {
                    self.consume(data)
                }
            }
        }
    }

    public func stop() {
        stdin.readabilityHandler = nil
    }

    private func consume(_ data: Data) {
        readBuffer.append(data)
        // Split on '\n' (0x0A); tolerate trailing '\r'.
        while let nl = readBuffer.firstIndex(of: 0x0A) {
            var line = readBuffer.subdata(in: readBuffer.startIndex..<nl)
            readBuffer.removeSubrange(readBuffer.startIndex...nl)
            if line.last == 0x0D { line.removeLast() }
            if line.isEmpty { continue }
            onLine?(line)
        }
    }

    private func deliverEOF() {
        guard !eofDelivered else { return }
        eofDelivered = true
        stdin.readabilityHandler = nil
        onEOF?()
    }

    // MARK: Writing

    private func writeLine(_ data: Data) {
        let handle = stdout
        writeQueue.async {
            var out = data
            out.append(0x0A)
            do {
                try handle.write(contentsOf: out)
            } catch {
                // stdout gone → parent died; nothing sensible left to do.
                Log.shared.error("stdout write failed: \(error.localizedDescription)")
            }
        }
    }

    public func sendResult<T: Encodable>(id: String, result: T) {
        encodeAndWrite(SuccessResponse(id: id, result: result), context: "response \(id)")
    }

    public func sendError(id: String, error: HelperError) {
        encodeAndWrite(ErrorResponse(id: id, error: error), context: "error \(id)")
    }

    public func emit<T: Encodable>(event: String, data: T) {
        encodeAndWrite(EventEnvelope(event: event, data: data), context: "event \(event)")
    }

    private func encodeAndWrite<T: Encodable>(_ value: T, context: String) {
        do {
            let data = try JSONCoding.encoder.encode(value)
            writeLine(data)
        } catch {
            Log.shared.error("failed to encode \(context): \(error.localizedDescription)")
        }
    }

    /// Drain queued writes (used before exiting on shutdown/SIGTERM).
    public func flush() {
        writeQueue.sync {}
    }
}
