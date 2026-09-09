// swift-tools-version: 6.0
// SwiftPM builds the app on Linux; XcodeGen owns simulator tests and releases.
import PackageDescription

let package = Package(
    name: "Halogen",
    platforms: [.iOS(.v17)],
    products: [
        // Exactly one library product: xtool wraps it in a generated app stub.
        .library(name: "Halogen", targets: ["Halogen"])
    ],
    targets: [
        .target(
            name: "Halogen",
            dependencies: ["halogen_mobileFFI"],
            path: ".",
            // Non-Swift files inside the source dirs; the Xcode lane consumes
            // them (assets/privacy at build, modulemap via header search).
            exclude: [
                "Tests", "UITests", "UITestsPkg", "FFI", "Rust", "build",
                "DevInfo.plist", "TestInventory.json", "ScreenshotInventory.json", "project.yml", "xtool.yml",
                "Halogen/Assets.xcassets",
                "Halogen/PrivacyInfo.xcprivacy",
                "Generated/halogen_mobileFFI.modulemap",
            ],
            sources: ["Halogen", "Generated"],
            // Match the Xcode lane (SWIFT_VERSION 5.10): v6 strict
            // concurrency would fail sources CI compiles fine.
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        // Modulemap over the staged uniffi header (ios/Rust/, `just ios-core`).
        // The .pc (generated into ios/Rust/) carries the -L/-l for the staticlib,
        // keeping the manifest free of unsafeFlags so xtool's wrapper can depend
        // on this package.
        .systemLibrary(
            name: "halogen_mobileFFI",
            path: "FFI",
            pkgConfig: "halogen_mobile"
        ),
    ]
)
