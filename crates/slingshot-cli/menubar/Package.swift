// swift-tools-version:5.9
// The Slingshot menu bar app. It only draws what `slingshot internal-watch` prints, so all
// polling and every decision to notify stays in the Rust program.

import PackageDescription

let package = Package(
    name: "SlingshotMenuBar",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(name: "SlingshotMenuBar", path: "Sources")
    ]
)
