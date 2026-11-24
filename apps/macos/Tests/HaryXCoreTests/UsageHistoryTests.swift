import Foundation
import Testing
@testable import HaryXCore

private func contract(_ name: String) throws -> Data {
    try Data(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        .appendingPathComponent("../../../../packages/contracts/haryx/\(name)"))
}

@Test func decodesTheUsageReport() throws {
    let r = try UsageHistory.decode(UsageHistory.Report.self, contract("usage-report.json"))
    #expect(r.by == "account")
    #expect(r.rows.count == 2)
    #expect(r.rows[0].cost == 157.64)
    #expect(r.rows[1].cost == nil)
    #expect(r.total.key == nil)
    #expect(r.history?.needsChoice == false)
}

@Test func decodesQuotaAndSummary() throws {
    let q = try UsageHistory.decode(UsageHistory.Quota.self, contract("usage-quota.json"))
    #expect(q.readings.count == 3)
    #expect(q.readings[2].fiveHour == nil)
    let s = try UsageHistory.decode(UsageHistory.Summary.self, contract("usage-summary.json"))
    #expect(s.accounts[0].ranOut == 3)
    let lines = s.lines { "Account \($0)" }
    #expect(lines[0] == "Account 2: 339.2M tokens (about $158 at API prices), ran out 3 times")
    #expect(lines[1].hasPrefix("Unknown account: 134.0k tokens"))
}

@Test func shortensTokenCounts() {
    #expect(UsageHistory.shortTokens(812) == "812")
    #expect(UsageHistory.shortTokens(340_000) == "340.0k")
    #expect(UsageHistory.shortTokens(2_500_000_000) == "2.5B")
}

@Test func aSwitchEventNamesTheAccountItLeft() {
    let line = #"{"event":"switch","ts":"2026-11-24T01:00:00Z","from":{"number":1,"email":"a@x.com"},"to":{"number":"2"},"trigger":"threshold"}"#
    #expect(AutoEvent.parse(line: line)?.fromAccount == 1)
    let poll = #"{"event":"poll","headroom":{}}"#
    #expect(AutoEvent.parse(line: poll)?.fromAccount == nil)
}
