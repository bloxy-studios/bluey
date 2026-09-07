import Foundation

/// Method → handler dispatch.
///
/// Handlers run on a **concurrent** background queue so a slow capture can
/// never head-of-line-block anything else. `helper.ping` (and other handlers
/// registered with `inline: true`) are answered directly on the stdin parse
/// queue, guaranteeing the < 50 ms budget of docs/HELPER_PROTOCOL.md even when
/// all worker threads are busy.
///
/// Rust owns request timeouts; the helper never times out on its own.
public final class Router {
    public typealias Respond = (Result<AnyEncodable, HelperError>) -> Void
    public typealias Handler = (JSONValue?, @escaping Respond) -> Void

    private struct Entry {
        let inline: Bool
        let handler: Handler
    }

    private var entries: [String: Entry] = [:]
    private let workQueue = DispatchQueue(
        label: "com.codewithabdul.bluey.helper.work",
        qos: .userInitiated,
        attributes: .concurrent)
    private let io: JSONLinesIO

    public init(io: JSONLinesIO) {
        self.io = io
    }

    /// Register before `JSONLinesIO.start()`; registration is not synchronized.
    public func register(_ method: String, inline: Bool = false, handler: @escaping Handler) {
        entries[method] = Entry(inline: inline, handler: handler)
    }

    /// Convenience for handlers with typed params.
    public func register<P: Decodable>(
        _ method: String,
        params type: P.Type,
        handler: @escaping (P, @escaping Respond) -> Void
    ) {
        register(method) { raw, respond in
            guard let raw else {
                respond(.failure(.invalidParams("\(method) requires params")))
                return
            }
            do {
                let typed = try raw.decode(P.self)
                handler(typed, respond)
            } catch {
                respond(.failure(.invalidParams("bad params for \(method): \(error)")))
            }
        }
    }

    public func handle(line: Data) {
        let request: RequestEnvelope
        do {
            request = try JSONCoding.decoder.decode(RequestEnvelope.self, from: line)
        } catch {
            // No id to correlate → log only (stderr), per protocol stdout stays clean.
            Log.shared.error("unparseable request line: \(error.localizedDescription)")
            return
        }

        guard let entry = entries[request.method] else {
            io.sendError(
                id: request.id,
                error: HelperError(
                    kind: .notSupported,
                    code: "method_not_found",
                    message: "unknown method \(request.method)"))
            return
        }

        let io = self.io
        let id = request.id
        var responded = false
        let respondOnce: Respond = { result in
            // Guard against double-respond bugs in handlers.
            guard !responded else { return }
            responded = true
            switch result {
            case .success(let value): io.sendResult(id: id, result: value)
            case .failure(let error): io.sendError(id: id, error: error)
            }
        }

        if entry.inline {
            entry.handler(request.params, respondOnce)
        } else {
            workQueue.async {
                entry.handler(request.params, respondOnce)
            }
        }
    }
}

/// Empty-params marker for methods that take none.
public struct NoParams: Decodable {
    public init() {}
}

/// `{ "ok": true }` results.
public struct OkResult: Encodable {
    public let ok: Bool
    public init(ok: Bool = true) { self.ok = ok }
}
