import AppKit
import HaryXCore

/// Claude Desktop follows the Use button (plan: `docs/desktop-follow-plan.md`). Only a manual Use
/// moves Desktop; auto-switch and AI Wake never call in here, and the panel's Desktop line shows
/// when Desktop has been left on another account.
extension AppStore {
    /// The setting is on and the backend has `haryx ai desktop`.
    var desktopFollows: Bool { Prefs.d.bool(forKey: Prefs.desktopFollows) && cli?.flavor == .haryx }

    /// The panel's Desktop line, when the setting is on.
    var desktopLine: DesktopLine? {
        guard desktopFollows, let desktop, let active else { return nil }
        return DesktopLine.make(run: desktopRun, status: desktop, active: active.number) { number in
            accounts.first { $0.number == number }?.displayName ?? desktop.folder(number)?.email ?? "account \(number)"
        }
    }

    /// Re-read the folders (local files only) and the running Desktop's folder.
    func reloadDesktop() async {
        guard desktopFollows, let cli else {
            setDesktop(nil, run: .notRunning)
            return
        }
        let status = try? await cli.json(DesktopStatus.self, ["ai", "desktop", "status", "--json"])
        // Keep the previous folders when a read fails rather than blanking the line.
        setDesktop(status ?? desktop, run: DesktopLauncher.state)
    }

    /// After a manual Use: move a running Desktop to `account`. A Desktop that is not running is
    /// left alone; the panel's line offers to open it.
    func followDesktop(to account: Account) async {
        guard desktopFollows else { return }
        await reloadDesktop()
        guard case .running(let current) = desktopRun, let desktop else { return }
        switch ClaudeDesktop.target(for: account.number, in: desktop) {
        case .ready(let folder):
            guard !ClaudeDesktop.sameFolder(current, folder) else { return }
            await launchDesktop(folder, as: account, setUp: false)
        case .setUp:
            flash("Claude Desktop stays as it is: \(account.displayName) has no Desktop folder yet. Use Set Up Desktop in the menu bar.")
        case .mismatch:
            flash(Self.mismatchMessage(account), error: true)
        case .unavailable:
            return
        }
    }

    /// The Desktop line's button: open Desktop as the active account, setting up its folder first
    /// when it has none.
    func openDesktopAsActive() async {
        await reloadDesktop()
        guard let account = active, let desktop else { return }
        switch ClaudeDesktop.target(for: account.number, in: desktop) {
        case .ready(let folder): await launchDesktop(folder, as: account, setUp: false)
        case .setUp(let folder): await launchDesktop(folder, as: account, setUp: true)
        case .mismatch: flash(Self.mismatchMessage(account), error: true)
        case .unavailable: flash("\(account.displayName) has no account UUID, so Claude Desktop cannot open as it.", error: true)
        }
    }

    private static func mismatchMessage(_ account: Account) -> String {
        "Claude Desktop stays as it is: \(account.displayName)'s Desktop folder is signed into another account. HaryX leaves that folder alone."
    }

    private func launchDesktop(_ folder: String, as account: Account, setUp: Bool) async {
        // Busy before the alert: it is modal, but other work (a refresh, auto-switch) still runs.
        busy = "Opening Claude Desktop"
        defer { busy = nil }
        if desktopRun != .notRunning {
            guard confirmDesktopRelaunch(account) else { return }
            guard active?.identityKey == account.identityKey else {
                return flash("Claude Desktop stays as it is: the active account changed while HaryX was asking.")
            }
        }
        do {
            try await DesktopLauncher.open(folder: folder)
            flash(setUp
                ? "Sign in to Claude Desktop as \(account.email). This folder is for that account only."
                : "Claude Desktop is now \(account.displayName)")
        } catch {
            flash(error.localizedDescription, error: true)
        }
        await reloadDesktop()
    }

    /// "Claude Desktop will quit and reopen as …", unless the user ticked "Don't ask again".
    private func confirmDesktopRelaunch(_ account: Account) -> Bool {
        if Prefs.d.bool(forKey: Prefs.desktopSkipConfirm) { return true }
        let alert = NSAlert()
        alert.messageText = "Claude Desktop will quit and reopen as \(account.email)."
        alert.informativeText = "It quits normally, as with ⌘Q. If it does not quit within 10 seconds, it is left running."
        alert.addButton(withTitle: "Quit and Reopen")
        alert.addButton(withTitle: "Cancel")
        alert.showsSuppressionButton = true
        alert.suppressionButton?.title = "Don't ask again"
        NSApp.activate()
        let confirmed = alert.runModal() == .alertFirstButtonReturn
        if confirmed, alert.suppressionButton?.state == .on {
            Prefs.d.set(true, forKey: Prefs.desktopSkipConfirm)
        }
        return confirmed
    }
}
