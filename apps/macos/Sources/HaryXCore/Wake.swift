import Foundation

// MARK: - `haryx ai wake status --json`

/// What `haryx ai wake` knows: settings, the background agent and each account's last check.
public struct WakeStatus: Decodable, Sendable, Hashable {
    public let quietHours: String?
    public let weeklyCapPct: Double
    public let agentInstalled: Bool
    public let accounts: [WakeAccount]
    /// The last wake events, oldest first; nil from a haryx that does not report them.
    public let events: [WakeEvent]?

    public init(quietHours: String?, weeklyCapPct: Double, agentInstalled: Bool, accounts: [WakeAccount],
                events: [WakeEvent]? = nil) {
        self.quietHours = quietHours; self.weeklyCapPct = weeklyCapPct
        self.agentInstalled = agentInstalled; self.accounts = accounts; self.events = events
    }

    public func account(_ number: Int) -> WakeAccount? { accounts.first { $0.number == number } }

    /// Newest first, at most `limit` (the Windows app shows 20).
    public func recentEvents(limit: Int = 20) -> [WakeEvent] {
        Array((events ?? []).sorted { $0.at > $1.at }.prefix(limit))
    }

    /// A result code in words, as the Windows app shows it: "Woke", "Not due", "Skipped: weekly cap".
    public static func resultText(_ code: String?) -> String {
        guard let code, !code.isEmpty else { return "—" }
        if code == "woke" { return "Woke" }
        if code == "not-due" { return "Not due" }
        let parts = code.split(separator: ":", maxSplits: 1)
        // "disabled" means the account is held out of rotation (Off), not that wake is off for it.
        let reason = parts.count > 1
            ? (parts[1] == "disabled" ? "account is off" : parts[1].replacingOccurrences(of: "-", with: " "))
            : ""
        switch parts.first {
        case "skip": return "Skipped: \(reason)"
        case "failed": return "Failed: \(reason)"
        default: return code
        }
    }
}

/// One line of the wake log: when, which account (slot number as text), and the result code.
public struct WakeEvent: Decodable, Sendable, Hashable {
    /// Seconds since 1970.
    public let at: Double
    public let account: String?
    public let result: String

    public var date: Date { Date(timeIntervalSince1970: at) }
    public var failed: Bool { result.hasPrefix("failed") }
}

public struct WakeAccount: Decodable, Sendable, Hashable {
    public let number: Int
    public let on: Bool
    public let lastWakeAt: Date?
    public let nextDue: Date?
    /// `woke`, `not-due`, `skip:<reason>` or `failed:<kind>`.
    public let lastResult: String?

    public init(number: Int, on: Bool, lastWakeAt: Date?, nextDue: Date?, lastResult: String?) {
        self.number = number; self.on = on; self.lastWakeAt = lastWakeAt; self.nextDue = nextDue; self.lastResult = lastResult
    }

    /// Seconds the CLI adds after a window's reset before it counts the account as due.
    static let dueGraceSeconds: TimeInterval = 5

    /// When the 5-hour window the last check knows about resets, if still ahead. The usage
    /// endpoint can report a window a wake just started as 0% with no reset time; this fills in.
    public func windowReset(now: Date = Date()) -> Date? {
        guard on, let next = nextDue, next > now else { return nil }
        return next.addingTimeInterval(-Self.dueGraceSeconds)
    }

    /// A short line for the UI: "wakes 14:05", "woke 09:00", "skipped: weekly cap".
    public func summary(now: Date = Date()) -> String? {
        guard on else { return nil }
        if let r = lastResult, r.hasPrefix("failed:") { return "wake failed (\(r.dropFirst(7)))" }
        if lastResult == "skip:login" { return "wake needs sign-in" }
        if let r = lastResult, r.hasPrefix("skip:") {
            return "wake skipped: " + r.dropFirst(5).replacingOccurrences(of: "-", with: " ")
        }
        if let next = nextDue, next > now { return "wakes " + next.formatted(date: .omitted, time: .shortened) }
        if let woke = lastWakeAt { return "woke " + woke.formatted(date: .omitted, time: .shortened) }
        return "wake on"
    }
}
