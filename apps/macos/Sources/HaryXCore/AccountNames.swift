import Foundation

public enum AccountNames {
    /// An almanac row's name: the alias, else the email's local part, else (when another account
    /// shares that local part) the whole email. Every account here shares a domain more often than a
    /// local part, so the local part is what tells rows apart.
    public static func rowName(_ account: Account, among accounts: [Account]) -> String {
        if let alias = account.alias, !alias.isEmpty { return alias }
        let local = localPart(account.email)
        let shared = accounts.contains {
            $0.number != account.number && localPart($0.email).caseInsensitiveCompare(local) == .orderedSame
        }
        return shared ? account.email : local
    }

    /// Whether a row needs its plan label: only when the accounts do not all share one plan. Five
    /// rows that each say "Team" tell nothing and take the room the names need.
    public static func plansVary(_ accounts: [Account]) -> Bool {
        Set(accounts.map { $0.plan?.label ?? "" }).count > 1
    }

    private static func localPart(_ email: String) -> String {
        String(email.split(separator: "@").first ?? Substring(email))
    }
}

extension Usage {
    /// Below this the week is never flagged as the tighter limit: a fresh week at 3% beside an idle
    /// 5-hour window is not a warning.
    public static let tighterWeekFloor: Double = 50

    /// The week has less left than the 5-hour window (and is past ``tighterWeekFloor``): the menu bar
    /// panel leads with the 5-hour figure and marks the week line instead.
    public var weekIsTighter: Bool {
        guard let week = sevenDay?.pct, week >= Self.tighterWeekFloor else { return false }
        return week > (fiveHour?.pct ?? 0)
    }
}

extension AccountGroups {
    /// The menu bar list's sections: `others` (in switch order) under each group once, the active
    /// account's groups first, an account in several groups at its first section only. A list with
    /// no groups is one unnamed section. Switch order runs across groups, so naming a group wherever
    /// the run changed printed the same header twice.
    public static func listSections(_ others: [Account], active: Account?) -> [Section] {
        let all = (active.map { [$0] } ?? []) + others
        guard inUse(all) else { return others.isEmpty ? [] : [Section(name: nil, accounts: others)] }
        var seen = Set<Int>()
        return sections(all, excludingActive: true).compactMap { section in
            let fresh = section.accounts.filter { seen.insert($0.number).inserted }
            return fresh.isEmpty ? nil : Section(name: section.name, accounts: fresh)
        }
    }
}
