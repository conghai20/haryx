// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "cswapui",
    platforms: [.macOS("26.0")],
    targets: [
        .target(name: "CSwapCore", path: "Sources/CSwapCore"),
        .executableTarget(name: "cswapui", dependencies: ["CSwapCore"], path: "Sources/cswapui"),
        .testTarget(name: "CSwapCoreTests", dependencies: ["CSwapCore"], path: "Tests/CSwapCoreTests"),
    ]
)
