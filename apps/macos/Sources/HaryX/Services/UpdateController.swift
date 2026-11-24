import AppKit
import HaryXCore
import Observation
import Security

/// Finds and installs new HaryX releases. The lookup is `haryx upgrade --check --json` (the bundled
/// haryx when the app carries one), so the app and the CLI read the same cached answer; the
/// install is `AppUpdater` (download, SHA-256, signature, swap after quit).
@MainActor @Observable
final class UpdateController {
    enum Phase: Equatable {
        case idle
        case checking
        case downloading
        /// The new app waits beside this one; it goes in on quit or "Restart Now".
        case ready(String)
        case failed(String)
    }

    /// How often the background loop wakes to see whether a daily check is due.
    static let loopSeconds = 3600.0
    static let checkEverySeconds = 24 * 3600.0

    private(set) var check: UpgradeCheck?
    private(set) var phase: Phase = .idle
    private(set) var lastChecked: Date? = Prefs.d.object(forKey: Prefs.lastUpdateCheck) as? Date

    let appVersion = AppVersion.running
    /// `swift run` builds are not an app bundle and cannot update themselves.
    let isAppBundle = Bundle.main.bundleURL.pathExtension == "app"
    /// Ad-hoc signed builds change signature each release, so macOS may ask for permissions again.
    let isAdHocSigned = UpdateController.adHocSigned(Bundle.main.bundleURL)

    @ObservationIgnored private let updater = AppUpdater()
    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored var cliProvider: () -> CSwapCLI? = { nil }
    @ObservationIgnored var onError: (String) -> Void = { _ in }

    var mode: UpdateMode {
        get { UpdateMode(rawValue: Prefs.d.string(forKey: Prefs.updateMode) ?? "") ?? .notify }
        set { Prefs.d.set(newValue.rawValue, forKey: Prefs.updateMode); modeChanged() }
    }

    /// A release newer than this app that the user has not skipped.
    var available: UpgradeCheck? {
        guard let check, check.isNewer(than: appVersion), check.latest != skippedVersion else { return nil }
        return check
    }

    var skippedVersion: String? { Prefs.d.string(forKey: Prefs.skippedVersion) }

    // MARK: loop

    func start() {
        guard loop == nil else { return }
        // An app staged by an earlier run: offer it again if it is newer, else drop it.
        if let staged = updater.stagedVersion() {
            if AppVersion.compare(staged, appVersion) == .orderedDescending {
                phase = .ready(staged)
            } else {
                try? FileManager.default.removeItem(at: updater.stagedURL)
            }
        }
        loop = Task { [weak self] in
            while !Task.isCancelled {
                await self?.backgroundTick()
                try? await Task.sleep(for: .seconds(Self.loopSeconds))
            }
        }
    }

    func stop() { loop?.cancel(); loop = nil }

    private func backgroundTick() async {
        guard mode.checksInBackground else { return }
        let due = lastChecked.map { Date().timeIntervalSince($0) >= Self.checkEverySeconds } ?? true
        if due { await checkNow(refresh: false) }
        if mode == .automatic, available != nil, phase == .idle { await download() }
    }

    private func modeChanged() {
        if mode == .automatic, available != nil, phase == .idle { Task { await download() } }
    }

    // MARK: actions

    /// Look up the latest release; `refresh` asks GitHub again instead of the day-old cache.
    func checkNow(refresh: Bool = true) async {
        guard let haryx = checkerPath() else {
            phase = .failed("Checking for updates needs the haryx command (this build has none bundled).")
            return
        }
        if case .ready = phase {} else { phase = .checking }
        var args = ["upgrade", "--check", "--json"]
        if refresh { args.append("--refresh") }
        do {
            let stdout = try await Task.detached { try Self.run(haryx, args) }.value
            let decoded = try UpgradeCheck.decode(stdout)
            check = decoded
            lastChecked = Date()
            Prefs.d.set(lastChecked, forKey: Prefs.lastUpdateCheck)
            if case .ready = phase { return }
            phase = decoded.error.map { .failed("Could not check for updates: \($0)") } ?? .idle
        } catch is DecodingError {
            phase = .failed("This haryx (\(haryx.path)) is too old to check for updates. Update it, then check again.")
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }

    /// Download and stage the available release. Falls back to opening the DMG in Finder when the
    /// app cannot replace itself.
    func download() async {
        guard isAppBundle else {
            phase = .failed("Only the HaryX.app build can update itself.")
            return
        }
        guard let release = available, let asset = release.assets.app, let version = release.latest else {
            phase = .failed(AppUpdateError.noDownload.localizedDescription)
            return
        }
        phase = .downloading
        let updater = self.updater
        do {
            let dmg = try await updater.download(asset)
            guard updater.canReplaceInPlace else {
                AppUpdater.openInFinder(dmg)
                phase = .failed(AppUpdateError.cannotReplace(updater.appURL.path).localizedDescription)
                return
            }
            _ = try await Task.detached { try updater.stage(dmg: dmg, expectedVersion: version) }.value
            try? FileManager.default.removeItem(at: dmg)
            phase = .ready(version)
        } catch {
            phase = .failed(error.localizedDescription)
            onError("Update failed: \(error.localizedDescription)")
        }
    }

    /// Set once a swap script runs, so quitting does not start a second one.
    @ObservationIgnored private var swapLaunched = false

    /// Swap the staged app in and reopen it.
    func restartNow() {
        guard !swapLaunched else { return }
        do {
            try updater.launchSwap(relaunch: true)
            swapLaunched = true
            NSApp.terminate(nil)
        } catch {
            phase = .failed("Could not start the update: \(error.localizedDescription)")
        }
    }

    /// Called on quit: a staged update goes in without reopening the app.
    func installOnQuit() {
        guard case .ready = phase, !swapLaunched else { return }
        swapLaunched = (try? updater.launchSwap(relaunch: false)) != nil
    }

    func skip() {
        guard let latest = check?.latest else { return }
        Prefs.d.set(latest, forKey: Prefs.skippedVersion)
    }

    func clearSkip() { Prefs.d.removeObject(forKey: Prefs.skippedVersion) }

    // MARK: helpers

    /// The bundled haryx (same version as the app), else the backend the app uses when it is haryx.
    private func checkerPath() -> URL? {
        if let bin = Bundle.main.resourceURL?.appendingPathComponent("bin/haryx"),
           FileManager.default.isExecutableFile(atPath: bin.path) {
            return bin
        }
        return cliProvider().flatMap { $0.flavor == .haryx ? $0.executable : nil }
    }

    /// Run `haryx` with fixed arguments and return its stdout (the check answers JSON with exit 0).
    nonisolated private static func run(_ exe: URL, _ args: [String]) throws -> Data {
        let process = Process()
        process.executableURL = exe
        process.arguments = args
        process.environment = CSwapCLI.environment
        let out = Pipe()
        process.standardOutput = out
        process.standardError = FileHandle.nullDevice
        process.standardInput = FileHandle.nullDevice
        try process.run()
        let data = out.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return data
    }

    nonisolated static func adHocSigned(_ url: URL) -> Bool {
        var code: SecStaticCode?
        guard SecStaticCodeCreateWithPath(url as CFURL, [], &code) == errSecSuccess, let code else { return false }
        var info: CFDictionary?
        guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &info) == errSecSuccess,
              let dict = info as? [String: Any],
              let flags = dict[kSecCodeInfoFlags as String] as? UInt32 else { return false }
        return flags & SecCodeSignatureFlags.adhoc.rawValue != 0
    }
}
