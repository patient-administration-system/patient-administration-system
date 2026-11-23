// swift-tools-version: 6.0
// The patient mobile app for Apple platforms (spec/mobile-apps/index.md). The logic is a Swift package so `swift test` proves it
// without a simulator; App/ is the thin iOS app that hosts PASMobileUI.
//
// Lily Design System for Swift is not published yet, so it is resolved from a local checkout: LILY_SWIFT_PATH, or the
// default layout ~/git/lilydesignsystem/lily-design-system/lily-design-system-swift beside this repository's parent folder.
import Foundation
import PackageDescription

let lilyPath = ProcessInfo.processInfo.environment["LILY_SWIFT_PATH"]
    ?? "../../../lilydesignsystem/lily-design-system/lily-design-system-swift"

let package = Package(
    name: "PASMobile",
    defaultLocalization: "en",
    platforms: [.iOS(.v18), .macOS(.v15)],
    products: [
        .library(name: "PASMobileCore", targets: ["PASMobileCore"]),
        .library(name: "PASMobileUI", targets: ["PASMobileUI"]),
    ],
    dependencies: [
        .package(name: "LilyDesignSystem", path: lilyPath),
    ],
    targets: [
        .target(name: "PASMobileCore", resources: [.process("Resources")]),
        .target(
            name: "PASMobileUI",
            dependencies: [
                "PASMobileCore",
                .product(name: "LilyUI", package: "LilyDesignSystem"),
                .product(name: "LilyThemes", package: "LilyDesignSystem"),
            ]
        ),
        .testTarget(name: "PASMobileCoreTests", dependencies: ["PASMobileCore"], resources: [.copy("Fixtures")]),
    ],
    swiftLanguageModes: [.v6]
)
