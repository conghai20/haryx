// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "HaryX",
    platforms: [.macOS("26.0")],
    targets: [
        .target(name: "HaryXCore", path: "Sources/HaryXCore"),
        .executableTarget(name: "HaryX", dependencies: ["HaryXCore"], path: "Sources/HaryX"),
        .testTarget(name: "HaryXCoreTests", dependencies: ["HaryXCore"], path: "Tests/HaryXCoreTests"),
    ]
)
