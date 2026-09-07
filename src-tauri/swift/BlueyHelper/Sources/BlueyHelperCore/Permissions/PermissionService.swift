import AVFoundation
import ApplicationServices
import CoreGraphics
import Foundation
import Speech

/// TCC permission status + request. The helper is spawned by Bluey.app, so all
/// prompts and grants are attributed to the *responsible process* — the app
/// bundle (com.codewithabdul.bluey). Usage strings (NSMicrophoneUsageDescription,
/// NSSpeechRecognitionUsageDescription, …) live in the app's Info.plist.
public final class PermissionService {
    public init() {}

    public enum Kind: String, Codable {
        case screenRecording
        case microphone
        case accessibility
        case speechRecognition
    }

    /// Status ∈ granted | denied | not_determined | restricted | unknown.
    public enum Status: String, Codable {
        case granted
        case denied
        case notDetermined = "not_determined"
        case restricted
        case unknown
    }

    public struct StatusResult: Encodable {
        public let screenRecording: Status
        public let microphone: Status
        public let accessibility: Status
        public let speechRecognition: Status
    }

    public struct RequestParams: Decodable {
        public let kind: Kind
    }

    public struct RequestResult: Encodable {
        public let kind: Kind
        public let status: Status
    }

    // MARK: Status

    public func status() -> StatusResult {
        StatusResult(
            screenRecording: screenRecordingStatus(),
            microphone: microphoneStatus(),
            accessibility: accessibilityStatus(),
            speechRecognition: speechStatus())
    }

    /// CGPreflightScreenCaptureAccess() → Bool (macOS 10.15+); there is no public
    /// tri-state API for Screen Recording, so false maps to `denied`.
    /// https://developer.apple.com/documentation/coregraphics/cgpreflightscreencaptureaccess()
    private func screenRecordingStatus() -> Status {
        CGPreflightScreenCaptureAccess() ? .granted : .denied
    }

    /// https://developer.apple.com/documentation/avfoundation/avcapturedevice/authorizationstatus(for:)
    private func microphoneStatus() -> Status {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return .granted
        case .denied: return .denied
        case .notDetermined: return .notDetermined
        case .restricted: return .restricted
        @unknown default: return .unknown
        }
    }

    /// AXIsProcessTrusted() → Bool; no tri-state, false maps to `denied`.
    /// https://developer.apple.com/documentation/applicationservices/1460720-axisprocesstrusted
    private func accessibilityStatus() -> Status {
        AXIsProcessTrusted() ? .granted : .denied
    }

    /// https://developer.apple.com/documentation/speech/sfspeechrecognizer/authorizationstatus()
    private func speechStatus() -> Status {
        switch SFSpeechRecognizer.authorizationStatus() {
        case .authorized: return .granted
        case .denied: return .denied
        case .notDetermined: return .notDetermined
        case .restricted: return .restricted
        @unknown default: return .unknown
        }
    }

    // MARK: Request

    public func request(kind: Kind, completion: @escaping (RequestResult) -> Void) {
        switch kind {
        case .screenRecording:
            // Shows the system dialog once (per TCC rules) and returns the
            // *current* grant synchronously; a fresh grant requires app relaunch
            // to take effect for ScreenCaptureKit.
            // https://developer.apple.com/documentation/coregraphics/cgrequestscreencaptureaccess()
            let granted = CGRequestScreenCaptureAccess()
            completion(RequestResult(kind: kind, status: granted ? .granted : .denied))

        case .microphone:
            if AVCaptureDevice.authorizationStatus(for: .audio) == .notDetermined {
                AVCaptureDevice.requestAccess(for: .audio) { [weak self] _ in
                    completion(RequestResult(kind: kind, status: self?.microphoneStatus() ?? .unknown))
                }
            } else {
                completion(RequestResult(kind: kind, status: microphoneStatus()))
            }

        case .accessibility:
            // kAXTrustedCheckOptionPrompt is imported as Unmanaged<CFString>.
            // https://developer.apple.com/documentation/applicationservices/kaxtrustedcheckoptionprompt
            let key = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
            let options = [key: true] as CFDictionary
            let trusted = AXIsProcessTrustedWithOptions(options)
            completion(RequestResult(kind: kind, status: trusted ? .granted : .denied))

        case .speechRecognition:
            if SFSpeechRecognizer.authorizationStatus() == .notDetermined {
                // https://developer.apple.com/documentation/speech/sfspeechrecognizer/requestauthorization(_:)
                SFSpeechRecognizer.requestAuthorization { [weak self] _ in
                    completion(RequestResult(kind: kind, status: self?.speechStatus() ?? .unknown))
                }
            } else {
                completion(RequestResult(kind: kind, status: speechStatus()))
            }
        }
    }
}
