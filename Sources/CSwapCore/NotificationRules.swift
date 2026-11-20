import Foundation

public struct Notice: Sendable, Hashable {
    public enum Category: String, Sendable, CaseIterable {
        case switched, usage, login, reset, session, autoSwitch
    }

    /// Dedupe key: the same condition in the same reset window fires once.
    public let key: String
    public let category: Category
    public let title: String
    public let body: String

    public init(key: String, category: Category, title: String, body: String) {
        self.key = key; self.category = category; self.title = title; self.body = body
    }
}

/// Pure diffing of two `list --json` snapshots into user-facing notices.
public enum NotificationRules {
    public static let usageLevels: [Double] = [80, 95]
    public static let exhaustedPct: Double = 95
    public static let recoveredPct: Double = 50
    public static let expiryWarning: TimeInterval = 3 * 86_400

    /// Notices whose keys carry no time component; the notifier re-arms them once the condition clears.
    public static let rearmingPrefixes = ["relogin:", "plan:"]

    /// Minute precision: cswap's reset times jitter by sub-seconds between polls.
    static func windowKey(_ date: Date?) -> String {
        date.map { String(Int(($0.timeIntervalSince1970 / 60).rounded())) } ?? "-"
    }

    public static func evaluate(previous: [Account], current: [Account], now: Date = Date()) -> [Notice] {
        var notices: [Notice] = []
        let prevByID = Dictionary(previous.map { ($0.identityKey, $0) }, uniquingKeysWith: { a, _ in a })

        if let active = current.first(where: \.active),
           let before = previous.first(where: \.active), before.identityKey != active.identityKey {
            notices.append(Notice(key: "switch:\(active.identityKey):\(Int(now.timeIntervalSince1970))", category: .switched,
                                  title: "Switched to \(active.displayName)",
                                  body: "From \(before.displayName). \(usageLine(active))"))
        }

        for account in current {
            let name = account.displayName
            if account.active {
                notices += usageNotices(account)
            }
            if account.usageStatus == "relogin_required" {
                notices.append(Notice(key: "relogin:\(account.identityKey)", category: .login,
                                      title: "\(name) needs a re-login",
                                      body: "Log in with Claude Code as this account, then choose Add current login."))
            }
            if let exp = account.loginExpiresAt, exp.timeIntervalSince(now) < expiryWarning, exp > now {
                notices.append(Notice(key: "expiry:\(account.identityKey):\(exp.timeIntervalSince1970)", category: .login,
                                      title: "\(name) login expires soon",
                                      body: "Expires \(exp.formatted(.relative(presentation: .named))). Re-add it after logging in again."))
            }
            if account.plan?.expired == true {
                notices.append(Notice(key: "plan:\(account.identityKey)", category: .login,
                                      title: "\(name) plan expired", body: "Its subscription is no longer active."))
            }
            if let before = prevByID[account.identityKey] {
                notices += resetNotices(before: before, after: account)
            }
        }
        return notices
    }

    static func usageNotices(_ a: Account) -> [Notice] {
        var out: [Notice] = []
        for (label, window) in [("5-hour", a.usage?.fiveHour), ("weekly", a.usage?.sevenDay)] {
            guard let window, let pct = window.pct else { continue }
            let resetKey = windowKey(window.resetsAt)
            if let level = usageLevels.last(where: { pct >= $0 }) {
                let reset = window.countdown.map { " Resets in \($0)." } ?? ""
                out.append(Notice(key: "usage:\(a.identityKey):\(label):\(Int(level)):\(resetKey)", category: .usage,
                                  title: "\(a.displayName) at \(Int(pct))% of \(label) limit",
                                  body: "Crossed \(Int(level))%.\(reset)"))
            }
            if window.aheadOfPace == true, window.willLastToReset == false, let runOut = window.projectedExhaustionAt {
                out.append(Notice(key: "pace:\(a.identityKey):\(label):\(resetKey)", category: .usage,
                                  title: "\(a.displayName) may run out early",
                                  body: "At this pace the \(label) quota runs out \(runOut.formatted(date: .abbreviated, time: .shortened)), before it resets."))
            }
        }
        return out
    }

    static func resetNotices(before: Account, after: Account) -> [Notice] {
        var out: [Notice] = []
        for (label, old, new) in [("5-hour", before.usage?.fiveHour, after.usage?.fiveHour),
                                  ("weekly", before.usage?.sevenDay, after.usage?.sevenDay)] {
            guard let o = old?.pct, let n = new?.pct, o >= exhaustedPct, n < recoveredPct else { continue }
            let resetKey = windowKey(old?.resetsAt)
            out.append(Notice(key: "reset:\(after.identityKey):\(label):\(resetKey)", category: .reset,
                              title: "\(after.displayName) is ready again",
                              body: "Its \(label) quota has reset (\(Int(n))% used)."))
        }
        return out
    }

    public static func usageLine(_ a: Account) -> String {
        let parts = [("5h", a.usage?.fiveHour?.pct), ("7d", a.usage?.sevenDay?.pct)]
            .compactMap { label, pct in pct.map { "\(label) \(Int($0))%" } }
        return parts.isEmpty ? (a.statusNote ?? "") : parts.joined(separator: " · ")
    }
}
