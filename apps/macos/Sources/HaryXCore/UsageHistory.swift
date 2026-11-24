import Foundation

/// `haryx ai usage … --json` (contracts: `packages/contracts/haryx/usage-*.json`).
public enum UsageHistory {
    public struct Status: Decodable, Equatable, Sendable {
        public let enabled: Bool
        public let needsChoice: Bool
        public let readReplies: Int
        public let keepDays: Int
    }

    public struct Row: Decodable, Equatable, Sendable, Identifiable {
        public let key: String?
        public let replies: Int
        public let input: Int
        public let output: Int
        public let cacheRead: Int
        public let cacheWrite: Int
        public let tokens: Int
        public let cost: Double?
        public let unpricedTokens: Int
        public let share: Double
        public let approximate: Bool
        public let lastActive: Int?
        public let accounts: [Int]
        public var id: String { key ?? "total" }
        public var lastActiveDate: Date? { lastActive.map { Date(timeIntervalSince1970: TimeInterval($0)) } }
    }

    public struct Report: Decodable, Equatable, Sendable {
        public let by: String
        public let since: Int
        public let pricesAsOf: String
        public let rows: [Row]
        public let total: Row
        public let history: Status?
    }

    public struct Reading: Decodable, Equatable, Sendable, Identifiable {
        public let at: Int
        public let account: Int
        public let fiveHour: Double?
        public let sevenDay: Double?
        public var id: String { "\(account)-\(at)" }
        public var date: Date { Date(timeIntervalSince1970: TimeInterval(at)) }
    }

    public struct Quota: Decodable, Equatable, Sendable {
        public let since: Int
        public let readings: [Reading]
        public let history: Status?
    }

    public struct SummaryAccount: Decodable, Equatable, Sendable {
        public let account: Int?
        public let tokens: Int
        public let cost: Double?
        public let share: Double
        public let ranOut: Int
        public let topProject: String?
        public let topProjectShare: Double?
    }

    public struct Summary: Decodable, Equatable, Sendable {
        public let since: Int
        public let pricesAsOf: String
        public let accounts: [SummaryAccount]
        public let total: Row
        public let history: Status?

        /// One line per account, for the weekly notification.
        public func lines(name: (Int) -> String) -> [String] {
            accounts.map { a in
                var line = "\(a.account.map(name) ?? "Unknown account"): \(UsageHistory.shortTokens(a.tokens)) tokens"
                if let cost = a.cost { line += String(format: " (about $%.0f at API prices)", cost) }
                if a.ranOut == 1 { line += ", ran out once" } else if a.ranOut > 1 { line += ", ran out \(a.ranOut) times" }
                return line
            }
        }
    }

    public static func decode<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T {
        try JSONDecoder().decode(T.self, from: data)
    }

    /// 1.2M, 340k, 812.
    public static func shortTokens(_ n: Int) -> String {
        switch n {
        case 1_000_000_000...: String(format: "%.1fB", Double(n) / 1e9)
        case 1_000_000...: String(format: "%.1fM", Double(n) / 1e6)
        case 1_000...: String(format: "%.1fk", Double(n) / 1e3)
        default: "\(n)"
        }
    }
}
