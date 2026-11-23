import Foundation

/// A `haryx login` the app started in a terminal, and how to tell from `list --json` when it landed.
///
/// The terminal is outside the app's control, so completion is read from the account list: a
/// re-login is done once the slot's login changed (new expiry, or "re-login needed" cleared); a new
/// sign-in is done once an account that was not there before appears.
public struct PendingLogin: Sendable, Equatable, Identifiable {
    public enum Kind: Sendable, Equatable {
        case relogin(identityKey: String, previousExpiry: Date?, neededRelogin: Bool)
        case add(knownKeys: Set<String>)
    }

    public enum Resolution: Sendable, Equatable {
        case waiting
        case done(Account)
        case timedOut
    }

    /// How long a sign-in may take before the app stops waiting for it.
    public static let timeout: TimeInterval = 10 * 60

    public let id: UUID
    public let kind: Kind
    public let startedAt: Date

    public init(kind: Kind, startedAt: Date = Date(), id: UUID = UUID()) {
        self.id = id
        self.kind = kind
        self.startedAt = startedAt
    }

    public static func relogin(_ account: Account, now: Date = Date()) -> PendingLogin {
        PendingLogin(kind: .relogin(identityKey: account.identityKey,
                                    previousExpiry: account.loginExpiresAt,
                                    neededRelogin: account.needsRelogin),
                     startedAt: now)
    }

    public static func add(existing accounts: [Account], now: Date = Date()) -> PendingLogin {
        PendingLogin(kind: .add(knownKeys: Set(accounts.map(\.identityKey))), startedAt: now)
    }

    /// Whether this pending login is for `account`.
    public func targets(_ account: Account) -> Bool {
        if case .relogin(let key, _, _) = kind { return key == account.identityKey }
        return false
    }

    public func resolve(in accounts: [Account], now: Date = Date()) -> Resolution {
        if let landed = landed(in: accounts) { return .done(landed) }
        return now.timeIntervalSince(startedAt) > Self.timeout ? .timedOut : .waiting
    }

    private func landed(in accounts: [Account]) -> Account? {
        switch kind {
        case .relogin(let key, let previousExpiry, let neededRelogin):
            guard let account = accounts.first(where: { $0.identityKey == key }) else { return nil }
            let renewed = account.loginExpiresAt != nil && account.loginExpiresAt != previousExpiry
            let cleared = neededRelogin && !account.needsRelogin
            return renewed || cleared ? account : nil
        case .add(let known):
            return accounts.first { !known.contains($0.identityKey) }
        }
    }
}
