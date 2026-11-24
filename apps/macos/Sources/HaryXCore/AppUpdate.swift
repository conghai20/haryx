import CryptoKit
import Foundation

/// `haryx upgrade --check --json` (contract: `packages/contracts/haryx/upgrade-check.json`).
public struct UpgradeCheck: Decodable, Equatable, Sendable {
    public struct Download: Decodable, Equatable, Sendable {
        public let name: String
        public let url: URL
        public let sha256Url: URL
        public let size: Int64

        enum CodingKeys: String, CodingKey { case name, url, size, sha256Url = "sha256_url" }

        public init(name: String, url: URL, sha256Url: URL, size: Int64) {
            self.name = name; self.url = url; self.sha256Url = sha256Url; self.size = size
        }
    }

    public struct Downloads: Decodable, Equatable, Sendable {
        public let app: Download?
        public let cli: Download?
    }

    /// The CLI's own version (the bundled one matches the app's).
    public let current: String
    public let latest: String?
    public let available: Bool
    public let publishedAt: String?
    public let notes: String?
    public let htmlUrl: URL?
    public let install: String
    public let assets: Downloads
    public let error: String?

    enum CodingKeys: String, CodingKey {
        case current, latest, available, notes, install, assets, error
        case publishedAt = "published_at", htmlUrl = "html_url"
    }

    public static func decode(_ data: Data) throws -> UpgradeCheck {
        try JSONDecoder().decode(UpgradeCheck.self, from: data)
    }

    /// Whether `latest` is newer than the running app (not the CLI, which may be the user's own).
    public func isNewer(than appVersion: String) -> Bool {
        guard let latest else { return false }
        return AppVersion.compare(latest, appVersion) == .orderedDescending
    }
}

/// How much the app does on its own when a release comes out.
public enum UpdateMode: String, CaseIterable, Sendable, Identifiable {
    /// Download in the background; install on the next quit or on "Restart Now".
    case automatic
    /// Check daily and show "Update to vX"; install only when asked.
    case notify
    /// Check only on "Check Now".
    case manual

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .automatic: "Install automatically"
        case .notify: "Notify me"
        case .manual: "Only when I check"
        }
    }

    public var checksInBackground: Bool { self != .manual }
}

public enum AppVersion {
    /// Compare dotted release numbers (`0.1.10` > `0.1.9`); a pre-release suffix sorts below its
    /// release (`1.0.0-rc1` < `1.0.0`). Missing parts count as 0.
    public static func compare(_ a: String, _ b: String) -> ComparisonResult {
        func split(_ v: String) -> ([Int], Bool) {
            let trimmed = v.hasPrefix("v") ? String(v.dropFirst()) : v
            let core = trimmed.prefix { $0.isNumber || $0 == "." }
            let parts = core.split(separator: ".").map { Int($0) ?? 0 }
            return (parts, core.count < trimmed.count)
        }
        let (l, lPre) = split(a), (r, rPre) = split(b)
        for i in 0..<max(l.count, r.count) {
            let x = i < l.count ? l[i] : 0, y = i < r.count ? r[i] : 0
            if x != y { return x < y ? .orderedAscending : .orderedDescending }
        }
        if lPre == rPre { return .orderedSame }
        return lPre ? .orderedAscending : .orderedDescending
    }

    /// The running app's `CFBundleShortVersionString`.
    public static var running: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    }
}

public enum AppUpdateError: LocalizedError, Equatable {
    case noDownload
    case badChecksumFile(String)
    case checksumMismatch(String)
    case notAnApp(String)
    case signature(String)
    case cannotReplace(String)
    case tool(String)

    public var errorDescription: String? {
        switch self {
        case .noDownload: "This release has no checked download for this Mac."
        case .badChecksumFile(let name): "\(name) is not a SHA-256 file; nothing was changed."
        case .checksumMismatch(let name): "\(name) does not match its checksum; nothing was changed."
        case .notAnApp(let name): "\(name) does not contain HaryX.app."
        case .signature(let detail): "The downloaded HaryX.app's signature does not verify: \(detail)"
        case .cannotReplace(let path): "HaryX cannot replace itself at \(path). The disk image is open in Finder: drag HaryX into Applications."
        case .tool(let message): message
        }
    }
}

/// Installs a release DMG over the running app, the way `scripts/install.sh` does: download, check
/// the SHA-256, copy the app out of the image beside the current one, check its signature, clear
/// quarantine, and swap after HaryX quits. Every step but the final swap is undone on failure.
public struct AppUpdater: Sendable {
    public typealias Fetch = @Sendable (URL, URL) async throws -> Void

    /// The running app (`/Applications/HaryX.app`).
    public let appURL: URL
    /// Where downloads and the staged app wait (`~/Library/Caches/dev.haryx.app/updates`).
    public let workDir: URL
    let fetch: Fetch

    public init(appURL: URL = Bundle.main.bundleURL,
                workDir: URL = AppUpdater.defaultWorkDir,
                fetch: @escaping Fetch = AppUpdater.urlSessionFetch) {
        self.appURL = appURL
        self.workDir = workDir
        self.fetch = fetch
    }

    public static var defaultWorkDir: URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("dev.haryx.app/updates", isDirectory: true)
    }

    /// Download `url` to `dest` with URLSession.
    public static let urlSessionFetch: Fetch = { url, dest in
        let (tmp, response) = try await URLSession.shared.download(from: url)
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            throw AppUpdateError.tool("Download failed (HTTP \(http.statusCode)): \(url.lastPathComponent)")
        }
        try? FileManager.default.removeItem(at: dest)
        try FileManager.default.moveItem(at: tmp, to: dest)
    }

    /// Where the new app waits until the swap.
    public var stagedURL: URL { workDir.appendingPathComponent("HaryX.app", isDirectory: true) }

    /// Whether the app can replace itself in place: its folder is writable and macOS has not
    /// moved it to a read-only translocation copy.
    public var canReplaceInPlace: Bool {
        let parent = appURL.deletingLastPathComponent().path
        return !appURL.path.contains("/AppTranslocation/")
            && FileManager.default.isWritableFile(atPath: parent)
            && FileManager.default.isWritableFile(atPath: appURL.path)
    }

    // MARK: download and check

    /// Download the DMG and its checksum into `workDir`; return the DMG once its SHA-256 matches.
    public func download(_ asset: UpgradeCheck.Download) async throws -> URL {
        try FileManager.default.createDirectory(at: workDir, withIntermediateDirectories: true)
        let dmg = workDir.appendingPathComponent(asset.name)
        let sumFile = workDir.appendingPathComponent(asset.name + ".sha256")
        try await fetch(asset.sha256Url, sumFile)
        let text = (try? String(contentsOf: sumFile, encoding: .utf8)) ?? ""
        guard let expected = Self.parseChecksum(text) else {
            throw AppUpdateError.badChecksumFile(sumFile.lastPathComponent)
        }
        try await fetch(asset.url, dmg)
        guard try Self.sha256(of: dmg) == expected else {
            try? FileManager.default.removeItem(at: dmg)
            throw AppUpdateError.checksumMismatch(asset.name)
        }
        return dmg
    }

    /// The hex digest a `.sha256` file names (`<hex>  <name>`).
    public static func parseChecksum(_ text: String) -> String? {
        guard let first = text.split(whereSeparator: \.isWhitespace).first else { return nil }
        let hex = first.lowercased()
        return hex.count == 64 && hex.allSatisfy(\.isHexDigit) ? hex : nil
    }

    public static func sha256(of file: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: file)
        defer { try? handle.close() }
        var hasher = SHA256()
        while let chunk = try handle.read(upToCount: 1 << 20), !chunk.isEmpty {
            hasher.update(data: chunk)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    // MARK: stage

    /// Copy HaryX.app out of the DMG to `stagedURL`, verify its signature and clear quarantine.
    public func stage(dmg: URL) throws -> URL {
        let mount = workDir.appendingPathComponent("mount-\(ProcessInfo.processInfo.processIdentifier)")
        try? FileManager.default.removeItem(at: mount)
        try FileManager.default.createDirectory(at: mount, withIntermediateDirectories: true)
        try Self.tool("/usr/bin/hdiutil", ["attach", "-quiet", "-nobrowse", "-readonly", "-mountpoint", mount.path, dmg.path])
        defer {
            _ = try? Self.tool("/usr/bin/hdiutil", ["detach", "-quiet", "-force", mount.path])
            try? FileManager.default.removeItem(at: mount)
        }
        let source = mount.appendingPathComponent("HaryX.app")
        guard FileManager.default.fileExists(atPath: source.path) else {
            throw AppUpdateError.notAnApp(dmg.lastPathComponent)
        }
        try? FileManager.default.removeItem(at: stagedURL)
        try Self.tool("/usr/bin/ditto", [source.path, stagedURL.path])
        do {
            try Self.tool("/usr/bin/codesign", ["--verify", "--strict", stagedURL.path])
        } catch AppUpdateError.tool(let message) {
            try? FileManager.default.removeItem(at: stagedURL)
            throw AppUpdateError.signature(message)
        }
        _ = try? Self.tool("/usr/bin/xattr", ["-dr", "com.apple.quarantine", stagedURL.path])
        return stagedURL
    }

    // MARK: swap

    /// A shell script that waits for `pid` to exit, swaps the staged app in (keeping the old one
    /// until the new one is in place), and opens it when `relaunch`.
    public static func swapScript(staged: URL, app: URL, pid: Int32, relaunch: Bool) -> String {
        let q = { (s: String) in "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'" }
        let old = app.deletingLastPathComponent().appendingPathComponent(".HaryX.app.old").path
        return """
            while kill -0 \(pid) 2>/dev/null; do sleep 0.2; done
            rm -rf \(q(old))
            mv \(q(app.path)) \(q(old)) || exit 1
            if mv \(q(staged.path)) \(q(app.path)); then rm -rf \(q(old)); else mv \(q(old)) \(q(app.path)); exit 1; fi
            \(relaunch ? "open \(q(app.path))" : ":")
            """
    }

    /// Start the swap in a detached shell; it runs once this process exits.
    public func launchSwap(relaunch: Bool) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = ["-c", Self.swapScript(staged: stagedURL, app: appURL,
                                                   pid: ProcessInfo.processInfo.processIdentifier,
                                                   relaunch: relaunch)]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
    }

    /// Whether an app is staged and newer than the running one.
    public func stagedVersion() -> String? {
        let plist = stagedURL.appendingPathComponent("Contents/Info.plist")
        guard let dict = NSDictionary(contentsOf: plist) else { return nil }
        return dict["CFBundleShortVersionString"] as? String
    }

    /// Show the DMG in Finder (the fallback when the app cannot replace itself).
    public static func openInFinder(_ dmg: URL) {
        _ = try? tool("/usr/bin/open", [dmg.path])
    }

    @discardableResult
    static func tool(_ path: String, _ args: [String]) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: path)
        process.arguments = args
        let out = Pipe(), err = Pipe()
        process.standardOutput = out
        process.standardError = err
        process.standardInput = FileHandle.nullDevice
        try process.run()
        let data = out.fileHandleForReading.readDataToEndOfFile()
        let errText = String(decoding: err.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let name = (path as NSString).lastPathComponent
            let detail = errText.trimmingCharacters(in: .whitespacesAndNewlines)
            throw AppUpdateError.tool("\(name) failed: \(detail.isEmpty ? "status \(process.terminationStatus)" : detail)")
        }
        return String(decoding: data, as: UTF8.self)
    }
}
