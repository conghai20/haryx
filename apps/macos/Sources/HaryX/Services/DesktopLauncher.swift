import AppKit
import HaryXCore

/// Quits and reopens Claude Desktop on an account's data folder (plan: `docs/desktop-follow-plan.md`).
/// Desktop is only ever asked to quit normally, never force-killed.
@MainActor
enum DesktopLauncher {
    static let quitTimeout: Duration = .seconds(10)
    static let launchCheckTimeout: Duration = .seconds(10)
    static let pollInterval: Duration = .milliseconds(200)

    enum LaunchError: LocalizedError {
        case notInstalled
        case didNotQuit
        case openFailed(String)
        case wrongFolder
        case folderNotCreated(String)

        var errorDescription: String? {
            switch self {
            case .notInstalled: "Claude Desktop is not installed."
            case .didNotQuit: "Claude Desktop did not quit within 10 seconds, so it was not reopened. Quit it yourself, then try again."
            case .openFailed(let reason): "Claude Desktop could not open on the account's folder (\(reason)). It was opened on its own folder instead."
            case .wrongFolder: "Claude Desktop opened, but not on the account's folder. Quit it and try again."
            case .folderNotCreated(let reason): "Could not create the account's Claude Desktop folder: \(reason)"
            }
        }
    }

    static var running: NSRunningApplication? {
        NSRunningApplication.runningApplications(withBundleIdentifier: ClaudeDesktop.bundleID)
            .first { !$0.isTerminated }
    }

    /// Reads the running Desktop's arguments: a later `open` with another folder is ignored while
    /// it runs, so the folder must be read, never assumed.
    static var state: DesktopRun {
        guard let app = running else { return .notRunning }
        guard let (args, _) = ProcessScanner.procArgs(app.processIdentifier) else { return .runningUnknownFolder }
        return .running(folder: ClaudeDesktop.userDataDir(fromArguments: args))
    }

    /// Quit Desktop if it runs, then open it on `folder` (created owner-only when new) and check
    /// that it really is on that folder.
    static func open(folder: String) async throws(LaunchError) {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: ClaudeDesktop.bundleID) else {
            throw .notInstalled
        }
        if let app = running {
            app.terminate()
            guard await waitUntil(quitTimeout, { app.isTerminated }) else { throw .didNotQuit }
        }
        do {
            try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true,
                                                    attributes: [.posixPermissions: 0o700])
        } catch {
            throw .folderNotCreated(error.localizedDescription)
        }
        do {
            let config = NSWorkspace.OpenConfiguration()
            config.arguments = ClaudeDesktop.launchArguments(folder: folder)
            _ = try await NSWorkspace.shared.openApplication(at: url, configuration: config)
        } catch {
            // A Desktop update could drop `--user-data-dir`: open it plainly and say so.
            _ = try? await NSWorkspace.shared.openApplication(at: url, configuration: NSWorkspace.OpenConfiguration())
            throw .openFailed(error.localizedDescription)
        }
        let onFolder = await waitUntil(launchCheckTimeout) {
            if case .running(let current) = state { ClaudeDesktop.sameFolder(current, folder) } else { false }
        }
        guard onFolder else { throw .wrongFolder }
    }

    private static func waitUntil(_ timeout: Duration, _ condition: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            guard ContinuousClock.now < deadline else { return false }
            try? await Task.sleep(for: pollInterval)
        }
        return true
    }
}
