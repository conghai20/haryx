import Foundation

/// Account groups (`haryx group`): a label the CLI stores on each account. Names match ignoring
/// case, and ungrouped accounts form a group of their own, as the CLI's automatic switching treats
/// them.
public enum AccountGroups {
    /// One dashboard section: a group's accounts, in the order they were given.
    public struct Section: Identifiable, Sendable, Equatable {
        /// The group's name as stored; nil for the ungrouped accounts.
        public let name: String?
        public let accounts: [Account]
        /// Case-insensitive, so it survives a rename that only changes case.
        public var id: String { Account.groupKey(name) }
    }

    /// Whether any account carries a group. With none, the dashboard shows no sections.
    public static func inUse(_ accounts: [Account]) -> Bool {
        accounts.contains { $0.group != nil }
    }

    /// The groups in use, sorted by name, each in the first spelling met.
    public static func names(_ accounts: [Account]) -> [String] {
        var seen: [String: String] = [:]
        for a in accounts { if let g = a.group, seen[a.groupKey] == nil { seen[a.groupKey] = g } }
        return seen.values.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    }

    /// `ordered` split into sections: the active account's group first, then the other groups by
    /// name, then the ungrouped accounts. Each section keeps the order it was given.
    public static func sections(_ ordered: [Account]) -> [Section] {
        let activeKey = ordered.first(where: \.active)?.groupKey
        let keys = ordered.map(\.groupKey).reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
        let sorted = keys.sorted { l, r in
            if (l == activeKey) != (r == activeKey) { return l == activeKey }
            if l.isEmpty != r.isEmpty { return r.isEmpty }
            return l < r
        }
        return sorted.map { key in
            let members = ordered.filter { $0.groupKey == key }
            return Section(name: members.first?.group, accounts: members)
        }
    }
}

extension AccountGroups {
    /// Where a session that ran out on `from` should continue: an account that can take it (enabled,
    /// signed in, below 100%), other than `from`, with the most headroom. One in `from`'s group wins
    /// over any outside it; only when none there can take it does the pick leave the group, as the
    /// CLI's automatic picks do. `from` nil leaves groups out of it.
    public static func handoffTarget(_ accounts: [Account], from: Account?) -> Account? {
        let usable = accounts.filter {
            $0.number != from?.number && !$0.isDisabled && !$0.needsRelogin && ($0.peakPct ?? 100) < 100
        }
        let mates = from.map { f in usable.filter { $0.sharesGroup(with: f) } } ?? []
        return (mates.isEmpty ? usable : mates).min { ($0.peakPct ?? 100) < ($1.peakPct ?? 100) }
    }
}

extension Account {
    /// The group, lowercased; "" when ungrouped.
    public var groupKey: String { Account.groupKey(group) }

    static func groupKey(_ group: String?) -> String { (group ?? "").lowercased() }

    /// In the same group as `other` (both ungrouped counts).
    public func sharesGroup(with other: Account) -> Bool { groupKey == other.groupKey }
}
