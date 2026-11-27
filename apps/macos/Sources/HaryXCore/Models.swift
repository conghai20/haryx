import Foundation

// MARK: - `cswap list --json`

public struct AccountList: Decodable, Sendable {
    public let schemaVersion: Int
    public let activeAccountNumber: Int?
    public let accounts: [Account]
    /// Will the week last (`crate::rotation` in the CLI); absent from older CLIs and when no account
    /// auto-switch may use has a weekly reading.
    public let rotation: Rotation?
}

public struct Account: Decodable, Sendable, Identifiable, Hashable {
    public let number: Int
    public let email: String
    public let organizationName: String?
    public let organizationUuid: String?
    public let active: Bool
    public let usageStatus: String
    public let usage: Usage?
    public let alias: String?
    /// `haryx group`: the first group, nil when ungrouped. Older CLIs send only this.
    public let group: String?
    /// Every group the account is in, in the order added (`groups` in `list --json`).
    let groups: [String]?
    public let disabled: Bool?
    public let threshold: Double?
    public let loginExpiresAt: Date?
    public let plan: Plan?
    public let usageAgeSeconds: Double?
    /// The reading was taken by another of your machines (usage share through HaryX sync).
    public let usageFromOtherMachine: Bool?
    public let usageError: String?

    public var id: Int { number }
    /// Stable across slot moves: cswap identifies accounts by (email, org).
    public var identityKey: String { "\(email)|\(organizationUuid ?? "")" }
    public var displayName: String { alias ?? email }
    public var isDisabled: Bool { disabled ?? false }
    public var isAPIKey: Bool { usageStatus == "api_key" }
    public var needsRelogin: Bool { ["relogin_required", "no_credentials", "foreign_credential"].contains(usageStatus) }
    /// Token and API-key slots (`add-token`) hold no Claude login, so `login` cannot renew them.
    public var canRelogin: Bool { !isAPIKey && !email.hasSuffix("@token.local") }
    /// The stored login expires within `window` (default: the CLI's 7-day notice).
    public func loginDue(now: Date = Date(), window: TimeInterval = 7 * 86_400) -> Bool {
        guard let exp = loginExpiresAt else { return false }
        return exp.timeIntervalSince(now) < window
    }

    /// Highest utilisation across the 5h and 7d windows (nil if unknown).
    public var peakPct: Double? {
        [usage?.fiveHour?.pct, usage?.sevenDay?.pct].compactMap { $0 }.max()
    }

    /// At 100% of a quota: nothing left until it resets.
    public var isExhausted: Bool { (peakPct ?? 0) >= 100 }

    /// Out of use for now, held out of rotation or at its limit: dimmed, with a sleeping mascot. An
    /// account at its limit stays in rotation and wakes when its quota resets.
    public var isResting: Bool { isDisabled || isExhausted }

    public var statusNote: String? {
        switch usageStatus {
        case "ok": nil
        case "token_expired": "Token expired — refreshes automatically"
        case "api_key": "API key (no quota)"
        case "keychain_unavailable": "Keychain locked or busy"
        case "relogin_required": "Re-login needed: use Sign In Again"
        case "foreign_credential": "Credential belongs to another account"
        case "no_credentials": "No stored credentials: use Sign In Again"
        default: "Usage unavailable"
        }
    }
}

public struct Usage: Decodable, Sendable, Hashable {
    public let fiveHour: UsageWindow?
    public let sevenDay: UsageWindow?
    public let scoped: [ScopedWindow]?
}

public struct UsageWindow: Decodable, Sendable, Hashable {
    public let pct: Double?
    public let resetsAt: Date?
    public let countdown: String?
    public let clock: String?
    public let expectedPct: Double?
    public let aheadOfPace: Bool?
    public let projectedExhaustionAt: Date?
    public let willLastToReset: Bool?

    public init(pct: Double?, resetsAt: Date? = nil, countdown: String? = nil, clock: String? = nil, expectedPct: Double? = nil,
                aheadOfPace: Bool? = nil, projectedExhaustionAt: Date? = nil, willLastToReset: Bool? = nil) {
        self.pct = pct; self.resetsAt = resetsAt; self.countdown = countdown; self.clock = clock; self.expectedPct = expectedPct
        self.aheadOfPace = aheadOfPace; self.projectedExhaustionAt = projectedExhaustionAt; self.willLastToReset = willLastToReset
    }
}

public struct ScopedWindow: Decodable, Sendable, Hashable {
    public let name: String
    public let pct: Double?
    public let resetsAt: Date?
}

public struct Plan: Decodable, Sendable, Hashable {
    public let label: String?
    public let expired: Bool?
}

// MARK: - `cswap switch --json`

public struct SwitchResult: Decodable, Sendable {
    public let switched: Bool
    public let to: AccountRef?
}

public struct AccountRef: Decodable, Sendable, Hashable {
    public let number: Int?
    public let email: String?

    enum CodingKeys: String, CodingKey { case number, email }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        email = try c.decodeIfPresent(String.self, forKey: .email)
        // cswap emits the slot as int in list output but as string in some event payloads.
        if let n = try? c.decodeIfPresent(Int.self, forKey: .number) {
            number = n
        } else {
            number = (try? c.decodeIfPresent(String.self, forKey: .number)).flatMap { Int($0) }
        }
    }
}

// MARK: - `cswap config list --json`

public struct ConfigList: Decodable, Sendable {
    public let path: String
    public let settings: [ConfigEntry]
}

public struct ConfigEntry: Decodable, Sendable, Identifiable, Hashable {
    public let key: String
    public let value: JSONScalar
    public let isSet: Bool
    public var id: String { key }
}

public enum JSONScalar: Decodable, Sendable, Hashable, CustomStringConvertible {
    case string(String), number(Double), bool(Bool), null

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else { self = .string(try c.decode(String.self)) }
    }

    public var description: String {
        switch self {
        case .string(let s): s
        case .number(let n): n.rounded() == n ? String(Int(n)) : String(n)
        case .bool(let b): b ? "true" : "false"
        case .null: ""
        }
    }
}

// MARK: - mappings.json (read-only)

public struct DirectoryMapping: Sendable, Identifiable, Hashable {
    public let path: String
    public let email: String
    public let organizationUuid: String
    public var id: String { path }
    public var identityKey: String { "\(email)|\(organizationUuid)" }
}

public struct UnclaimedEntry: Sendable, Identifiable, Hashable {
    public let id: String
    public let slot: String
    public let reason: String
}

// MARK: - JSON decoding

public enum CSwapJSON {
    public static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .custom { decoder in
            let raw = try decoder.singleValueContainer().decode(String.self)
            guard let date = parseDate(raw) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Bad date \(raw)"))
            }
            return date
        }
        return d
    }()

    /// cswap emits both `…Z` and `….497815+00:00` (microseconds), which ISO8601DateFormatter can't
    /// take as-is, so the fraction is trimmed to milliseconds first.
    public static func parseDate(_ raw: String) -> Date? {
        let trimmed = raw.replacingOccurrences(of: #"(\.\d{3})\d+"#, with: "$1", options: .regularExpression)
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return withFraction.date(from: trimmed) ?? ISO8601DateFormatter().date(from: trimmed)
    }

    public static func mappings(from data: Data) -> [DirectoryMapping] {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let table = root["mappings"] as? [String: [String: Any]] else { return [] }
        return table.map { path, entry in
            DirectoryMapping(path: path,
                             email: entry["email"] as? String ?? "",
                             organizationUuid: entry["organizationUuid"] as? String ?? "")
        }.sorted { $0.path < $1.path }
    }
}
