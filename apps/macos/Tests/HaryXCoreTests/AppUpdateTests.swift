import CryptoKit
import Foundation
import Testing
@testable import HaryXCore

private let upgradeContract = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("../../../../packages/contracts/haryx/upgrade-check.json")

@Test func decodesTheUpgradeCheckContract() throws {
    let check = try UpgradeCheck.decode(Data(contentsOf: upgradeContract))
    #expect(check.latest == "0.2.0")
    #expect(check.available)
    #expect(check.install == "app")
    #expect(check.assets.app?.name == "HaryX-0.2.0.dmg")
    #expect(check.assets.app?.sha256Url.lastPathComponent == "HaryX-0.2.0.dmg.sha256")
    #expect(check.error == nil)
    #expect(check.isNewer(than: "0.1.4"))
    #expect(!check.isNewer(than: "0.2.0"))
}

@Test func decodesAFailedCheck() throws {
    let json = #"{"current":"0.1.4","latest":null,"available":false,"published_at":null,"notes":null,"html_url":null,"install":"source","assets":{"app":null,"cli":null},"error":"offline"}"#
    let check = try UpgradeCheck.decode(Data(json.utf8))
    #expect(check.error == "offline")
    #expect(!check.isNewer(than: "0.0.1"))
}

@Test func comparesVersions() {
    #expect(AppVersion.compare("0.1.10", "0.1.9") == .orderedDescending)
    #expect(AppVersion.compare("v0.2", "0.2.0") == .orderedSame)
    #expect(AppVersion.compare("1.0.0-rc1", "1.0.0") == .orderedAscending)
    #expect(AppVersion.compare("1.0.0", "1.0.0-rc1") == .orderedDescending)
    #expect(AppVersion.compare("0.1.4", "0.2.0") == .orderedAscending)
}

@Test func parsesChecksumFiles() {
    let hex = String(repeating: "a", count: 64)
    #expect(AppUpdater.parseChecksum("\(hex)  HaryX.dmg\n") == hex)
    #expect(AppUpdater.parseChecksum(hex.uppercased()) == hex)
    #expect(AppUpdater.parseChecksum("<html>") == nil)
    #expect(AppUpdater.parseChecksum("") == nil)
}

/// A temp folder with a served "release": url → file contents.
private struct Release {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("app-update-\(UUID().uuidString)")
    var work: URL { root.appendingPathComponent("work") }

    func updater(serving files: [String: Data], app: URL? = nil) -> AppUpdater {
        AppUpdater(appURL: app ?? root.appendingPathComponent("Applications/HaryX.app"), workDir: work) { url, dest in
            guard let body = files[url.absoluteString] else { throw AppUpdateError.tool("404 \(url)") }
            try body.write(to: dest)
        }
    }

    func asset(_ name: String) -> UpgradeCheck.Download {
        UpgradeCheck.Download(name: name, url: URL(string: "https://x/\(name)")!,
                              sha256Url: URL(string: "https://x/\(name).sha256")!, size: 1)
    }
}

private func hexDigest(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

@Test func downloadKeepsAMatchingFile() async throws {
    let r = Release()
    let body = Data("dmg".utf8)
    let updater = r.updater(serving: [
        "https://x/HaryX-1.dmg": body,
        "https://x/HaryX-1.dmg.sha256": Data("\(hexDigest(body))  HaryX-1.dmg\n".utf8),
    ])
    let dmg = try await updater.download(r.asset("HaryX-1.dmg"))
    #expect(try Data(contentsOf: dmg) == body)
}

@Test func downloadRefusesAndDeletesAMismatch() async throws {
    let r = Release()
    let updater = r.updater(serving: [
        "https://x/HaryX-1.dmg": Data("tampered".utf8),
        "https://x/HaryX-1.dmg.sha256": Data("\(String(repeating: "0", count: 64))  x\n".utf8),
    ])
    await #expect(throws: AppUpdateError.checksumMismatch("HaryX-1.dmg")) {
        try await updater.download(r.asset("HaryX-1.dmg"))
    }
    #expect(!FileManager.default.fileExists(atPath: r.work.appendingPathComponent("HaryX-1.dmg").path))
}

@Test func downloadRefusesABadChecksumFile() async throws {
    let r = Release()
    let updater = r.updater(serving: ["https://x/HaryX-1.dmg.sha256": Data("<html>".utf8)])
    await #expect(throws: AppUpdateError.badChecksumFile("HaryX-1.dmg.sha256")) {
        try await updater.download(r.asset("HaryX-1.dmg"))
    }
}

/// A minimal ad-hoc signed HaryX.app at `dir` reporting `version`.
private func makeApp(at dir: URL, version: String) throws {
    let macOS = dir.appendingPathComponent("Contents/MacOS")
    try FileManager.default.createDirectory(at: macOS, withIntermediateDirectories: true)
    let plist = """
        <?xml version="1.0" encoding="UTF-8"?>
        <plist version="1.0"><dict>
        <key>CFBundleExecutable</key><string>HaryX</string>
        <key>CFBundleIdentifier</key><string>dev.haryx.test</string>
        <key>CFBundleShortVersionString</key><string>\(version)</string>
        </dict></plist>
        """
    try plist.write(to: dir.appendingPathComponent("Contents/Info.plist"), atomically: true, encoding: .utf8)
    try "#!/bin/sh\n".write(to: macOS.appendingPathComponent("HaryX"), atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: macOS.appendingPathComponent("HaryX").path)
    try AppUpdater.tool("/usr/bin/codesign", ["--force", "--sign", "-", dir.path])
}

@Test func stagesTheAppFromADMGAndSwapsItIn() throws {
    let r = Release()
    let app = r.root.appendingPathComponent("Applications/HaryX.app")
    try makeApp(at: app, version: "0.1.0")
    let src = r.root.appendingPathComponent("dmg-src")
    try makeApp(at: src.appendingPathComponent("HaryX.app"), version: "0.2.0")
    let dmg = r.root.appendingPathComponent("HaryX-0.2.0.dmg")
    try AppUpdater.tool("/usr/bin/hdiutil", ["create", "-quiet", "-volname", "HaryX", "-srcfolder", src.path,
                                            "-fs", "HFS+", "-format", "UDZO", dmg.path])
    let updater = r.updater(serving: [:], app: app)
    try FileManager.default.createDirectory(at: r.work, withIntermediateDirectories: true)

    let staged = try updater.stage(dmg: dmg)
    #expect(updater.stagedVersion() == "0.2.0")
    #expect(updater.canReplaceInPlace)

    // Run the swap for an already-finished pid, without relaunching.
    let script = AppUpdater.swapScript(staged: staged, app: app, pid: 999_999, relaunch: false)
    try AppUpdater.tool("/bin/sh", ["-c", script])
    let info = NSDictionary(contentsOf: app.appendingPathComponent("Contents/Info.plist"))
    #expect(info?["CFBundleShortVersionString"] as? String == "0.2.0")
    #expect(!FileManager.default.fileExists(atPath: staged.path))
    #expect(!FileManager.default.fileExists(atPath: r.root.appendingPathComponent("Applications/.HaryX.app.old").path))
}

@Test func aDMGWithoutTheAppIsRefused() throws {
    let r = Release()
    let src = r.root.appendingPathComponent("empty-src")
    try FileManager.default.createDirectory(at: src, withIntermediateDirectories: true)
    try "x".write(to: src.appendingPathComponent("README"), atomically: true, encoding: .utf8)
    let dmg = r.root.appendingPathComponent("Other.dmg")
    try AppUpdater.tool("/usr/bin/hdiutil", ["create", "-quiet", "-volname", "X", "-srcfolder", src.path,
                                            "-fs", "HFS+", "-format", "UDZO", dmg.path])
    let updater = r.updater(serving: [:])
    try FileManager.default.createDirectory(at: r.work, withIntermediateDirectories: true)
    #expect(throws: AppUpdateError.notAnApp("Other.dmg")) { try updater.stage(dmg: dmg) }
}

@Test func swapScriptRestoresTheOldAppWhenTheMoveFails() throws {
    let r = Release()
    let app = r.root.appendingPathComponent("Applications/HaryX.app")
    try makeApp(at: app, version: "0.1.0")
    let script = AppUpdater.swapScript(staged: r.root.appendingPathComponent("missing.app"), app: app,
                                       pid: 999_999, relaunch: false)
    #expect(throws: (any Error).self) { try AppUpdater.tool("/bin/sh", ["-c", script]) }
    let info = NSDictionary(contentsOf: app.appendingPathComponent("Contents/Info.plist"))
    #expect(info?["CFBundleShortVersionString"] as? String == "0.1.0")
}

@Test func translocatedOrReadOnlyAppsCannotReplaceThemselves() {
    let translocated = AppUpdater(appURL: URL(fileURLWithPath: "/private/var/folders/x/AppTranslocation/1/d/HaryX.app"))
    #expect(!translocated.canReplaceInPlace)
    let system = AppUpdater(appURL: URL(fileURLWithPath: "/System/Applications/HaryX.app"))
    #expect(!system.canReplaceInPlace)
}
