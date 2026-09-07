import Foundation
import Speech

/// Wires services to the protocol router and owns process lifecycle
/// (helper.ready, EOF, SIGTERM, helper.shutdown).
public final class HelperApp {
    public static let version = "0.1.0"
    public static let protocolVersion = 1

    private let io = JSONLinesIO()
    private let router: Router
    private let startedAtMs = Clock.monotonicMs()

    private let permissions = PermissionService()
    private let displays = DisplayService()
    private let windows = WindowService()
    private let tempFrames = TempFrames()
    private let changeDetector = ChangeDetector()
    private let capture: ScreenCaptureService
    private let observer: ScreenObserver
    private let ocr = OCRService()
    private let ax = AXSnapshotService()
    private let audioDevices = AudioDeviceService()
    private let audio: AudioSession
    private var sigtermSource: DispatchSourceSignal?

    public init() {
        router = Router(io: io)
        capture = ScreenCaptureService(tempFrames: tempFrames, changeDetector: changeDetector)
        let io = self.io
        let emit: (String, AnyEncodable) -> Void = { event, data in
            io.emit(event: event, data: data)
        }
        observer = ScreenObserver(emit: emit)
        audio = AudioSession(deviceService: audioDevices, emit: emit)
        registerHandlers()
    }

    // MARK: lifecycle

    public func run() {
        tempFrames.cleanupStale()
        installSignalHandlers()

        io.onLine = { [weak self] line in
            self?.router.handle(line: line)
        }
        io.onEOF = { [weak self] in
            // Protocol: on stdin EOF stop everything and exit 0.
            Log.shared.info("stdin EOF -> shutting down")
            self?.shutdown(exitCode: 0)
        }

        struct ReadyEvent: Encodable {
            let version: String
            let protocolVersion: Int
            enum CodingKeys: String, CodingKey {
                case version
                case protocolVersion = "protocol"
            }
        }
        // Queue helper.ready BEFORE the reader starts so it is guaranteed to be
        // the first stdout line even if requests are already waiting in stdin
        // (the writer queue serializes; docs/HELPER_PROTOCOL.md "Lifecycle").
        io.emit(
            event: "helper.ready",
            data: ReadyEvent(version: Self.version, protocolVersion: Self.protocolVersion))
        io.start()
        Log.shared.info("bluey-helper \(Self.version) ready (pid \(getpid()), parent \(getppid()))")
    }

    private func installSignalHandlers() {
        // Handle SIGTERM via a dispatch source (safe context, unlike C signal handlers).
        signal(SIGTERM, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        source.setEventHandler { [weak self] in
            Log.shared.info("SIGTERM -> shutting down")
            self?.shutdown(exitCode: 0)
        }
        source.resume()
        sigtermSource = source
        // SIGPIPE would kill the process when the parent dies mid-write.
        signal(SIGPIPE, SIG_IGN)
    }

    private func shutdown(exitCode: Int32) {
        let io = self.io
        observer.stop { _ in }
        audio.stop { _ in }
        audioDevices.stopListening()
        io.stop()
        // Give in-flight stop paths a beat, drain stdout, exit.
        DispatchQueue.global().asyncAfter(deadline: .now() + .milliseconds(150)) {
            io.flush()
            exit(exitCode)
        }
    }

    // MARK: handler registration

    private func registerHandlers() {
        registerHelperHandlers()
        registerPermissionHandlers()
        registerScreenHandlers()
        registerAudioHandlers()
    }

    private func registerHelperHandlers() {
        let startedAtMs = self.startedAtMs

        router.register("helper.ping", inline: true) { _, respond in
            struct Pong: Encodable {
                let pong: Bool
                let uptimeMs: Int
            }
            respond(
                .success(
                    AnyEncodable(Pong(pong: true, uptimeMs: Int(Clock.monotonicMs() - startedAtMs)))))
        }

        router.register("helper.version", inline: true) { _, respond in
            struct Version: Encodable {
                let version: String
                let protocolVersion: Int
                let macos: String
                let arch: String
                let capabilities: [String]
                enum CodingKeys: String, CodingKey {
                    case version, macos, arch, capabilities
                    case protocolVersion = "protocol"
                }
            }
            let osv = ProcessInfo.processInfo.operatingSystemVersion
            #if arch(arm64)
                let arch = "arm64"
            #else
                let arch = "x86_64"
            #endif
            var capabilities = ["capture", "ocr", "accessibility", "audio.microphone", "audio.system"]
            // https://developer.apple.com/documentation/speech/sfspeechrecognizer/supportsondevicerecognition
            if SFSpeechRecognizer(locale: Locale(identifier: "en-US"))?.supportsOnDeviceRecognition
                == true
            {
                capabilities.append("speech.onDevice")
            }
            var macos = "\(osv.majorVersion).\(osv.minorVersion)"
            if osv.patchVersion > 0 { macos += ".\(osv.patchVersion)" }
            respond(
                .success(
                    AnyEncodable(
                        Version(
                            version: HelperApp.version,
                            protocolVersion: HelperApp.protocolVersion,
                            macos: macos, arch: arch, capabilities: capabilities))))
        }

        router.register("helper.shutdown", inline: true) { [weak self] _, respond in
            respond(.success(AnyEncodable(OkResult())))
            DispatchQueue.global().asyncAfter(deadline: .now() + .milliseconds(50)) {
                self?.shutdown(exitCode: 0)
            }
        }
    }

    private func registerPermissionHandlers() {
        let permissions = self.permissions

        router.register("permissions.status") { _, respond in
            respond(.success(AnyEncodable(permissions.status())))
        }
        router.register("permissions.request", params: PermissionService.RequestParams.self) {
            params, respond in
            permissions.request(kind: params.kind) { result in
                respond(.success(AnyEncodable(result)))
            }
        }
    }

    private func registerScreenHandlers() {
        let displays = self.displays
        let windows = self.windows
        let capture = self.capture
        let observer = self.observer
        let tempFrames = self.tempFrames
        let ocr = self.ocr
        let ax = self.ax

        router.register("displays.list") { _, respond in
            displays.list { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("windows.list") { raw, respond in
            let params = try? raw?.decode(WindowService.Params.self)
            windows.list(params: params) { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("app.frontmost") { _, respond in
            FrontmostAppService.frontmost { respond($0.map { AnyEncodable($0) }) }
        }

        router.register("capture.display") { raw, respond in
            capture.captureDisplay(HelperApp.captureParams(raw)) { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("capture.window") { raw, respond in
            capture.captureWindow(HelperApp.captureParams(raw)) { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("capture.region") { raw, respond in
            capture.captureRegion(HelperApp.captureParams(raw)) { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("capture.activeWindow") { raw, respond in
            capture.captureActiveWindow(HelperApp.captureParams(raw)) { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("capture.discard", params: TempFrames.DiscardParams.self) { params, respond in
            respond(tempFrames.discard(path: params.path).map { AnyEncodable($0) })
        }

        router.register("observe.start") { raw, respond in
            let params = (try? raw?.decode(ObserveParams.self)) ?? ObserveParams()
            observer.start(params) { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("observe.stop") { _, respond in
            observer.stop { respond($0.map { AnyEncodable($0) }) }
        }

        router.register("ocr.recognize", params: OCRService.Params.self) { params, respond in
            ocr.recognize(params) { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("accessibility.snapshot") { raw, respond in
            let params = try? raw?.decode(AXSnapshotService.Params.self)
            ax.snapshot(params) { respond($0.map { AnyEncodable($0) }) }
        }
    }

    private func registerAudioHandlers() {
        let audio = self.audio
        let audioDevices = self.audioDevices

        router.register("audio.devices") { _, respond in
            respond(
                .success(
                    AnyEncodable(
                        AudioDeviceService.ListResult(devices: audioDevices.listInputDevices()))))
        }
        router.register("audio.start", params: AudioStartParams.self) { params, respond in
            audio.start(params) { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("audio.stop") { _, respond in
            audio.stop { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("audio.pause") { _, respond in
            audio.pause { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("audio.resume") { _, respond in
            audio.resume { respond($0.map { AnyEncodable($0) }) }
        }
        router.register("audio.testMicrophone") { raw, respond in
            let params =
                (try? raw?.decode(AudioSession.TestMicrophoneParams.self))
                ?? AudioSession.TestMicrophoneParams()
            audio.testMicrophone(params) { respond($0.map { AnyEncodable($0) }) }
        }
    }

    private static func captureParams(_ raw: JSONValue?) -> CaptureParams {
        (try? raw?.decode(CaptureParams.self)) ?? CaptureParams()
    }
}
