import Foundation

// The HaryX account and encrypted sync, as the `haryx` CLI reports them
// (`packages/contracts/haryx/{whoami,login-events,sync-status,sync-result}.json`).
// The app never holds a HaryX token or the sync key: it only reads this JSON.

/// `haryx whoami --json` and `haryx logout --json`.
public struct WhoAmI: Decodable, Sendable, Equatable {
    public let signedIn: Bool
    public let user: HaryXUser?
    /// Absent from an older `haryx` and when signed out.
    public let consents: [ConsentState]?

    public static let signedOut = WhoAmI(signedIn: false, user: nil)

    public init(signedIn: Bool, user: HaryXUser?, consents: [ConsentState]? = nil) {
        self.signedIn = signedIn
        self.user = user
        self.consents = consents
    }
}

public struct HaryXUser: Decodable, Sendable, Equatable {
    public let id: String
    public let email: String
    /// `github`, `google`, `apple` or `email`.
    public let provider: String

    public var providerName: String {
        switch provider {
        case "github": "GitHub"
        case "google": "Google"
        case "apple": "Apple"
        case "email": "Email"
        default: provider.capitalized
        }
    }
}

/// One line of `haryx login --provider … --json`.
public enum LoginEvent: Equatable, Sendable {
    case browser(url: URL?, opened: Bool)
    case signedIn(HaryXUser)
    case error(code: String, message: String)

    private struct Raw: Decodable {
        let event: String
        let url: String?
        let opened: Bool?
        let user: HaryXUser?
        let code: String?
        let message: String?
    }

    public static func parse(line: String) -> LoginEvent? {
        guard let raw = try? JSONDecoder().decode(Raw.self, from: Data(line.utf8)) else { return nil }
        switch raw.event {
        case "browser": return .browser(url: raw.url.flatMap(URL.init(string:)), opened: raw.opened ?? false)
        case "signed_in": return raw.user.map(LoginEvent.signedIn)
        case "error": return .error(code: raw.code ?? "unknown", message: raw.message ?? "Sign-in failed.")
        default: return nil
        }
    }
}

/// A Claude account set up on another machine and not on this one.
public struct OtherMachineAccount: Decodable, Sendable, Hashable, Identifiable {
    public let email: String
    public let organizationName: String?
    public let alias: String?
    public var id: String { email + "|" + (organizationName ?? "") }
}

/// A value sync chose between two machines; the other side's value was lost.
public struct SyncOverwrite: Decodable, Sendable, Hashable, Identifiable {
    public let at: Date
    public let kind: String
    public let label: String
    /// `local` (this machine's value won) or `remote`.
    public let kept: String

    public var id: String { "\(at.timeIntervalSince1970)|\(kind)|\(label)" }
    public var keptLocal: Bool { kept == "local" }

    private enum CodingKeys: String, CodingKey { case at, kind, label, kept }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        at = Date(timeIntervalSince1970: try c.decode(Double.self, forKey: .at))
        kind = try c.decode(String.self, forKey: .kind)
        label = try c.decode(String.self, forKey: .label)
        kept = try c.decode(String.self, forKey: .kept)
    }
}

/// `haryx sync status --json`: local only, no network.
public struct SyncStatus: Decodable, Sendable, Equatable {
    public let signedIn: Bool
    public let unlocked: Bool
    public let lastSyncAt: Date?
    public let otherMachineAccounts: [OtherMachineAccount]
    public let overwritten: [SyncOverwrite]
}

/// `haryx sync --json` on success.
public struct SyncResult: Decodable, Sendable {
    public let pushed: Int
    public let pulled: Int
    /// Kinds that changed this machine: `settings`, `app-prefs`, `account`, `credential`.
    public let changed: [String]
    public let overwritten: [SyncOverwrite]
    public let otherMachineAccounts: [OtherMachineAccount]
}

/// `{"ok": true, "hasPassphrase": …}`
public struct PassphraseCheck: Decodable, Sendable { public let hasPassphrase: Bool }
/// `{"ok": true, "created": …}`
public struct UnlockResult: Decodable, Sendable { public let created: Bool? }
/// `{"ok": true}` and other replies whose only news is success.
public struct OKReply: Decodable, Sendable {}

/// A failed account or sync command: `{"ok": false, "code", "message"}`, or a CLI that printed
/// something else (too old to know the command, crashed).
public struct HaryXFailure: Error, Sendable, Equatable {
    public let code: String
    public let message: String

    public init(code: String, message: String) {
        self.code = code
        self.message = message
    }

    private struct Envelope: Decodable {
        let ok: Bool?
        let code: String?
        let message: String?
    }

    /// Decodes a reply: an `ok: false` envelope is a failure whatever the exit status; anything
    /// else must decode as `T`, else the CLI's error text becomes an `unexpected` failure.
    public static func decode<T: Decodable>(_ type: T.Type, stdout: Data, status: Int32, stderr: String) -> Result<T, HaryXFailure> {
        if let env = try? JSONDecoder().decode(Envelope.self, from: stdout), env.ok == false {
            return .failure(HaryXFailure(code: env.code ?? "unknown", message: env.message ?? "The command failed."))
        }
        if let value = try? CSwapJSON.decoder.decode(T.self, from: stdout) { return .success(value) }
        let result = CLIResult(status: status, stdout: stdout, stderr: stderr)
        return .failure(HaryXFailure(code: "unexpected", message: CSwapCLI.errorMessage(result)))
    }
}

extension CSwapCLI {
    /// Runs an account or sync command and decodes its JSON reply (see `HaryXFailure.decode`).
    public func reply<T: Decodable>(_ type: T.Type, _ args: [String], stdin: String? = nil) async -> Result<T, HaryXFailure> {
        do {
            let r = try await run(args, stdin: stdin)
            return HaryXFailure.decode(T.self, stdout: r.stdout, status: r.status, stderr: r.stderr)
        } catch {
            return .failure(HaryXFailure(code: "unexpected", message: error.localizedDescription))
        }
    }
}

/// What a failed sync means for the account pane. Failures are quiet: a status line, never an alert.
public enum SyncProblem: Equatable, Sendable {
    /// Not signed in on this machine: show the sign-in state.
    case signedOut
    /// The sign-in server ended this machine's session (signed out from HaryX on the web or the
    /// WeCoAp account page): show the sign-in state with that notice.
    case signedOutFromWeb
    /// The HaryX ID has not given (or withdrew) its consent to sync: ask for it again.
    case consentRequired
    /// No sync key on this machine: show the passphrase form.
    case locked
    /// The passphrase was reset on another machine: show the passphrase form with that reason.
    case keyChanged
    /// Another sync holds the lock: try again later.
    case busy
    /// The server could not be reached or failed: try again later.
    case unavailable
    case failed(String)

    public init(_ failure: HaryXFailure) {
        switch failure.code {
        case "not_signed_in": self = .signedOut
        case "signed_out": self = .signedOutFromWeb
        case "consent_required": self = .consentRequired
        case "locked": self = .locked
        case "key_changed": self = .keyChanged
        case "busy": self = .busy
        case "network", "server": self = .unavailable
        default: self = .failed(failure.message)
        }
    }

    /// The status line shown under "Sync Now".
    public var summary: String {
        switch self {
        case .signedOut: "Signed out."
        case .signedOutFromWeb: "Signed out of HaryX from the web."
        case .consentRequired: "Sync is off for your HaryX account. Turn it on to sync."
        case .locked: "Enter your sync passphrase to sync."
        case .keyChanged: "The sync passphrase was changed on another machine. Enter the new one."
        case .busy: "Another sync is running. Will retry."
        case .unavailable: "Sync unavailable, will retry."
        case .failed(let message): message
        }
    }

    /// The pane should ask for the passphrase again.
    public var needsUnlock: Bool { self == .locked || self == .keyChanged }

    /// The session is gone: re-read who is signed in.
    public var isSignedOut: Bool { self == .signedOut || self == .signedOutFromWeb }
}

/// The rules the passphrase form enforces before `sync unlock` / `sync passphrase`.
public enum SyncPassphrase {
    public static let minimumLength = 12

    /// Why `passphrase` (typed twice when `confirm` is non-nil) cannot be set yet; nil when it can.
    public static func problem(_ passphrase: String, confirm: String?) -> String? {
        if passphrase.count < minimumLength { return "Use at least \(minimumLength) characters." }
        if passphrase.contains(where: \.isNewline) { return "The passphrase cannot contain a line break." }
        if let confirm, confirm != passphrase { return "The two passphrases do not match." }
        return nil
    }
}

// MARK: - consents, machines, usage sync
// (`packages/contracts/haryx/{consents,machines,usage-sync}.json`; `whoami.json` carries `consents`.)

/// A HaryX consent purpose: `sync` or `machines`.
public typealias ConsentPurpose = String

/// One entry of `whoami.json`'s `consents`.
public struct ConsentState: Decodable, Sendable, Equatable {
    public let purpose: ConsentPurpose
    public let granted: Bool
    public let version: String?
}

public struct ConsentEvent: Decodable, Sendable, Equatable {
    /// `grant` or `withdraw`.
    public let action: String
    public let version: String
    public let at: String
    /// `cli`, `macos`, `windows`, `web` or `sso`.
    public let source: String
}

public struct Consent: Decodable, Sendable, Equatable {
    public let purpose: ConsentPurpose
    public let granted: Bool
    public let version: String?
    public let at: String?
    public let history: [ConsentEvent]
}

/// `haryx account consents --json`.
public struct Consents: Decodable, Sendable, Equatable {
    public let consents: [Consent]
}

public struct HaryXMachine: Decodable, Sendable, Equatable {
    public let machineId: String
    public let name: String
    public let os: String
    public let version: String
    public let lastSeenAt: String
    public let isThisMachine: Bool
    public let signedIn: Bool
    /// The machine's Keycloak session id (the web ends that session); unused by the app.
    public let sessionId: String?
}

/// `haryx account machines --json`.
public struct HaryXMachines: Decodable, Sendable, Equatable {
    public let machines: [HaryXMachine]
}
