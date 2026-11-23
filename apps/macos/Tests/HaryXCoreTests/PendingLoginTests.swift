import Foundation
import Testing
@testable import HaryXCore

private func row(_ email: String, status: String = "ok", expires: String? = nil) throws -> Account {
    var json: [String: Any] = ["number": 1, "email": email, "active": false, "usageStatus": status]
    if let expires { json["loginExpiresAt"] = expires }
    let data = try JSONSerialization.data(withJSONObject: json)
    return try CSwapJSON.decoder.decode(Account.self, from: data)
}

@Test func reloginWaitsUntilLoginChanges() throws {
    let before = try row("a@x.com", status: "relogin_required", expires: "2026-11-20T00:00:00Z")
    let start = Date()
    let pending = PendingLogin.relogin(before, now: start)
    #expect(pending.targets(before))
    #expect(pending.resolve(in: [before], now: start) == .waiting)

    let renewed = try row("a@x.com", status: "relogin_required", expires: "2027-11-20T00:00:00Z")
    #expect(pending.resolve(in: [renewed], now: start) == .done(renewed))

    let cleared = try row("a@x.com", status: "ok", expires: "2026-11-20T00:00:00Z")
    #expect(pending.resolve(in: [cleared], now: start) == .done(cleared))
}

@Test func reloginTimesOut() throws {
    let before = try row("a@x.com", status: "relogin_required")
    let start = Date()
    let pending = PendingLogin.relogin(before, now: start)
    #expect(pending.resolve(in: [before], now: start.addingTimeInterval(PendingLogin.timeout + 1)) == .timedOut)
}

@Test func addIsDoneWhenANewAccountAppears() throws {
    let a = try row("a@x.com")
    let pending = PendingLogin.add(existing: [a])
    #expect(pending.resolve(in: [a]) == .waiting)
    let b = try row("b@x.com")
    #expect(pending.resolve(in: [a, b]) == .done(b))
    #expect(!pending.targets(a))
}

@Test func tokenSlotsCannotSignInAgain() throws {
    #expect(try row("a@x.com").canRelogin)
    #expect(try !row("setup-token-3@token.local").canRelogin)
    #expect(try !row("k@x.com", status: "api_key").canRelogin)
}

@Test func loginDueWithinAWeek() throws {
    let now = try #require(CSwapJSON.parseDate("2026-11-20T00:00:00Z"))
    #expect(try row("a@x.com", expires: "2026-11-25T00:00:00Z").loginDue(now: now))
    #expect(try !row("a@x.com", expires: "2026-12-25T00:00:00Z").loginDue(now: now))
    #expect(try !row("a@x.com").loginDue(now: now))
}
