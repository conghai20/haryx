import Foundation

/// The Accounts pane's filter: by group and by rotation status. Kept per window, never synced.
public struct AccountFilter: Equatable, Sendable {
    public enum Group: Hashable, Sendable {
        case all
        case ungrouped
        /// A group by its case-insensitive key (lowercased name).
        case named(String)
    }

    public enum Status: String, CaseIterable, Identifiable, Sendable {
        case all, on, off
        public var id: String { rawValue }
        public var title: String {
            switch self {
            case .all: "All"
            case .on: "On (in rotation)"
            case .off: "Off (held out)"
            }
        }
    }

    public var group: Group
    public var status: Status

    public init(group: Group = .all, status: Status = .all) {
        self.group = group
        self.status = status
    }

    public var isActive: Bool { self != AccountFilter() }

    public func matches(_ a: Account) -> Bool {
        switch group {
        case .all: break
        case .ungrouped: if !a.groupNames.isEmpty { return false }
        case .named(let key): if !a.isInGroup(key) { return false }
        }
        switch status {
        case .all: return true
        case .on: return !a.isDisabled
        case .off: return a.isDisabled
        }
    }

    public func apply(_ accounts: [Account]) -> [Account] { accounts.filter(matches) }

    /// The same filter with a group that no account carries any more reset to all, so a filter
    /// never hides everything because its group was emptied.
    public func pruned(for accounts: [Account]) -> AccountFilter {
        guard case .named(let key) = group, !accounts.contains(where: { $0.isInGroup(key) }) else { return self }
        return AccountFilter(group: .all, status: status)
    }
}
