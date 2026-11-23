import Foundation

/// Puts the `haryx` bundled in the app (the DMG build's `Contents/Resources/bin/haryx`) on the user's
/// PATH as a symlink, the way editors install their shell command. A link (not a copy) keeps the CLI in
/// step with the app: updating HaryX.app updates `haryx`. Another `haryx` (Homebrew, cargo, pipx, npm)
/// is never replaced.
public struct CLIInstaller: Sendable {
    /// Where the system-wide link goes; on the default macOS PATH (`/etc/paths`), needs an admin password.
    public static let systemLink = "/usr/local/bin/haryx"
    /// Where the per-user link goes; no password, but the folder must be on the shell's PATH.
    public static var userLink: String { CSwapCLI.home + "/.local/bin/haryx" }

    public enum State: Equatable, Sendable {
        /// This build carries no CLI (a development build).
        case notBundled
        /// A link to the bundled CLI exists at this path.
        case linked(String)
        /// Another `haryx` is installed here; it is left alone.
        case otherInstall(String)
        case notInstalled
    }

    public enum InstallError: LocalizedError, Equatable {
        case notBundled
        case occupied(String)

        public var errorDescription: String? {
            switch self {
            case .notBundled: "This build of HaryX does not include the haryx command."
            case .occupied(let path): "\(path) already exists and is not a HaryX link; it was left alone."
            }
        }
    }

    public let bundled: String?
    public let links: [String]
    /// Folders searched for another `haryx` (PATH-like), besides the link locations.
    public let otherPaths: [String]

    public init(bundled: String?, links: [String] = [Self.systemLink, Self.userLink],
                otherPaths: [String] = ["/opt/homebrew/bin", CSwapCLI.home + "/.cargo/bin"]) {
        self.bundled = bundled
        self.links = links
        self.otherPaths = otherPaths
    }

    /// The CLI inside the running app, when it has one.
    public static var current: CLIInstaller {
        let path = Bundle.main.resourceURL?.appendingPathComponent("bin/haryx").path
        let exists = path.map { FileManager.default.isExecutableFile(atPath: $0) } ?? false
        return CLIInstaller(bundled: exists ? path : nil)
    }

    /// Whether `path` is a symlink that resolves to the bundled CLI.
    public func isOurLink(_ path: String) -> Bool {
        guard let bundled,
              let dest = try? FileManager.default.destinationOfSymbolicLink(atPath: path) else { return false }
        let absolute = dest.hasPrefix("/") ? dest : ((path as NSString).deletingLastPathComponent as NSString).appendingPathComponent(dest)
        return Self.canonical(absolute) == Self.canonical(bundled)
    }

    private static func canonical(_ path: String) -> String {
        URL(fileURLWithPath: path).resolvingSymlinksInPath().standardizedFileURL.path
    }

    /// A non-link file, or a link elsewhere (a dangling HaryX link from a moved app does not count).
    private func occupied(_ path: String) -> Bool {
        let fm = FileManager.default
        if let dest = try? fm.destinationOfSymbolicLink(atPath: path) {
            return !isOurLink(path) && !dest.contains("HaryX.app/") && fm.fileExists(atPath: path)
        }
        return fm.fileExists(atPath: path)
    }

    public var state: State {
        if let linked = links.first(where: isOurLink) { return .linked(linked) }
        let candidates = links + otherPaths.map { $0 + "/haryx" }
        if let other = candidates.first(where: { FileManager.default.isExecutableFile(atPath: $0) && !isOurLink($0) }) {
            return .otherInstall(other)
        }
        return bundled == nil ? .notBundled : .notInstalled
    }

    /// Whether to offer the install on launch.
    public var shouldOffer: Bool { state == .notInstalled }

    /// Link at a path the user can write (the per-user location). Replaces only a link of ours, or a
    /// dangling one left by a moved app.
    public func installLink(at path: String) throws {
        guard let bundled else { throw InstallError.notBundled }
        if occupied(path) { throw InstallError.occupied(path) }
        let fm = FileManager.default
        try fm.createDirectory(atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        if (try? fm.destinationOfSymbolicLink(atPath: path)) != nil { try fm.removeItem(atPath: path) }
        try fm.createSymbolicLink(atPath: path, withDestinationPath: bundled)
    }

    /// Remove a link of ours; anything else is left alone.
    public func removeLink(at path: String) throws {
        guard isOurLink(path) else { throw InstallError.occupied(path) }
        try FileManager.default.removeItem(atPath: path)
    }

    /// The shell command that links at a root-owned path, for `do shell script … with administrator
    /// privileges`. Paths are single-quoted; `ln -h` never follows an existing link.
    public func adminInstallCommand(at path: String) throws -> String {
        guard let bundled else { throw InstallError.notBundled }
        if occupied(path) { throw InstallError.occupied(path) }
        let dir = (path as NSString).deletingLastPathComponent
        return "/bin/mkdir -p \(Self.quote(dir)) && /bin/ln -sfh \(Self.quote(bundled)) \(Self.quote(path))"
    }

    public func adminRemoveCommand(at path: String) throws -> String {
        guard isOurLink(path) else { throw InstallError.occupied(path) }
        return "/bin/rm -f \(Self.quote(path))"
    }

    /// Single-quote for /bin/sh.
    public static func quote(_ s: String) -> String {
        "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    /// Whether a folder is on a PATH string.
    public static func onPath(_ dir: String, path: String?) -> Bool {
        (path ?? "").split(separator: ":").contains { String($0) == dir }
    }
}
