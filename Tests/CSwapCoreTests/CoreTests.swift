import Foundation
import Testing
@testable import CSwapCore

/// Trimmed from a real `cswap list --json` (1.1.0); emails anonymised.
let listFixture = """
{"schemaVersion":1,"activeAccountNumber":3,"accounts":[
 {"number":1,"email":"a@x.com","organizationName":"Org","organizationUuid":"o1","isOrganization":true,"active":false,
  "usageStatus":"ok","usage":{"fiveHour":{"pct":0.0},"sevenDay":{"pct":50.0,"resetsAt":"2026-11-25T11:00:00.497815+00:00",
  "countdown":"4d 20h","clock":"Nov 25 18:00","expectedPct":30.8,"aheadOfPace":true,"projectedExhaustionAt":"2026-11-22T18:19:28Z",
  "willLastToReset":false}},"loginExpiresAt":"2026-12-14T03:30:48Z","plan":{"label":"Team","expired":false},"usageAgeSeconds":158.8},
 {"number":3,"email":"c@x.com","organizationUuid":"o1","active":true,"usageStatus":"ok","alias":"work",
  "usage":{"fiveHour":{"pct":88.0,"resetsAt":"2026-11-20T15:29:59.930003+00:00","countdown":"47m"},"sevenDay":{"pct":77.0}}},
 {"number":4,"email":"d@x.com","active":false,"usageStatus":"relogin_required","usage":null,"disabled":true}
]}
"""

func decodeFixture() throws -> AccountList {
    try CSwapJSON.decoder.decode(AccountList.self, from: Data(listFixture.utf8))
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
