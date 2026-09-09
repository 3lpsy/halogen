// swift-tools-version: 6.0
// Cross-build the shared UI test sources into a device test bundle on Linux.
import PackageDescription

let package = Package(
    name: "HalogenUITestsPkg",
    platforms: [.iOS(.v17)],
    targets: [
        // SwiftPM refuses a test-only package; one empty library anchors it.
        .target(name: "Anchor"),
        // v5 language mode matches the Xcode lane (no strict-concurrency).
        .testTarget(
            name: "HalogenUITests",
            dependencies: [],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
