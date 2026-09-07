import BlueyHelperCore
import Foundation

// bluey-helper — Tauri sidecar entry point.
// Protocol: docs/HELPER_PROTOCOL.md (JSON Lines over stdio).
// The app object owns all services; keep a strong reference for process lifetime.
let app = HelperApp()
app.run()

// Park the main thread and service the main queue forever
// (readabilityHandler / dispatch sources / AX+NSWorkspace main-queue hops).
// https://developer.apple.com/documentation/dispatch/dispatchmain()
dispatchMain()
