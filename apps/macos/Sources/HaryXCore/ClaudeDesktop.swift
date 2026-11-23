import Foundation

// MARK: - `haryx ai desktop status --json`

/// Each account's Claude Desktop data folder (plan: `docs/desktop-follow-plan.md`).
public struct DesktopStatus: Decodable, Sendable, Hashable {
    public let accounts: [DesktopFolder]

    public init(accounts: [DesktopFolder]) { self.accounts = accounts }

    public func folder(_ number: Int) -> DesktopFolder? { accounts.first { $0.number == number } }

    /// The account whose folder Desktop is running on; nil for the default folder or any other.
    public func owner(ofRunningFolder path: String?) -> DesktopFolder? {
        guard let path else { return nil }
        let running = ClaudeDesktop.normalized(path)
        return accounts.first { $0.folder.map(ClaudeDesktop.normalized) == running }
    }
}

public struct DesktopFolder: Decodable, Sendable, Hashable {
    public let number: Int
    public let email: String
    public let uuid: String
    /// nil when the account has no UUID, so it cannot have a folder.
    public let folder: String?
    public let folderExists: Bool
    /// The account the folder last signed into; nil is unknown, never a mismatch.
    public let signedInUuid: String?
    public let mismatch: Bool

    public init(number: Int, email: String, uuid: String, folder: String?, folderExists: Bool,
                signedInUuid: String?, mismatch: Bool) {
        self.number = number; self.email = email; self.uuid = uuid; self.folder = folder
        self.folderExists = folderExists; self.signedInUuid = signedInUuid; self.mismatch = mismatch
    }
}

/// What Claude Desktop can do for one account.
public enum DesktopTarget: Equatable, Sendable {
    /// The folder exists: Desktop can open on it.
    case ready(folder: String)
    /// No folder yet: Desktop opens on a new empty one and the user signs in once.
    case setUp(folder: String)
    /// The folder is signed into another account: leave it alone.
    case mismatch
    /// The account has no UUID (a token slot), or is not in the status.
    case unavailable
}

public enum ClaudeDesktop {
    public static let bundleID = "com.anthropic.claudefordesktop"
    static let userDataFlag = "--user-data-dir"

    public static func target(for number: Int, in status: DesktopStatus) -> DesktopTarget {
        guard let row = status.folder(number), let folder = row.folder else { return .unavailable }
        if row.mismatch { return .mismatch }
        return row.folderExists ? .ready(folder: folder) : .setUp(folder: folder)
    }

    /// The argument that opens Desktop on `folder`.
    public static func launchArguments(folder: String) -> [String] { ["\(userDataFlag)=\(folder)"] }

    /// The data folder a running Desktop was given (`--user-data-dir=X` or `--user-data-dir X`);
    /// nil means its default folder.
    public static func userDataDir(fromArguments args: [String]) -> String? {
        for (i, arg) in args.enumerated().dropFirst() {
            if arg.hasPrefix(userDataFlag + "=") { return String(arg.dropFirst(userDataFlag.count + 1)) }
            if arg == userDataFlag, i + 1 < args.count { return args[i + 1] }
        }
        return nil
    }

    static func normalized(_ path: String) -> String {
        URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath().path
    }

    public static func sameFolder(_ a: String?, _ b: String?) -> Bool {
        guard let a, let b else { return false }
        return normalized(a) == normalized(b)
    }
}

// MARK: - Menu bar status line

/// Whether Claude Desktop runs, and on which data folder (nil: its default folder).
public enum DesktopRun: Equatable, Sendable {
    case notRunning
    case running(folder: String?)
    /// Running, but its arguments could not be read: never touched, never guessed.
    case runningUnknownFolder
}

/// The panel's Desktop line: what Desktop is on, plus at most one button that moves it to the
/// active account.
public struct DesktopLine: Equatable, Sendable {
    public enum Action: Equatable, Sendable {
        /// Open (or quit and reopen) Desktop on the active account's existing folder.
        case open
        /// Open Desktop on a new empty folder for the active account, to sign in once.
        case setUp

        public var title: String { self == .open ? "Open" : "Set Up Desktop" }
    }

    public let text: String
    public let action: Action?

    public init(text: String, action: Action?) { self.text = text; self.action = action }

    /// `name` turns an account number into the name the UI shows.
    public static func make(run: DesktopRun, status: DesktopStatus, active: Int,
                            name: (Int) -> String) -> DesktopLine? {
        let action: Action? = switch ClaudeDesktop.target(for: active, in: status) {
        case .ready: .open
        case .setUp: .setUp
        case .mismatch, .unavailable: nil
        }
        let warning = ClaudeDesktop.target(for: active, in: status) == .mismatch
            ? "\(name(active))'s Desktop folder is signed into another account" : nil
        switch run {
        case .runningUnknownFolder:
            return DesktopLine(text: warning ?? "Desktop: running", action: nil)
        case .notRunning:
            if let warning { return DesktopLine(text: warning, action: nil) }
            return action.map { DesktopLine(text: "Desktop: not running", action: $0) }
        case .running(let folder):
            guard let owner = status.owner(ofRunningFolder: folder) else {
                return DesktopLine(text: warning ?? "Desktop: its own folder, not one of HaryX's", action: action)
            }
            if owner.number == active { return DesktopLine(text: "Desktop: \(name(active))", action: nil) }
            return DesktopLine(text: warning ?? "Desktop still on \(name(owner.number))", action: action)
        }
    }
}
