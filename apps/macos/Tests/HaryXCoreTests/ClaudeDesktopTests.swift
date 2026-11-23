import Foundation
import Testing
@testable import HaryXCore

private let statusJSON = """
{
  "accounts": [
    {"number": 1, "email": "a@x.com", "alias": "", "uuid": "aaaa", "folder": "/s/desktop/aaaa",
     "folderExists": true, "signedInUuid": "aaaa", "mismatch": false},
    {"number": 2, "email": "b@x.com", "alias": "work", "uuid": "bbbb", "folder": "/s/desktop/bbbb",
     "folderExists": false, "signedInUuid": null, "mismatch": false},
    {"number": 3, "email": "c@x.com", "alias": "", "uuid": "cccc", "folder": "/s/desktop/cccc",
     "folderExists": true, "signedInUuid": "aaaa", "mismatch": true},
    {"number": 4, "email": "d@token.local", "alias": "", "uuid": "", "folder": null,
     "folderExists": false, "signedInUuid": null, "mismatch": false}
  ]
}
"""

private func status() throws -> DesktopStatus {
    try CSwapJSON.decoder.decode(DesktopStatus.self, from: Data(statusJSON.utf8))
}

@Test func desktopTargetPerAccount() throws {
    let s = try status()
    #expect(ClaudeDesktop.target(for: 1, in: s) == .ready(folder: "/s/desktop/aaaa"))
    #expect(ClaudeDesktop.target(for: 2, in: s) == .setUp(folder: "/s/desktop/bbbb"))
    #expect(ClaudeDesktop.target(for: 3, in: s) == .mismatch)
    #expect(ClaudeDesktop.target(for: 4, in: s) == .unavailable)
    #expect(ClaudeDesktop.target(for: 9, in: s) == .unavailable)
}

@Test func runningFolderMapsToItsAccount() throws {
    let s = try status()
    #expect(s.owner(ofRunningFolder: "/s/desktop/aaaa")?.number == 1)
    #expect(s.owner(ofRunningFolder: "/s/desktop/aaaa/")?.number == 1)
    #expect(s.owner(ofRunningFolder: "/s/desktop/../desktop/bbbb")?.number == 2)
    #expect(s.owner(ofRunningFolder: "/elsewhere") == nil)
    #expect(s.owner(ofRunningFolder: nil) == nil, "default folder belongs to no account")
}

@Test func userDataDirFromArguments() {
    let exe = "/Applications/Claude.app/Contents/MacOS/Claude"
    #expect(ClaudeDesktop.userDataDir(fromArguments: [exe, "--user-data-dir=/a b/c"]) == "/a b/c")
    #expect(ClaudeDesktop.userDataDir(fromArguments: [exe, "--user-data-dir", "/d"]) == "/d")
    #expect(ClaudeDesktop.userDataDir(fromArguments: [exe]) == nil)
    #expect(ClaudeDesktop.userDataDir(fromArguments: [exe, "--user-data-dir"]) == nil)
    #expect(ClaudeDesktop.userDataDir(fromArguments: ["--user-data-dir=/exe-slot"]) == nil, "argv[0] is never a flag")
}

@Test func launchArgumentsRoundTrip() {
    let args = ["/exe"] + ClaudeDesktop.launchArguments(folder: "/s/desktop/aaaa")
    #expect(ClaudeDesktop.userDataDir(fromArguments: args) == "/s/desktop/aaaa")
    #expect(ClaudeDesktop.sameFolder("/s/x/", "/s/x"))
    #expect(!ClaudeDesktop.sameFolder(nil, "/s/x"))
}

private func line(_ run: DesktopRun, active: Int) throws -> DesktopLine? {
    DesktopLine.make(run: run, status: try status(), active: active) { "#\($0)" }
}

@Test func desktopLineWhenRunningOnTheActiveAccount() throws {
    #expect(try line(.running(folder: "/s/desktop/aaaa"), active: 1) == DesktopLine(text: "Desktop: #1", action: nil))
}

@Test func desktopLineWhenStillOnAnotherAccount() throws {
    #expect(try line(.running(folder: "/s/desktop/bbbb"), active: 1) == DesktopLine(text: "Desktop still on #2", action: .open))
    #expect(try line(.running(folder: "/s/desktop/aaaa"), active: 2) == DesktopLine(text: "Desktop still on #1", action: .setUp))
}

@Test func desktopLineOnTheDefaultFolder() throws {
    #expect(try line(.running(folder: nil), active: 1)
        == DesktopLine(text: "Desktop: its own folder, not one of HaryX's", action: .open))
}

@Test func desktopLineNeverActsOnAnUnreadableFolder() throws {
    #expect(try line(.runningUnknownFolder, active: 1) == DesktopLine(text: "Desktop: running", action: nil))
}

@Test func desktopLineWhenNotRunning() throws {
    #expect(try line(.notRunning, active: 1) == DesktopLine(text: "Desktop: not running", action: .open))
    #expect(try line(.notRunning, active: 2) == DesktopLine(text: "Desktop: not running", action: .setUp))
    #expect(try line(.notRunning, active: 4) == nil, "a token slot has no Desktop")
}

@Test func desktopLineNeverOffersToOpenAMismatchedFolder() throws {
    let warning = "#3's Desktop folder is signed into another account"
    #expect(try line(.notRunning, active: 3) == DesktopLine(text: warning, action: nil))
    #expect(try line(.running(folder: "/s/desktop/aaaa"), active: 3) == DesktopLine(text: warning, action: nil))
}
