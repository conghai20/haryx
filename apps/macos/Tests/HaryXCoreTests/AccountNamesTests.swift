import Foundation
import Testing
@testable import HaryXCore

private func named(_ number: Int, _ email: String, alias: String? = nil, plan: String? = nil,
                   groups: [String] = [], active: Bool = false) throws -> Account {
    var json: [String: Any] = ["number": number, "email": email, "active": active, "usageStatus": "ok"]
    if let alias { json["alias"] = alias }
    if let plan { json["plan"] = ["label": plan] }
    if let first = groups.first { json["group"] = first; json["groups"] = groups }
    return try CSwapJSON.decoder.decode(Account.self, from: JSONSerialization.data(withJSONObject: json))
}

@Test func rowNameUsesTheLocalPartUnlessAnotherAccountSharesIt() throws {
    let a = try named(1, "hieunguyen@gmail.com")
    let b = try named(2, "dep1.pj02@corp.example")
    let c = try named(3, "Dep1.PJ02@other.example")
    let d = try named(4, "x@y.z", alias: "work")
    let all = [a, b, c, d]
    #expect(AccountNames.rowName(a, among: all) == "hieunguyen")
    #expect(AccountNames.rowName(b, among: all) == "dep1.pj02@corp.example")
    #expect(AccountNames.rowName(c, among: all) == "Dep1.PJ02@other.example")
    #expect(AccountNames.rowName(d, among: all) == "work")
}

@Test func plansShowOnlyWhenTheyDiffer() throws {
    let team = [try named(1, "a@x", plan: "Team"), try named(2, "b@x", plan: "Team")]
    #expect(!AccountNames.plansVary(team))
    #expect(AccountNames.plansVary(team + [try named(3, "c@x", plan: "Pro")]))
}

@Test func weekIsTighterOnlyPastTheFloorAndAboveTheFiveHour() throws {
    let base = try decodeFixture().accounts[0]
    #expect(try account(base, five: 26, seven: 75).usage?.weekIsTighter == true)
    #expect(try account(base, five: nil, seven: 60).usage?.weekIsTighter == true)
    #expect(try account(base, five: 90, seven: 75).usage?.weekIsTighter == false)
    // A fresh week beside an idle 5-hour window is not a warning.
    #expect(try account(base, five: 0, seven: 3).usage?.weekIsTighter == false)
}

@Test func listSectionsNameEachGroupOnceWithTheActiveGroupFirst() throws {
    let active = try named(9, "me@x", groups: ["Phung's"], active: true)
    // Switch order interleaves the groups: Personal, Phung's, Personal, Phung's.
    let others = [
        try named(1, "a@x", groups: ["Personal"]), try named(2, "b@x", groups: ["Phung's"]),
        try named(3, "c@x", groups: ["Personal"]), try named(4, "d@x", groups: ["Phung's", "Personal"]),
        try named(5, "e@x"),
    ]
    let sections = AccountGroups.listSections(others, active: active)
    #expect(sections.map(\.name) == ["Phung's", "Personal", nil])
    #expect(sections[0].accounts.map(\.number) == [2, 4], "switch order kept inside a group")
    #expect(sections[1].accounts.map(\.number) == [1, 3], "an account in two groups is listed once")
}

@Test func listSectionsWithoutGroupsIsOneUnnamedSection() throws {
    let sections = AccountGroups.listSections([try named(1, "a@x"), try named(2, "b@x")], active: nil)
    #expect(sections.count == 1 && sections[0].name == nil && sections[0].accounts.count == 2)
    #expect(AccountGroups.listSections([], active: nil).isEmpty)
}

@Test func wakeSkippedOnAnUnswitchableAccountSaysSo() {
    let skipped = WakeAccount(number: 1, on: true, lastWakeAt: nil, nextDue: nil, lastResult: "skip:disabled")
    #expect(skipped.summary() == "wake skipped: not switchable")
}
