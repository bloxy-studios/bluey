// swift-tools-version: 5.9
// BlueyHelper — native macOS sidecar for the Bluey Tauri app.
// Built by scripts/build-helper.sh into src-tauri/binaries/bluey-helper-<triple>.
import PackageDescription

let package = Package(
    name: "BlueyHelper",
    platforms: [
        // ScreenCaptureKit's SCScreenshotManager requires macOS 14.0
        // (https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager).
        // The app declares minimumSystemVersion 14.0, so the helper matches.
        .macOS(.v14)
    ],
    products: [
        .executable(name: "bluey-helper", targets: ["bluey-helper"]),
        .library(name: "BlueyHelperCore", targets: ["BlueyHelperCore"]),
    ],
    targets: [
        // All logic lives in the library target so it is unit-testable.
        .target(
            name: "BlueyHelperCore",
            path: "Sources/BlueyHelperCore",
            linkerSettings: [
                .linkedFramework("ScreenCaptureKit"),
                .linkedFramework("Vision"),
                .linkedFramework("AVFoundation"),
                .linkedFramework("Speech"),
                .linkedFramework("CoreAudio"),
                .linkedFramework("AudioToolbox"), // AudioUnitSetProperty for input-device selection
                .linkedFramework("ApplicationServices"), // AXUIElement*
                .linkedFramework("AppKit"),
                .linkedFramework("CoreMedia"),
                .linkedFramework("CoreImage"),
                .linkedFramework("ImageIO"),
                .linkedFramework("UniformTypeIdentifiers"),
                .linkedFramework("CoreGraphics"),
            ]
        ),
        // Thin executable: emits helper.ready, runs the stdio loop.
        .executableTarget(
            name: "bluey-helper",
            dependencies: ["BlueyHelperCore"],
            path: "Sources/bluey-helper"
        ),
        .testTarget(
            name: "BlueyHelperCoreTests",
            dependencies: ["BlueyHelperCore"],
            path: "Tests/BlueyHelperCoreTests"
        ),
    ],
    // Stay in Swift 5 language mode to avoid Swift 6 strict-concurrency friction;
    // shared state is protected with serial queues / locks instead.
    swiftLanguageVersions: [.v5]
)
