import AppKit
import HaryXCore

/// Claude Desktop per account (plan: `docs/desktop-follow-plan.md`). Desktop opens on an account's
/// folder only from its Open/Set Up button or the panel's Desktop line, never on a switch; the
/// line shows when Desktop is on another account than the active one.
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

    /// The Desktop line's button: open Desktop as the active account, setting up its folder first
    /// when it has none.
    func openDesktopAsActive() async {
        guard let account = active else { return }
        await openDesktop(as: account)
    }

    /// What an account's card and menu offer for Desktop: Set Up until its folder has signed in,
    /// then Open; nothing when the setting is off or the folder is not this account's to use.
    func desktopAction(for account: Account) -> DesktopLine.Action? {
        guard desktopFollows, let desktop else { return nil }
        return switch ClaudeDesktop.target(for: account.number, in: desktop) {
        case .ready: .open
        case .setUp: .setUp
        case .mismatch, .unavailable: nil
        }
    }

    func offersDesktopSetUp(_ account: Account) -> Bool { desktopAction(for: account) == .setUp }

    /// Open Desktop on `account`'s folder, setting it up first when it has not signed in. Claude
    /// Code stays on whichever account it is on.
    func openDesktop(as account: Account) async {
        await reloadDesktop()
        guard let desktop else { return }
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
            // Opening as the active account must not outlive a switch made while the alert was up;
            // opening a non-active account was never tied to the active one.
            let wasActive = active?.identityKey == account.identityKey
            guard confirmDesktopRelaunch(account) else { return }
            guard !wasActive || active?.identityKey == account.identityKey else {
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
