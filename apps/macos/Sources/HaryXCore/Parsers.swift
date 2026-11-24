import Foundation

/// Parsers for the few cswap outputs that have no `--json` form.
public enum TextParsers {
    /// `cswap priority` rows: `  Account-3  me@x.com  10`
    public static func priorities(_ text: String) -> [Int: Int] {
        var result: [Int: Int] = [:]
        for line in text.split(separator: "\n") {
            guard let m = line.wholeMatch(of: #/\s*Account-(\d+)\s+.*?\s(-?\d+)\s*/#),
                  let slot = Int(m.1), let tier = Int(m.2) else { continue }
            result[slot] = tier
        }
        return result
    }

    /// `cswap unclaimed` rows: `<id>  slot <slot>  <reason>`
    public static func unclaimed(_ text: String) -> [UnclaimedEntry] {
        text.split(separator: "\n").compactMap { line in
            guard let m = line.wholeMatch(of: #/(\S+)\s+slot\s+(\S+)\s+(.*)/#) else { return nil }
            return UnclaimedEntry(id: String(m.1), slot: String(m.2), reason: String(m.3))
        }
    }
}

/// POSIX single-quote escaping for building shell command lines.
public func shellQuote(_ s: String) -> String {
    "'" + s.replacingOccurrences(of: "'", with: #"'\''"#) + "'"
}

// MARK: - `cswap auto --json` events

public struct AutoEvent: Sendable, Hashable, Identifiable {
    public enum Severity: Sendable { case info, notable, problem }

    public let id = UUID()
    public let kind: String
    public let date: Date
    public let summary: String
    public let severity: Severity
    /// Title for a user notification, when the event deserves one.
    public let notificationTitle: String?
    /// For a switch: the account it left (sessions still running there may need continuing).
    public let fromAccount: Int?

    public init(kind: String, date: Date = Date(), summary: String, severity: Severity, notificationTitle: String? = nil,
                fromAccount: Int? = nil) {
        self.kind = kind; self.date = date; self.summary = summary; self.severity = severity; self.notificationTitle = notificationTitle
        self.fromAccount = fromAccount
    }

    /// Parses one stdout line. Returns nil for non-JSON lines and for noise (`sleep`).
    public static func parse(line: String) -> AutoEvent? {
        guard let data = line.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let kind = obj["event"] as? String else { return nil }
        let date = (obj["ts"] as? String).flatMap(CSwapJSON.parseDate) ?? Date()
        func ref(_ key: String) -> String {
            guard let r = obj[key] as? [String: Any] else { return "(none)" }
            let num = r["number"].map { "\($0)" } ?? "?"
            return "Account-\(num)" + ((r["email"] as? String).map { " (\($0))" } ?? "")
        }
        let str = { (key: String) in obj[key] as? String ?? "" }

        switch kind {
        case "poll":
            let headroom = (obj["headroom"] as? [String: Any])?.count ?? 0
            return AutoEvent(kind: kind, date: date, summary: "Checked \(headroom) account(s); active \(ref("active"))",
                             severity: .info, notificationTitle: nil)
        case "switch":
            let dry = (obj["dryRun"] as? Bool) == true ? " [dry run]" : ""
            let from = (obj["from"] as? [String: Any])?["number"]
            let fromAccount = (from as? Int) ?? (from as? String).flatMap(Int.init)
            return AutoEvent(kind: kind, date: date, summary: "Switched \(ref("from")) → \(ref("to")) (\(str("trigger")))\(dry)",
                             severity: .notable, notificationTitle: nil, // list diff notifies switches
                             fromAccount: fromAccount)
        case "no-switch":
            let detail = str("detail").isEmpty ? "" : " — \(str("detail"))"
            return AutoEvent(kind: kind, date: date, summary: "No switch: \(str("reason"))\(detail)",
                             severity: .info, notificationTitle: nil)
        case "account-quarantined":
            return AutoEvent(kind: kind, date: date, summary: "Account-\(obj["number"] ?? "?") (\(str("email"))) quarantined: \(str("reason"))",
                             severity: .problem, notificationTitle: "Account quarantined")
        case "account-unquarantined":
            return AutoEvent(kind: kind, date: date, summary: "Account-\(obj["number"] ?? "?") (\(str("email"))) back in rotation",
                             severity: .notable, notificationTitle: "Account back in rotation")
        case "all-exhausted":
            let reset = (obj["earliestResetAt"] as? String).flatMap(CSwapJSON.parseDate)
                .map { "; earliest reset \($0.formatted(date: .abbreviated, time: .shortened))" } ?? ""
            return AutoEvent(kind: kind, date: date, summary: "All accounts exhausted\(reset)",
                             severity: .problem, notificationTitle: "All Claude accounts exhausted")
        case "error":
            let retry = (obj["transient"] as? Bool) == true ? " (will retry)" : ""
            return AutoEvent(kind: kind, date: date, summary: "Error: \(str("message"))\(retry)",
                             severity: .problem, notificationTitle: (obj["transient"] as? Bool) == true ? nil : "Auto-switch error")
        case "sleep":
            return nil
        default:
            return AutoEvent(kind: kind, date: date, summary: kind, severity: .info, notificationTitle: nil)
        }
    }
}
