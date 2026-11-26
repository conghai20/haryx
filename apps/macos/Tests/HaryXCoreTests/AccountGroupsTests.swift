import Foundation
import Testing
@testable import HaryXCore

private func member(_ number: Int, _ group: String?, active: Bool = false) throws -> Account {
    var json: [String: Any] = ["number": number, "email": "a\(number)@x.com", "active": active, "usageStatus": "ok"]
    if let group { json["group"] = group }
    return try CSwapJSON.decoder.decode(Account.self, from: JSONSerialization.data(withJSONObject: json))
}

@Test func groupsSectionTheActiveGroupFirstThenByNameThenUngrouped() throws {
    let ordered = [
        try member(2, "Home", active: true), try member(1, nil), try member(3, "client"),
        try member(4, "home"), try member(5, "Beta"),
    ]
    let sections = AccountGroups.sections(ordered)
    #expect(sections.map(\.name) == ["Home", "Beta", "client", nil])
    #expect(sections[0].accounts.map(\.number) == [2, 4], "case-insensitive, given order kept")
    #expect(sections.last?.accounts.map(\.number) == [1])
}

@Test func ungroupedComesFirstWhenTheActiveAccountIsUngrouped() throws {
    let sections = AccountGroups.sections([try member(1, nil, active: true), try member(2, "Work")])
    #expect(sections.map(\.name) == [nil, "Work"])
}

@Test func groupNamesAreDistinctIgnoringCaseAndSorted() throws {
    let accounts = [try member(1, "work"), try member(2, "Work"), try member(3, "Alpha"), try member(4, nil)]
    #expect(AccountGroups.names(accounts) == ["Alpha", "work"])
    #expect(AccountGroups.inUse(accounts))
    #expect(!AccountGroups.inUse([try member(1, nil)]))
    #expect(try member(1, "Work").sharesGroup(with: try member(2, "WORK")))
    #expect(try member(1, nil).sharesGroup(with: try member(2, nil)))
}

private func reading(_ number: Int, _ group: String?, five: Double, disabled: Bool = false) throws -> Account {
    var json: [String: Any] = ["number": number, "email": "a\(number)@x.com", "active": number == 1, "usageStatus": "ok",
                               "usage": ["fiveHour": ["pct": five]], "disabled": disabled]
    if let group { json["group"] = group }
    return try CSwapJSON.decoder.decode(Account.self, from: JSONSerialization.data(withJSONObject: json))
}

@Test func handoffPrefersTheGroupAndLeavesItOnlyWhenNoneThereCanTakeIt() throws {
    let from = try reading(1, "Work", five: 100)
    let roomy = try reading(3, "Home", five: 5)
    let mate = try reading(2, "work", five: 40)
    #expect(AccountGroups.handoffTarget([from, mate, roomy], from: from)?.number == 2)
    let fullMate = try reading(2, "Work", five: 100)
    let offMate = try reading(4, "Work", five: 1, disabled: true)
    #expect(AccountGroups.handoffTarget([from, fullMate, offMate, roomy], from: from)?.number == 3)
    #expect(AccountGroups.handoffTarget([from, mate, roomy], from: nil)?.number == 3, "no source: most room")
    #expect(AccountGroups.handoffTarget([from, fullMate], from: from) == nil)
}

@Test func typedGroupNamesAreTrimmedBoundedAndJoinAnExistingSpelling() throws {
    let accounts = [try member(1, "Work"), try member(2, nil)]
    #expect(AccountGroups.normalizedName("  work ", among: accounts) == "Work", "joins the stored spelling")
    #expect(AccountGroups.normalizedName(" Home ", among: accounts) == "Home")
    #expect(AccountGroups.normalizedName("   ", among: accounts) == nil)
    #expect(AccountGroups.normalizedName(String(repeating: "x", count: 32), among: accounts) != nil)
    #expect(AccountGroups.normalizedName(String(repeating: "x", count: 33), among: accounts) == nil)
}

@Test func sectionsCanLeaveTheActiveAccountOutAndDropEmptiedSections() throws {
    let ordered = [try member(1, "Solo", active: true), try member(2, "Work"), try member(3, "work")]
    let sections = AccountGroups.sections(ordered, excludingActive: true)
    #expect(sections.map(\.name) == ["Work"], "Solo held only the active account")
    #expect(sections[0].accounts.map(\.number) == [2, 3])
    let withMate = AccountGroups.sections([try member(1, "Work", active: true), try member(2, "Work")], excludingActive: true)
    #expect(withMate.map(\.name) == ["Work"])
    #expect(withMate[0].accounts.map(\.number) == [2])
}

private func multi(_ number: Int, _ groups: [String], active: Bool = false) throws -> Account {
    var json: [String: Any] = ["number": number, "email": "a\(number)@x.com", "active": active, "usageStatus": "ok"]
    if let first = groups.first { json["group"] = first; json["groups"] = groups }
    return try CSwapJSON.decoder.decode(Account.self, from: JSONSerialization.data(withJSONObject: json))
}

@Test func anAccountInSeveralGroupsSharesAnyOfThemAndShowsInEachSection() throws {
    let a = try multi(1, ["Work", "Home"], active: true)
    let b = try multi(2, ["home"]), c = try multi(3, ["Client"]), d = try multi(4, [])
    #expect(a.sharesGroup(with: b))
    #expect(!a.sharesGroup(with: c))
    #expect(!a.sharesGroup(with: d))
    #expect(a.groupLabel == "Work, Home")
    #expect(d.groupLabel == "Ungrouped")
    let sections = AccountGroups.sections([a, b, c, d])
    #expect(sections.map(\.name) == ["Home", "Work", "Client", nil], "the active account's groups first")
    #expect(sections[0].accounts.map(\.number) == [1, 2])
    #expect(sections[1].accounts.map(\.number) == [1])
    #expect(AccountGroups.names([a, b, c]) == ["Client", "Home", "Work"])
}

@Test func anOlderCLIsSingleGroupStillReads() throws {
    let old = try member(1, "Work")
    #expect(old.groupNames == ["Work"])
    #expect(old.isInGroup("work"))
}

@Test func filterByGroupMatchesAnyOfAnAccountsGroups() throws {
    let accounts = [try multi(1, ["Work", "Home"]), try multi(2, ["Home"]), try multi(3, [])]
    #expect(AccountFilter(group: .named("work")).apply(accounts).map(\.number) == [1])
    #expect(AccountFilter(group: .named("home")).apply(accounts).map(\.number) == [1, 2])
    #expect(AccountFilter(group: .ungrouped).apply(accounts).map(\.number) == [3])
}
