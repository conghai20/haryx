import Foundation

/// Account groups (`haryx group`): labels the CLI stores on each account; an account can be in
/// several. Names match ignoring case. Two accounts are in the same group when they share any
/// group, and ungrouped accounts form a group of their own, as the CLI's automatic switching
/// treats them.
public enum AccountGroups {
    /// One dashboard section: a group's accounts, in the order they were given. An account in
    /// several groups shows in each of their sections.
    public struct Section: Identifiable, Sendable, Equatable {
        /// The group's name as stored; nil for the ungrouped accounts.
        public let name: String?
        public let accounts: [Account]
        /// Case-insensitive, so it survives a rename that only changes case.
        public var id: String { Account.groupKey(name) }
    }

    /// Whether any account carries a group. With none, the dashboard shows no sections.
    public static func inUse(_ accounts: [Account]) -> Bool {
        accounts.contains { !$0.groupNames.isEmpty }
    }

    /// The groups in use, sorted by name, each in the first spelling met.
    public static func names(_ accounts: [Account]) -> [String] {
        var seen: [String: String] = [:]
        for a in accounts {
            for g in a.groupNames where seen[g.lowercased()] == nil { seen[g.lowercased()] = g }
        }
        return seen.values.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    }

    /// The CLI's limit on a group name (`GROUP_MAX_CHARS`).
    public static let maxNameLength = 32

    /// A typed name as the CLI would store it: trimmed, and in the existing group's spelling when it
    /// matches one ignoring case. Nil when empty or longer than `maxNameLength`.
    public static func normalizedName(_ typed: String, among accounts: [Account]) -> String? {
        let name = typed.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty, name.count <= maxNameLength else { return nil }
        return names(accounts).first { $0.lowercased() == name.lowercased() } ?? name
    }

    /// `ordered` split into sections: the active account's groups first (by name), then the other
    /// groups by name, then the ungrouped accounts. Each section keeps the order it was given.
    /// `excludingActive` leaves the active account out of its sections (it has its own card) while
    /// still ordering by its groups; a section left empty is dropped.
    public static func sections(_ ordered: [Account], excludingActive: Bool = false) -> [Section] {
        let activeKeys = ordered.first(where: \.active)?.groupKeys ?? []
        let activeUngrouped = ordered.first(where: \.active)?.groupNames.isEmpty ?? false
        var spelling: [String: String] = [:]
        for a in ordered { for g in a.groupNames where spelling[g.lowercased()] == nil { spelling[g.lowercased()] = g } }
        let hasUngrouped = ordered.contains { $0.groupNames.isEmpty }
        let keys = Array(spelling.keys) + (hasUngrouped ? [""] : [])
        let sorted = keys.sorted { l, r in
            let lActive = l.isEmpty ? activeUngrouped : activeKeys.contains(l)
            let rActive = r.isEmpty ? activeUngrouped : activeKeys.contains(r)
            if lActive != rActive { return lActive }
            if l.isEmpty != r.isEmpty { return r.isEmpty }
            return l < r
        }
        return sorted.compactMap { key in
            let members = ordered.filter { key.isEmpty ? $0.groupNames.isEmpty : $0.groupKeys.contains(key) }
            let shown = excludingActive ? members.filter { !$0.active } : members
            return shown.isEmpty ? nil : Section(name: key.isEmpty ? nil : spelling[key], accounts: shown)
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
    /// Every group the account is in: `groups`, or `group` alone from an older CLI.
    public var groupNames: [String] { groups ?? group.map { [$0] } ?? [] }

    /// The groups, lowercased.
    public var groupKeys: Set<String> { Set(groupNames.map { $0.lowercased() }) }

    /// Its groups for a header or a list: "Work, Home", or "Ungrouped".
    public var groupLabel: String { groupNames.isEmpty ? "Ungrouped" : groupNames.joined(separator: ", ") }

    /// Whether the account is in `name` (ignoring case).
    public func isInGroup(_ name: String) -> Bool { groupKeys.contains(name.lowercased()) }

    static func groupKey(_ group: String?) -> String { (group ?? "").lowercased() }

    /// In the same group as `other`: they share any group, or both are ungrouped.
    public func sharesGroup(with other: Account) -> Bool {
        let mine = groupKeys, theirs = other.groupKeys
        return mine.isEmpty && theirs.isEmpty || !mine.isDisjoint(with: theirs)
    }
}
