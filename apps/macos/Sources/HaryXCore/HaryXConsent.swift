import Foundation

// The HaryX consents the app asks for, the WeCoAp pages it links to, and the words both use.
// The Windows app says the same (`apps/windows/src/lib/consent.ts`); change the two together.

/// The pages HaryX opens on the web. The Windows app allows exactly these (`WECOAP_LINKS`).
public enum HaryXLinks {
    public static let web = URL(string: "https://haryx.wecoap.com/platform/")!
    /// Where a WeCoAp ID, which is the HaryX account, is deleted (plan A10).
    public static let wecoapAccount = URL(string: "https://sso.wecoap.com/realms/wecoap/account/")!
    public static let privacyPolicy = URL(string: "https://www.wecoap.com/en/privacy-policy/")!
}

/// The privacy policy version a grant records. It is the CLI's `POLICY_VERSION`
/// (`cli/crates/haryx-core/src/account/api.rs`): bump them together.
public let privacyPolicyVersion = "2026-11-01"

/// A consent the app asks for; the raw value is the CLI's purpose (`account consent … sync`).
public enum ConsentKind: String, CaseIterable, Sendable, Identifiable {
    case sync, machines
    public var id: String { rawValue }

    /// The consent sheet's title. `device` is "Mac" or "PC".
    public func title(device: String) -> String {
        switch self {
        case .sync: "Turn on sync?"
        case .machines: "Show this \(device) on HaryX web?"
        }
    }

    /// What the user agrees to, one point per line.
    public func points(device: String) -> [String] {
        switch self {
        case .sync:
            [
                "HaryX uploads your settings, app preferences, account names and groups, API keys and tokens, setup profiles, and daily usage totals per account and model. Your Claude logins and your projects never leave this \(device).",
                "Everything is end-to-end encrypted with your sync passphrase before it leaves this \(device). HaryX cannot read it.",
                "The encrypted copy is stored on HaryX's servers in Vietnam.",
                "You can withdraw this consent at any time in HaryX Account. HaryX then stops sync on every machine and deletes its copy.",
            ]
        case .machines:
            [
                "HaryX web lists this \(device) with its name, system, HaryX version and when it was last seen, so you can rename it or sign it out from your browser.",
                "This list is not encrypted: HaryX can read it. It is stored on HaryX's servers in Vietnam.",
                "Turn it off at any time. HaryX then deletes your machines from HaryX web. Sign-in and sync are not affected.",
            ]
        }
    }

    /// The sheet's confirm button.
    public var agree: String {
        switch self {
        case .sync: "Agree and Turn On Sync"
        case .machines: "Agree and Show"
        }
    }

    /// The withdraw confirmation's title.
    public var withdrawTitle: String {
        switch self {
        case .sync: "Withdraw your consent to sync?"
        case .machines: "Stop showing your machines on HaryX web?"
        }
    }

    /// What withdrawing erases, said before it does (the CLI's own words).
    public var withdrawMessage: String {
        switch self {
        case .sync:
            "Withdrawing Sync stops sync on every machine and deletes HaryX's server copy of your synced data: settings, accounts, credentials, profile files and usage. What is on each machine stays."
        case .machines:
            "Withdrawing it deletes your machines from HaryX web: their names, systems, versions and when they were last seen. Sign-in and sync are not affected."
        }
    }

    public var withdrawButton: String {
        switch self {
        case .sync: "Withdraw and Delete"
        case .machines: "Stop Showing"
        }
    }

    /// `haryx account consent grant|withdraw <purpose> --yes --source <source> --json`.
    public func arguments(grant: Bool, source: String) -> [String] {
        ["account", "consent", grant ? "grant" : "withdraw", rawValue, "--yes", "--source", source, "--json"]
    }
}

extension WhoAmI {
    /// Whether `kind` is granted; nil when the CLI could not say (offline, older CLI, signed out).
    public func consent(_ kind: ConsentKind) -> Bool? {
        guard signedIn, let consents else { return nil }
        return consents.first { $0.purpose == kind.rawValue }?.granted ?? false
    }

    /// The same account with the consents a grant or withdraw answered.
    public func with(_ updated: Consents) -> WhoAmI {
        WhoAmI(signedIn: signedIn, user: user, consents: updated.consents.map {
            ConsentState(purpose: $0.purpose, granted: $0.granted, version: $0.version)
        })
    }
}

/// `haryx account export --json`: `{"ok": true, "path": …}`.
public struct ExportResult: Decodable, Sendable { public let path: String }

/// The Usage pane's line on where usage data lives, which depends on the Sync consent.
public enum UsagePrivacy {
    /// `syncing`: signed in, the Sync consent granted and sync on for this machine.
    public static func kept(device: String, syncing: Bool) -> String {
        syncing
            ? "Daily totals per account and model sync end-to-end encrypted to your other machines and HaryX web; projects and sessions stay on this \(device)."
            : "It stays on this \(device)."
    }
}
