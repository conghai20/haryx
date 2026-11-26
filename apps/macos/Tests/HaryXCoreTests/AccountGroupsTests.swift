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
