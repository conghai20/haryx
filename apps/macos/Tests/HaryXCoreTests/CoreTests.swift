import Foundation
import Testing
@testable import HaryXCore

/// Shared contract fixture: a trimmed, anonymised `cswap list --json` (1.1.0) payload.
/// `haryx` is a drop-in port, so the app must decode both CLIs' payloads (see `decodesHaryxPayload`).
let contractsDir = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("../../../../packages/contracts/cswap")
    .standardizedFileURL


func decodeFixture() throws -> AccountList {
    try CSwapJSON.decoder.decode(AccountList.self, from: Data(contentsOf: contractsDir.appendingPathComponent("list.json")))
}

func account(_ base: Account, five: Double?, seven: Double? = nil, active: Bool = true) throws -> Account {
    // Round-trip through JSON so tests use the same decoding path as the app.
    let json: [String: Any?] = [
        "number": base.number, "email": base.email, "organizationUuid": base.organizationUuid,
        "active": active, "usageStatus": "ok",
        "usage": ["fiveHour": ["pct": five as Any, "resetsAt": "2026-11-20T15:29:59Z"], "sevenDay": ["pct": seven as Any]],
    ]
    let data = try JSONSerialization.data(withJSONObject: json.compactMapValues { $0 })
    return try CSwapJSON.decoder.decode(Account.self, from: data)
}

@Test func decodesRealListPayload() throws {
    let list = try decodeFixture()
    #expect(list.activeAccountNumber == 3)
    #expect(list.accounts.count == 3)
    let first = list.accounts[0]
    #expect(first.usage?.sevenDay?.resetsAt != nil, "microsecond dates must parse")
    #expect(first.usage?.sevenDay?.aheadOfPace == true)
    #expect(first.plan?.label == "Team")
    #expect(list.accounts[1].displayName == "work")
    #expect(list.accounts[2].isDisabled && list.accounts[2].needsRelogin)
    #expect(list.accounts[1].peakPct == 88)
}

@Test func anAccountAtItsLimitRestsAndItsMascotSleeps() throws {
    let list = try decodeFixture()
    let full = try account(list.accounts[0], five: 100, seven: 40)
    #expect(full.isExhausted && full.isResting && !full.isDisabled)
    #expect(Mood.of(full, running: true) == .sleeping)
    let tired = try account(list.accounts[0], five: 92)
    #expect(!tired.isResting)
    #expect(Mood.of(tired, running: false) == .tired)
    #expect(Mood.of(tired, running: true) == .running)
    #expect(list.accounts[2].isResting)
    #expect(Mood.of(list.accounts[2], running: true) == .sleeping)
}

@Test func aMascotGetsAngryFrom75PercentUntilItTires() throws {
    let base = try decodeFixture().accounts[0]
    #expect(Mood.of(try account(base, five: 74), running: false) == .idle)
    #expect(Mood.of(try account(base, five: 10, seven: 75), running: false) == .angry)
    #expect(Mood.of(try account(base, five: 89), running: false) == .angry)
    #expect(Mood.of(try account(base, five: 89), running: true) == .running)
    #expect(Mood.of(try account(base, five: 90), running: false) == .tired)
}

@Test func anAngryMascotThrobsAnAngerMark() {
    let palette = Palette(species: .critter, hue: nil)
    let calm = PixelArt.pixels(.critter, palette: palette, pose: Pose(mood: .idle))
    let big = PixelArt.pixels(.critter, palette: palette, pose: Pose(overlayPhase: 0, mood: .angry))
    let small = PixelArt.pixels(.critter, palette: palette, pose: Pose(overlayPhase: 1, mood: .angry))
    let marks = { (g: [[RGB?]]) in g.joined().filter { $0 == PixelArt.alert }.count }
    #expect(marks(calm) == 0)
    #expect(marks(big) == 12)
    #expect(marks(small) == 4)
}

@Test(arguments: ["2026-11-25T11:00:00.497815+00:00", "2026-12-14T03:30:48Z", "2026-11-20T15:29:59.9+00:00"])
func parsesCswapDates(_ raw: String) {
    #expect(CSwapJSON.parseDate(raw) != nil)
}

@Test func mapsConfigDirToSlot() {
    let home = "/Users/me", backup = "/Users/me/.claude-swap-backup"
    #expect(ProcessScanner.slot(forConfigDir: nil, backupRoot: backup, home: home, activeSlot: 3) == 3)
    #expect(ProcessScanner.slot(forConfigDir: "/Users/me/.claude/", backupRoot: backup, home: home, activeSlot: 3) == 3)
    #expect(ProcessScanner.slot(forConfigDir: backup + "/sessions/12-me_x.com", backupRoot: backup, home: home, activeSlot: 3) == 12)
    #expect(ProcessScanner.slot(forConfigDir: "/tmp/other", backupRoot: backup, home: home, activeSlot: 3) == nil)
}

@Test func parsesProcArgsBuffer() {
    var buf = withUnsafeBytes(of: Int32(2)) { Array($0) }
    for s in ["/bin/claude", "", "", "claude", "--resume", "PWD=/w", "CLAUDE_CONFIG_DIR=/c", ""] {
        buf += Array(s.utf8) + [0]
    }
    let parsed = ProcessScanner.parseProcArgs(buf)
    #expect(parsed?.0 == ["claude", "--resume"])
    #expect(parsed?.1["CLAUDE_CONFIG_DIR"] == "/c")
    var withEmpty = withUnsafeBytes(of: Int32(3)) { Array($0) }
    for s in ["/bin/claude", "", "claude", "-p", "", "CLAUDE_CONFIG_DIR=/s", ""] {
        withEmpty += Array(s.utf8) + [0]
    }
    let parsedEmpty = ProcessScanner.parseProcArgs(withEmpty)
    #expect(parsedEmpty?.0 == ["claude", "-p", ""], "empty argv entries must not end parsing")
    #expect(parsedEmpty?.1["CLAUDE_CONFIG_DIR"] == "/s")
    #expect(ProcessScanner.isClaudeBinary("/Users/me/.local/share/claude/versions/2.1.3"))
    #expect(!ProcessScanner.isClaudeBinary("/Applications/Claude.app/Contents/MacOS/Claude"))
}

@Test func notifiesSwitchAndThresholdsOnce() throws {
    let list = try decodeFixture()
    let a = list.accounts[0], c = list.accounts[1]
    let before = [try account(a, five: 10, active: true), try account(c, five: 10, active: false)]
    let after = [try account(a, five: 10, active: false), try account(c, five: 96, active: true)]
    let notices = NotificationRules.evaluate(previous: before, current: after)
    #expect(notices.contains { $0.category == .switched && $0.title.contains("c@x.com") })
    let usage = notices.filter { $0.category == .usage }
    #expect(usage.count == 1 && usage[0].key.contains(":95:"), "only the highest crossed level fires")
}

@Test func notifiesQuotaReset() throws {
    let a = try decodeFixture().accounts[0]
    let notices = NotificationRules.evaluate(previous: [try account(a, five: 100)], current: [try account(a, five: 3)])
    #expect(notices.contains { $0.category == .reset })
}

@Test func parsesTextOutputs() {
    let prio = TextParsers.priorities("""
    Account priorities (higher first; default 0):
      Account-1  a@x.com  0
      Account-12  b@x.com  250
    Used only by: cswap auto --strategy priority-consume-first
    """)
    #expect(prio == [1: 0, 12: 250])
    let unclaimed = TextParsers.unclaimed("abc123  slot 4  orphaned (no manifest row)")
    #expect(unclaimed.first?.slot == "4" && unclaimed.first?.reason == "orphaned (no manifest row)")
}

@Test func parsesAutoEvents() {
    let sw = AutoEvent.parse(line: #"{"schemaVersion":1,"event":"switch","ts":"2026-11-20T10:00:00Z","trigger":"at-limit","from":{"number":"1","email":"a@x"},"to":{"number":"2","email":"b@x"}}"#)
    #expect(sw?.summary.contains("Account-1") == true && sw?.summary.contains("Account-2") == true)
    #expect(AutoEvent.parse(line: #"{"event":"all-exhausted","earliestResetAt":null}"#)?.notificationTitle != nil)
    #expect(AutoEvent.parse(line: #"{"event":"sleep","seconds":60}"#) == nil)
    #expect(AutoEvent.parse(line: "not json") == nil)
}

@Test func quotesForShell() {
    #expect(shellQuote("it's") == #"'it'\''s'"#)
}

@Test(arguments: Species.allCases)
func spritesAreWellFormed(_ species: Species) {
    let rows = species.body + species.legs.a + species.legs.b
    #expect(rows.allSatisfy { $0.count == 16 }, "every row must be 16 wide")
    #expect(species.legs.a.count == species.legs.b.count)
    #expect(species.body.count + species.legs.a.count <= PixelArt.canvas - 3, "leave headroom for overlays")
    #expect(species.body.joined().contains("E"), "needs eyes to blink")
    let palette = Palette(species: species, hue: 0.5)
    for mood in [Mood.idle, .running, .tired, .sleeping, .error] {
        #expect(PixelArt.image(species, palette: palette, pose: Pose.at(1.3, mood: mood, phase: 2), scale: 2) != nil)
    }
}

@Test func blinkingChangesPixels() {
    let p = Palette(species: .cat, hue: nil)
    #expect(PixelArt.pixels(.cat, palette: p, pose: Pose()) != PixelArt.pixels(.cat, palette: p, pose: Pose(blink: true)))
}

@Test func stableMascotAssignment() {
    #expect(Species.defaultFor(key: "a@x.com|o1") == Species.defaultFor(key: "a@x.com|o1"))
    #expect(AppIcon.render()?.width == AppIcon.side)
}

@Test func locatePrefersHaryxOverCswap() throws {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let first = dir.appendingPathComponent("a"), second = dir.appendingPathComponent("b")
    for d in [first, second] { try FileManager.default.createDirectory(at: d, withIntermediateDirectories: true) }
    defer { try? FileManager.default.removeItem(at: dir) }
    func touch(_ url: URL) throws {
        try Data("#!/bin/sh\n".utf8).write(to: url)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
    }
    // cswap earlier on the path, haryx later: haryx still wins.
    try touch(first.appendingPathComponent("cswap"))
    #expect(CSwapCLI.locate(override: nil, searchPaths: [first.path, second.path])?.lastPathComponent == "cswap")
    try touch(second.appendingPathComponent("haryx"))
    let found = try #require(CSwapCLI.locate(override: nil, searchPaths: [first.path, second.path]))
    #expect(found.lastPathComponent == "haryx")
    #expect(CLIFlavor.of(found) == .haryx)
}

@Test func flavorsKeepSeparateStores() {
    #expect(CLIFlavor.cswap.storeRoot.hasSuffix("/.claude-swap-backup"))
    #expect(CLIFlavor.haryx.logFileName == "haryx.log")
    #expect(CLIFlavor.of(URL(fileURLWithPath: "/opt/homebrew/bin/cswap")) == .cswap)
    #expect(CLIFlavor.of(URL(fileURLWithPath: "/x/haryx")) == .haryx)
}

/// Real `haryx list --json` output (sandbox, usage endpoint unreachable): `usage: null` plus
/// `usageError`/`usageRetryAt` must decode like a cswap row.
@Test func decodesHaryxPayload() throws {
    let url = contractsDir.deletingLastPathComponent().appendingPathComponent("haryx/list-unavailable.json")
    let list = try CSwapJSON.decoder.decode(AccountList.self, from: Data(contentsOf: url))
    #expect(list.activeAccountNumber == 2)
    #expect(list.accounts.count == 2)
    #expect(list.accounts[1].active)
    #expect(list.accounts[0].usageError == "network")
}

// MARK: - profiles

func profileFixture<T: Decodable>(_ type: T.Type, _ name: String) throws -> T {
    let url = contractsDir.deletingLastPathComponent().appendingPathComponent("haryx/\(name)")
    return try ProfileJSON.decoder.decode(T.self, from: Data(contentsOf: url))
}

@Test func decodesProfileContracts() throws {
    let list = try profileFixture(ProfileList.self, "profile-list.json")
    #expect(list.profiles.first?.name == "work")
    #expect(list.profiles.first?.secrets == "local")
    let show = try profileFixture(ProfileInventory.self, "profile-show.json")
    #expect(show.profile == "default")
    #expect(show.items.contains { $0.id == "skills/review" })
    #expect(show.grouped.first?.category == "settings")
    let plan = try profileFixture(ProfilePlan.self, "profile-plan.json")
    #expect(plan.changes.contains { $0.id == "skills/review" && $0.action == "update" })
    #expect(plan.unchangedCount == plan.entries.count - plan.changes.count)
}

@Test func profileArgumentsFollowOptions() {
    var o = ProfileExportOptions()
    #expect(ProfileArgs.export("default", to: "/tmp/f", o)
        == ["ai", "profile", "export", "default", "/tmp/f", "--secrets", "redact", "--plugins", "ref"])
    o.includeHistory = false
    o.credentials = true
    #expect(!o.isValid, "credentials need encryption")
    o.encrypt = true
    o.includeDirs = ["/x"]
    #expect(o.isValid)
    #expect(ProfileArgs.export("w", to: "f", o)
        == ["ai", "profile", "export", "w", "f", "--no-history", "--credentials", "--include-dir", "/x",
            "--secrets", "encrypt", "--plugins", "ref"])
    #expect(ProfileArgs.apply("w", prune: true, includeHistory: true) == ["ai", "profile", "apply", "w", "--yes", "--prune"])
}

@Test func validatesProfileNames() {
    #expect(ProfileArgs.isValidName("work-2"))
    #expect(!ProfileArgs.isValidName("default"))
    #expect(!ProfileArgs.isValidName(".hidden"))
    #expect(!ProfileArgs.isValidName("a/b"))
    #expect(!ProfileArgs.isValidName("bé"))
}

// MARK: - wake

@Test func decodesWakeStatus() throws {
    let url = contractsDir.deletingLastPathComponent().appendingPathComponent("haryx/wake-status.json")
    let status = try CSwapJSON.decoder.decode(WakeStatus.self, from: Data(contentsOf: url))
    #expect(status.agentInstalled)
    #expect(status.quietHours == "01:00-06:00")
    #expect(status.accounts.count == 3)
    #expect(status.account(1)?.on == true)
    #expect(status.account(3)?.summary() == nil)
    #expect(status.account(2)?.summary() == "wake skipped: weekly cap")
    #expect(status.account(1)?.lastWakeAt == Date(timeIntervalSince1970: 1_795_496_412))
    let events = status.recentEvents()
    #expect(events.map(\.account) == ["2", "1"], "newest first")
    #expect(events.first?.date == Date(timeIntervalSince1970: 1_795_502_463))
    #expect(events.first?.result == "skip:weekly-cap")
    #expect(status.recentEvents(limit: 1).count == 1)
}

@Test func wakeResultTextMatchesWindows() {
    #expect(WakeStatus.resultText(nil) == "—")
    #expect(WakeStatus.resultText("woke") == "Woke")
    #expect(WakeStatus.resultText("not-due") == "Not due")
    #expect(WakeStatus.resultText("skip:weekly-cap") == "Skipped: weekly cap")
    #expect(WakeStatus.resultText("skip:disabled") == "Skipped: account is off")
    #expect(WakeStatus.resultText("failed:http-401") == "Failed: http 401")
    #expect(WakeStatus.resultText("other") == "other")
}

@Test func decodesProfileDiff() throws {
    let diff = try profileFixture(ProfileDiff.self, "profile-diff.json")
    #expect(diff.left == "work")
    #expect(diff.right == "default")
    #expect(diff.differences.map(\.id) == ["skills/review"])
    #expect(diff.sameCount == 5)
    #expect(diff.differences.first?.stateText(left: "work") == "differs")
    let only = ProfileDiffItem(category: "skills", id: "x", state: "only-left")
    #expect(only.stateText(left: "work") == "only in work")
    #expect(ProfileArgs.diff("work") == ["ai", "profile", "diff", "work", "--json"])
}

@Test func wakeSummaryPrefersFailuresAndNextDue() {
    let now = Date(timeIntervalSince1970: 1_000)
    let failed = WakeAccount(number: 1, on: true, lastWakeAt: nil, nextDue: nil, lastResult: "failed:http-401")
    #expect(failed.summary(now: now) == "wake failed (http-401)")
    let due = WakeAccount(number: 1, on: true, lastWakeAt: nil, nextDue: Date(timeIntervalSince1970: 5_000), lastResult: "not-due")
    #expect(due.summary(now: now)?.hasPrefix("wakes ") == true)
}

@Test func wakeWindowResetIsNextDueLessGraceWhileAhead() {
    let now = Date(timeIntervalSince1970: 1_000)
    let next = Date(timeIntervalSince1970: 5_005)
    let woken = WakeAccount(number: 4, on: true, lastWakeAt: now, nextDue: next, lastResult: "woke")
    #expect(woken.windowReset(now: now) == Date(timeIntervalSince1970: 5_000))
    #expect(woken.windowReset(now: Date(timeIntervalSince1970: 6_000)) == nil)
    let off = WakeAccount(number: 4, on: false, lastWakeAt: now, nextDue: next, lastResult: "woke")
    #expect(off.windowReset(now: now) == nil)
}

/// A reading another machine took carries `usageFromOtherMachine`; older CLIs omit it.
@Test func decodesUsageFromOtherMachine() throws {
    let row = #"{"schemaVersion":1,"activeAccountNumber":1,"accounts":[{"number":1,"email":"a@x.com","active":true,"usageStatus":"ok","usageAgeSeconds":120,"usageFromOtherMachine":true},{"number":2,"email":"b@x.com","active":false,"usageStatus":"ok"}]}"#
    let list = try CSwapJSON.decoder.decode(AccountList.self, from: Data(row.utf8))
    #expect(list.accounts[0].usageFromOtherMachine == true)
    #expect(list.accounts[1].usageFromOtherMachine == nil)
}
