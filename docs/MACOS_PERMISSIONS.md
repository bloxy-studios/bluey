# macOS Permissions

Bluey requires **macOS 14 (Sonoma) or newer** on Apple Silicon or Intel. This is driven by the
APIs actually used: `SCScreenshotManager` (macOS 14), `SCStream` audio capture (macOS 13),
Vision text recognition, Accessibility, AVAudioEngine and `SFSpeechRecognizer` on-device
recognition. Older versions are not supported and Bluey does not pretend otherwise.

## Permissions and why Bluey asks

| Permission | What Bluey needs | Why | What it can access | How it's checked |
|---|---|---|---|---|
| **Screen Recording** | Capture the current display/window/region on demand (⌘↵) and optional low-frequency observation | To read the question, code or document you are looking at | Pixels of the selected display/window (Bluey's own windows are excluded); nothing is captured without your action unless Smart observation is enabled | `CGPreflightScreenCaptureAccess` (Rust); request via `CGRequestScreenCaptureAccess` |
| **Microphone** | Capture your voice while a session is listening | Live transcript of what you say | Microphone audio only while `● Listening`; never stored by default | `AVCaptureDevice.authorizationStatus(for: .audio)` (helper) |
| **Speech Recognition** | Apple Speech speech-to-text | Transcribe without a cloud provider: used when no Google AI Studio key is connected, when Apple Speech is selected, or with Privacy → Cloud AI off (the default, Gemini Live, streams the audio to Google while listening) | The same audio, on device when the language has an on-device model, otherwise on Apple's servers (Bluey shows a notice) | `SFSpeechRecognizer.authorizationStatus()` (helper) |
| **Accessibility** | Read the focused window's accessibility tree | Semantic understanding of the UI (focused field, selected text, buttons) that complements OCR | Text and roles of visible elements in the frontmost app; bounded depth/size | `AXIsProcessTrusted` (Rust); prompt via `AXIsProcessTrustedWithOptions` (helper) |
| **Notifications** | Local notifications for prepared suggestions and session events | Optional; you can leave it off | Nothing beyond displaying notifications | `tauri-plugin-notification` |
| **System audio** | Part of Screen Recording — `SCStream` audio capture of other apps | Transcribe the other side of a call | Audio of other applications, excluding Bluey itself | Requires Screen Recording |

System audio capture is only available when Screen Recording is granted (this is how macOS
gates ScreenCaptureKit audio). If Screen Recording is denied, Bluey can still transcribe your
microphone.

## Permission flow
1. **First run wizard** explains each permission (what / why / access) before requesting it,
   one screen per permission, with *Continue* and *Open System Settings*.
2. `PermissionState` is refreshed at launch, after each request, every 30 s while a session is
   active, and — while the onboarding permissions step or Settings → Permissions is open — when
   that window regains focus and every 2 s (`useLivePermissions`). Revocation stops the dependent subsystem (audio
   session, observation) and surfaces a repair flow.
3. Deep links used for *Open System Settings*:
   * Screen Recording: `x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture`
   * Microphone: `…?Privacy_Microphone`
   * Accessibility: `…?Privacy_Accessibility`
   * Speech Recognition: `…?Privacy_SpeechRecognition`
   * Notifications: `x-apple.systempreferences:com.apple.preference.notifications`

4. **After an update** (MAC-001): builds without a Developer ID get a new code identity with
   every update, and macOS stops honouring the grants of the previous version. The first launch
   of a new version compares the current state with the persisted snapshot of the last-known
   grants; lost ones fill `PermissionState.lostAfterUpdate`, Settings opens on *Permissions*, and a
   card names them, explains that unsigned updates reset permissions and Keychain approvals, and
   offers *Open System Settings* for each (remove Bluey with "–" and add it again if it is listed).
   See [UPDATES.md](UPDATES.md#unsigned-builds-reset-macos-permissions).

## TCC attribution
The native helper is a child process of Bluey.app inside the bundle (`Contents/MacOS/`). macOS
attributes TCC checks of child processes to the *responsible* application, so prompts name
"Bluey" and grants apply to the app. Usage strings live in the app's `Info.plist`
(`NSMicrophoneUsageDescription`, `NSSpeechRecognitionUsageDescription`,
`NSAccessibilityUsageDescription`, `NSScreenCaptureUsageDescription`).

## Known platform limits
* Screen Recording changes take effect only after the app restarts in some macOS versions;
  Bluey detects `denied → granted` transitions and offers *Restart Bluey*.
* Notifications authorization requires a signed, bundled app (`.app`); it is unavailable when
  running the raw dev binary.
* On-device speech models are downloaded per locale by macOS; unsupported locales fall back to
  server-based recognition or the configured cloud transcription provider.
* Accessibility trees of some apps (Electron without AX enabled, games) are empty; Bluey falls
  back to OCR.
