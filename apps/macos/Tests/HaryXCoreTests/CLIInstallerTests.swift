import Foundation
import Testing
@testable import HaryXCore

/// A temp folder with a fake bundled CLI inside `HaryX.app/Contents/Resources/bin`.
private struct Sandbox {
    let root: URL
    let bundled: String

    init() throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent("cli-installer-\(UUID().uuidString)")
        let bin = root.appendingPathComponent("HaryX.app/Contents/Resources/bin")
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        bundled = bin.appendingPathComponent("haryx").path
        FileManager.default.createFile(atPath: bundled, contents: Data("#!/bin/sh\n".utf8), attributes: [.posixPermissions: 0o755])
    }

    func path(_ rel: String) -> String { root.appendingPathComponent(rel).path }

    func installer(bundled: String? = nil, others: [String] = []) -> CLIInstaller {
        CLIInstaller(bundled: bundled ?? self.bundled, links: [path("system/haryx"), path("user/bin/haryx")],
                     otherPaths: others)
    }
}

@Test func offersInstallWhenNothingIsLinked() throws {
    let s = try Sandbox()
    #expect(s.installer().state == .notInstalled)
    #expect(s.installer().shouldOffer)
    #expect(CLIInstaller(bundled: nil, links: [s.path("x/haryx")], otherPaths: []).state == .notBundled)
}

@Test func linksAndRemovesPerUserCommand() throws {
    let s = try Sandbox()
    let installer = s.installer()
    let link = s.path("user/bin/haryx")
    try installer.installLink(at: link)
    #expect(installer.state == .linked(link))
    #expect(!installer.shouldOffer)
    try installer.installLink(at: link)  // reinstall over our own link is fine
    try installer.removeLink(at: link)
    #expect(installer.state == .notInstalled)
}

@Test func neverReplacesAnotherInstall() throws {
    let s = try Sandbox()
    let brew = s.path("brew")
    try FileManager.default.createDirectory(atPath: brew, withIntermediateDirectories: true)
    FileManager.default.createFile(atPath: brew + "/haryx", contents: Data(), attributes: [.posixPermissions: 0o755])
    let installer = s.installer(others: [brew])
    #expect(installer.state == .otherInstall(brew + "/haryx"))
    #expect(!installer.shouldOffer)

    let occupied = s.path("user/bin/haryx")
    try FileManager.default.createDirectory(atPath: s.path("user/bin"), withIntermediateDirectories: true)
    FileManager.default.createFile(atPath: occupied, contents: Data("real".utf8))
    #expect(throws: CLIInstaller.InstallError.occupied(occupied)) { try installer.installLink(at: occupied) }
    #expect(throws: CLIInstaller.InstallError.occupied(occupied)) { try installer.removeLink(at: occupied) }
}

@Test func replacesDanglingLinkFromMovedApp() throws {
    let s = try Sandbox()
    let link = s.path("user/bin/haryx")
    try FileManager.default.createDirectory(atPath: s.path("user/bin"), withIntermediateDirectories: true)
    try FileManager.default.createSymbolicLink(atPath: link, withDestinationPath: "/gone/HaryX.app/Contents/Resources/bin/haryx")
    try s.installer().installLink(at: link)
    #expect(s.installer().isOurLink(link))
}

@Test func adminCommandQuotesPaths() throws {
    let s = try Sandbox()
    let cmd = try s.installer().adminInstallCommand(at: "/usr/local/bin/haryx")
    #expect(cmd.hasPrefix("/bin/mkdir -p '/usr/local/bin' && /bin/ln -sfh '"))
    #expect(cmd.hasSuffix("' '/usr/local/bin/haryx'"))
    #expect(CLIInstaller.quote("it's") == "'it'\\''s'")
    #expect(CLIInstaller.onPath("/usr/local/bin", path: "/usr/bin:/usr/local/bin"))
    #expect(!CLIInstaller.onPath("/x", path: nil))
}
