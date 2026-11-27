import Foundation
import Testing
@testable import HaryXCore

private func fixture(_ name: String) throws -> Data {
    try Data(contentsOf: contractsDir.deletingLastPathComponent().appendingPathComponent("haryx/\(name)"))
}

private func decode<T: Decodable>(_ type: T.Type, _ json: String, status: Int32 = 0) -> Result<T, HaryXFailure> {
    HaryXFailure.decode(T.self, stdout: Data(json.utf8), status: status, stderr: "")
}

private func failure<T>(_ r: Result<T, HaryXFailure>) -> HaryXFailure? {
    if case .failure(let f) = r { return f }
    return nil
}

// MARK: - contract fixtures

@Test func decodesWhoAmIFixture() throws {
    let w = try HaryXFailure.decode(WhoAmI.self, stdout: fixture("whoami.json"), status: 0, stderr: "").get()
    #expect(w.signedIn)
    #expect(w.user?.email == "you@example.com")
    #expect(w.user?.providerName == "GitHub")
    #expect(try decode(WhoAmI.self, #"{"signedIn": false, "user": null}"#).get() == .signedOut)
}

@Test func decodesTheConsentMachineAndWhoAmIConsentFixtures() throws {
    let w = try HaryXFailure.decode(WhoAmI.self, stdout: fixture("whoami.json"), status: 0, stderr: "").get()
    #expect(w.consents?.map(\.purpose) == ["sync", "machines"])
    #expect(w.consents?[1].version == nil)
    let c = try HaryXFailure.decode(Consents.self, stdout: fixture("consents.json"), status: 0, stderr: "").get()
    #expect(c.consents[1].granted == false)
    #expect(c.consents[1].history.map(\.action) == ["grant", "withdraw"])
    let m = try HaryXFailure.decode(HaryXMachines.self, stdout: fixture("machines.json"), status: 0, stderr: "").get()
    #expect(m.machines[0].isThisMachine && !m.machines[1].signedIn)
    #expect(m.machines[0].lastSeenAt == "2026-11-25T10:00:00Z")
}

@Test func parsesLoginEventLines() throws {
    let lines = try JSONSerialization.jsonObject(with: fixture("login-events.json")) as? [[String: Any]] ?? []
    let events = try lines.map { try JSONSerialization.data(withJSONObject: $0) }
        .compactMap { LoginEvent.parse(line: String(decoding: $0, as: UTF8.self)) }
    #expect(events.count == 3)
    guard case .browser(let url, let opened) = events[0] else { Issue.record("not browser"); return }
    #expect(url?.host() == "ygvcznwlugaxqqmlsssh.supabase.co")
    #expect(opened)
    guard case .signedIn(let user) = events[1] else { Issue.record("not signed_in"); return }
    #expect(user.provider == "github")
    #expect(events[2] == .error(code: "timeout", message: "No sign-in arrived within 5 minutes."))
    #expect(LoginEvent.parse(line: "not json") == nil)
    #expect(LoginEvent.parse(line: #"{"event":"later"}"#) == nil)
}

@Test func decodesSyncStatusFixture() throws {
    let s = try HaryXFailure.decode(SyncStatus.self, stdout: fixture("sync-status.json"), status: 0, stderr: "").get()
    #expect(s.signedIn && s.unlocked)
    #expect(s.lastSyncAt == CSwapJSON.parseDate("2026-11-25T10:00:00Z"))
    #expect(s.otherMachineAccounts.first?.alias == "work")
    #expect(s.overwritten.first?.keptLocal == true)
    #expect(s.overwritten.first?.at == Date(timeIntervalSince1970: 1_795_600_800))
    let fresh = try decode(SyncStatus.self, #"{"signedIn":true,"unlocked":false,"lastSyncAt":null,"otherMachineAccounts":[],"overwritten":[]}"#).get()
    #expect(fresh.lastSyncAt == nil)
}

@Test func decodesSyncResultFixture() throws {
    let r = try HaryXFailure.decode(SyncResult.self, stdout: fixture("sync-result.json"), status: 0, stderr: "").get()
    #expect(r.pushed == 2 && r.pulled == 1)
    #expect(r.changed == ["settings", "account"])
    #expect(r.overwritten.first?.keptLocal == false)
    #expect(r.otherMachineAccounts.first?.organizationName == "Acme")
}

// MARK: - replies and failures

@Test func failureEnvelopeWinsOverTheExpectedShape() throws {
    let r = decode(SyncResult.self, #"{"ok":false,"code":"locked","message":"Unlock first."}"#, status: 1)
    #expect(failure(r) == HaryXFailure(code: "locked", message: "Unlock first."))
    #expect(try decode(PassphraseCheck.self, #"{"ok":true,"hasPassphrase":false}"#).get().hasPassphrase == false)
    #expect(try decode(UnlockResult.self, #"{"ok":true,"created":true}"#).get().created == true)
    #expect((try? decode(OKReply.self, #"{"ok":true,"removed":2}"#).get()) != nil)
}

@Test func nonJSONOutputBecomesTheCLIErrorLine() {
    let r = HaryXFailure.decode(SyncResult.self, stdout: Data(), status: 2, stderr: "Error: unrecognized subcommand 'sync'\n")
    #expect(failure(r) == HaryXFailure(code: "unexpected", message: "unrecognized subcommand 'sync'"))
}

@Test(arguments: [
    ("not_signed_in", SyncProblem.signedOut), ("signed_out", .signedOut), ("locked", .locked),
    ("key_changed", .keyChanged), ("busy", .busy), ("network", .unavailable), ("server", .unavailable),
    ("local", .failed("disk full")),
])
func mapsSyncFailureCodes(code: String, expected: SyncProblem) {
    let problem = SyncProblem(HaryXFailure(code: code, message: "disk full"))
    #expect(problem == expected)
    #expect(problem.needsUnlock == (code == "locked" || code == "key_changed"))
}

@Test func passphraseRules() {
    #expect(SyncPassphrase.problem("short", confirm: nil) != nil)
    #expect(SyncPassphrase.problem("twelve chars", confirm: nil) == nil)
    #expect(SyncPassphrase.problem("twelve chars", confirm: "twelve charz") == "The two passphrases do not match.")
    #expect(SyncPassphrase.problem("twelve chars\nmore", confirm: nil) != nil)
}

// MARK: - app preferences

@Test func importsKnownKeysWithTheRightTypeOnly() throws {
    let synced = try JSONSerialization.jsonObject(with: fixture("app-prefs.json")) as? [String: Any] ?? [:]
    var values = AppPrefsSync.importValues(synced)
    #expect(values["theme"] as? String == "system")
    #expect(values["notify.usage"] as? Bool == true)
    #expect(values["notify.switched"] as? Bool == true)
    #expect(values["notify.session"] as? Bool == true)
    #expect(values["menuBarMascot"] as? Bool == false)
    #expect(values["menuBarPercent"] as? Bool == true)
    #expect(values.keys.allSatisfy { !$0.hasPrefix("windows") }, "windows.* is the Windows app's")
    let fixtureStyles = try JSONSerialization.jsonObject(with: values["mascotStyles"] as? Data ?? Data())
        as? [String: [String: Any]]
    #expect(fixtureStyles?["you@example.com|"]?["species"] as? String == "cat")
    #expect(fixtureStyles?["you@example.com|"]?["hue"] as? Double == 0.55)

    values = AppPrefsSync.importValues([
        "theme": "sepia", "weeklySummary": 1, "macos.refreshSeconds": true, "updateMode": "manual",
        "macos.desktopFollows": "yes", "mascotStyles": ["a|b": ["species": "cat", "hue": 0.5], "c|d": ["species": "cat", "hue": "red"]],
    ])
    #expect(values["theme"] == nil && values["weeklySummary"] == nil && values["refreshSeconds"] == nil)
    #expect(values["desktopFollows"] == nil)
    #expect(values["updateMode"] as? String == "manual")
    let styles = try JSONSerialization.jsonObject(with: values["mascotStyles"] as? Data ?? Data()) as? [String: Any]
    #expect(styles?.keys.sorted() == ["a|b"])
}

@Test func exportsOnlySetSyncedPrefsAndNeverMachineState() throws {
    let styles = try JSONSerialization.data(withJSONObject: ["a|b": ["species": "frog"]])
    let stored: [String: Any] = [
        "theme": "dark", "refreshSeconds": 120.0, "notify.session": false, "mascotStyles": styles,
        "cswapPath": "/opt/haryx", "terminal": "iTerm", "lastUpdateCheck": 5.0, "autoWasOn": true, "syncEnabled": false,
    ]
    let out = AppPrefsSync.export(stored)
    #expect(out.keys.sorted() == ["macos.refreshSeconds", "mascotStyles", "notifications.session", "theme"])
    #expect(out["notifications.session"] as? Bool == false)
    #expect((out["mascotStyles"] as? [String: Any])?["a|b"] != nil)
}

@Test func roundTripsThroughUserDefaultsShapes() {
    let stored: [String: Any] = ["theme": "light", "weeklySummary": true, "notify.autoSwitch": false, "refreshSeconds": 300.0]
    let back = AppPrefsSync.importValues(AppPrefsSync.export(stored))
    #expect(AppPrefsSync.same(back, stored))
}

@Test func overlayKeepsKeysThisAppDoesNotOwn() {
    let current: [String: Any] = ["theme": "system", "windows.trayMascot": false, "future": 1]
    let merged = AppPrefsSync.overlay(["theme": "dark"], on: current)
    #expect(merged["theme"] as? String == "dark")
    #expect(merged["windows.trayMascot"] as? Bool == false)
    #expect(merged["future"] as? Int == 1)
    #expect(!AppPrefsSync.same(merged, current))
    #expect(AppPrefsSync.same(AppPrefsSync.overlay([:], on: current), current))
}
