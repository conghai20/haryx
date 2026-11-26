import Foundation
import Testing
@testable import HaryXCore

private func acct(_ number: Int, _ group: String?, disabled: Bool = false) throws -> Account {
    var json: [String: Any] = ["number": number, "email": "a\(number)@x.com", "active": false,
                               "usageStatus": "ok", "disabled": disabled]
    if let group { json["group"] = group }
    return try CSwapJSON.decoder.decode(Account.self, from: JSONSerialization.data(withJSONObject: json))
}

@Test func filterByGroupMatchesIgnoringCaseAndKeepsUngroupedApart() throws {
    let accounts = [try acct(1, "Work"), try acct(2, "work"), try acct(3, nil), try acct(4, "Home")]
    #expect(AccountFilter(group: .named("WORK")).apply(accounts).map(\.number) == [1, 2])
    #expect(AccountFilter(group: .ungrouped).apply(accounts).map(\.number) == [3])
    #expect(AccountFilter().apply(accounts).count == 4)
    #expect(!AccountFilter().isActive)
}

@Test func filterByStatusSplitsOnAndOffAndCombinesWithGroup() throws {
    let accounts = [try acct(1, "Work"), try acct(2, "Work", disabled: true), try acct(3, nil, disabled: true)]
    #expect(AccountFilter(status: .on).apply(accounts).map(\.number) == [1])
    #expect(AccountFilter(status: .off).apply(accounts).map(\.number) == [2, 3])
    #expect(AccountFilter(group: .named("work"), status: .off).apply(accounts).map(\.number) == [2])
    #expect(AccountFilter(status: .off).isActive)
}

@Test func filterOnAnEmptiedGroupFallsBackToAllGroups() throws {
    let accounts = [try acct(1, "Home")]
    let filter = AccountFilter(group: .named("work"), status: .on)
    #expect(filter.pruned(for: accounts) == AccountFilter(group: .all, status: .on))
    #expect(AccountFilter(group: .named("home")).pruned(for: accounts).group == .named("home"))
}
