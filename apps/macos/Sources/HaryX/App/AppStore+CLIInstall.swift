import AppKit
import HaryXCore

/// Installing the bundled `haryx` command (DMG builds): offered once on first launch, and from
/// Preferences any time.
extension AppStore {
    /// Ask once, on the first launch of a build that carries the CLI and finds none installed.
    func offerCLIInstallIfNeeded() {
        let installer = CLIInstaller.current
        guard installer.shouldOffer, !Prefs.d.bool(forKey: Prefs.cliInstallOffered) else { return }
        Prefs.d.set(true, forKey: Prefs.cliInstallOffered)
        let alert = NSAlert()
        alert.messageText = "Install the haryx command?"
        alert.informativeText = """
            HaryX includes the haryx command-line tool (accounts, usage, setup profiles). Install links \
            it into /usr/local/bin so any terminal can run it; macOS asks for your password. "For Me \
            Only" uses ~/.local/bin instead, no password. Updating HaryX updates the command too. You \
            can change this later in Preferences.
            """
        alert.addButton(withTitle: "Install")
        alert.addButton(withTitle: "For Me Only")
        alert.addButton(withTitle: "Not Now")
        NSApp.activate()
        switch alert.runModal() {
        case .alertFirstButtonReturn: installCLI(systemWide: true)
        case .alertSecondButtonReturn: installCLI(systemWide: false)
        default: break
        }
    }

    /// Link the bundled CLI at `/usr/local/bin` (admin prompt) or `~/.local/bin`.
    @discardableResult
    func installCLI(systemWide: Bool) -> Bool {
        let installer = CLIInstaller.current
        do {
            if systemWide {
                try runAsAdmin(installer.adminInstallCommand(at: CLIInstaller.systemLink))
            } else {
                try installer.installLink(at: CLIInstaller.userLink)
            }
        } catch AdminError.cancelled {
            return false
        } catch {
            flash("Could not install haryx: \(error.localizedDescription)", error: true)
            return false
        }
        relocateCLI()
        let dir = systemWide ? "/usr/local/bin" : "~/.local/bin"
        let hint = systemWide || CLIInstaller.onPath(CSwapCLI.home + "/.local/bin", path: CSwapCLI.loginShellPath)
            ? "" : " Add ~/.local/bin to your PATH to use it."
        flash("Installed haryx in \(dir).\(hint)")
        return true
    }

    func removeCLI(at path: String) {
        let installer = CLIInstaller.current
        do {
            if path == CLIInstaller.systemLink {
                try runAsAdmin(installer.adminRemoveCommand(at: path))
            } else {
                try installer.removeLink(at: path)
            }
            relocateCLI()
            flash("Removed the haryx command from \((path as NSString).deletingLastPathComponent)")
        } catch AdminError.cancelled {
        } catch {
            flash("Could not remove haryx: \(error.localizedDescription)", error: true)
        }
    }

    enum AdminError: LocalizedError {
        case cancelled
        case failed(String)
        var errorDescription: String? {
            switch self {
            case .cancelled: "Cancelled"
            case .failed(let message): message
            }
        }
    }

    /// Run a fixed shell command (paths quoted by CLIInstaller) with the macOS admin prompt.
    private func runAsAdmin(_ command: String) throws {
        let escaped = command.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        let script = NSAppleScript(source: "do shell script \"\(escaped)\" with administrator privileges")
        var error: NSDictionary?
        script?.executeAndReturnError(&error)
        guard let error else { return }
        if (error[NSAppleScript.errorNumber] as? Int) == -128 { throw AdminError.cancelled }
        throw AdminError.failed(error[NSAppleScript.errorMessage] as? String ?? "Unknown error")
    }
}
