import AppKit
import HaryXCore
import Foundation

/// Every cswap command the app can issue. Each goes through `perform`, which reports
/// success/failure in the banner and refreshes state afterwards.
extension AppStore {
    enum SwitchMode: String, CaseIterable, Identifiable {
        case next, best, nextAvailable = "next-available"
        var id: String { rawValue }
        var title: String {
            switch self {
            case .next: "Next account"
            case .best: "Most headroom"
            case .nextAvailable: "Next available"
            }
        }
        var args: [String] { self == .next ? ["switch", "--json"] : ["switch", "--strategy", rawValue, "--json"] }
    }

    @discardableResult
    func perform(_ label: String, _ args: [String], stdin: String? = nil, success: String? = nil,
                 extras: Bool = false) async -> Bool {
        guard let cli else { flash(CLIError.notFound.localizedDescription, error: true); return false }
        busy = label
        defer { busy = nil }
        do {
            let output = try await cli.checked(args, stdin: stdin)
            flash(success ?? Self.lastLine(output) ?? "\(label): done")
            await refresh()
            if extras { await loadExtras() }
            return true
        } catch {
            flash(error.localizedDescription, error: true)
            return false
        }
    }

    static func lastLine(_ s: String) -> String? {
        s.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.last(where: { !$0.isEmpty })
    }

    /// How to name an account to cswap. Slot numbers can change underneath a stale snapshot
    /// (a `move`/`swap` from a terminal), so use the email whenever it is unique.
    func ref(_ a: Account) -> String {
        accounts.filter { $0.email == a.email }.count == 1 ? a.email : String(a.number)
    }

    // MARK: switching

    /// A manual switch. The only path that moves Claude Desktop (when that setting is on).
    func switchTo(_ a: Account, force: Bool = false) async {
        guard await runSwitch("Switching", ["switch", ref(a), "--json"] + (force ? ["--force"] : [])) else { return }
        await followDesktop(to: active ?? a)
    }

    func switchNext(_ mode: SwitchMode) async {
        await runSwitch(mode.title, mode.args)
    }

    @discardableResult
    private func runSwitch(_ label: String, _ args: [String]) async -> Bool {
        guard let cli else { flash(CLIError.notFound.localizedDescription, error: true); return false }
        busy = label
        defer { busy = nil }
        do {
            let result = try await cli.json(SwitchResult.self, args)
            await refresh()
            if result.switched {
                flash("Now using \(active?.displayName ?? result.to?.email ?? "account")")
            } else {
                flash("Already on the best account — no switch needed")
            }
            return true
        } catch {
            flash(error.localizedDescription, error: true)
            return false
        }
    }

    // MARK: accounts

    func addCurrent(slot: Int?, alias: String?, overwrite: Bool) async {
        var args = ["add"]
        if let slot { args += ["--slot", String(slot)] }
        if let alias, !alias.isEmpty { args += ["--alias", alias] }
        await perform("Adding", args, stdin: overwrite ? "y\n" : "n\n", extras: true)
    }

    /// Sign an account in again: `haryx ai login` runs in the background and its sign-in window
    /// shows the link and takes the pasted code.
    func signInAgain(_ a: Account) {
        guard a.canRelogin else {
            return flash("\(a.displayName) is a token, not a login. Replace it with Sign In New Account.", error: true)
        }
        if loginPending(for: a) { return flash("A sign-in for \(a.displayName) is already open.") }
        startLogin(["ai", "login", ref(a)], title: "Sign In Again: \(a.email)", identityKey: a.identityKey)
    }

    /// Add a new account through `haryx ai login` without changing the active login.
    func signInNew(slot: Int? = nil, alias: String? = nil) {
        var args = ["ai", "login"]
        if let slot { args += ["--slot", String(slot)] }
        if let alias, !alias.isEmpty { args += ["--alias", alias] }
        startLogin(args, title: "Sign In New Account", identityKey: nil)
    }

    private func startLogin(_ args: [String], title: String, identityKey: String?) {
        guard let cli else { return flash(CLIError.notFound.localizedDescription, error: true) }
        guard cli.flavor == .haryx else {
            return flash("Signing in from the app needs the haryx backend.", error: true)
        }
        let session = LoginSession(title: title, identityKey: identityKey)
        do {
            try session.start(executable: cli.executable, arguments: args)
        } catch {
            return flash(error.localizedDescription, error: true)
        }
        trackLogin(session)
        LoginWindow.show(session, store: self)
    }

    /// The token travels on stdin (`add-token -`), never in argv where `ps` could see it.
    func addToken(_ token: String, slot: Int?, email: String?, overwrite: Bool) async {
        var args = ["add-token", "-"]
        if let slot { args += ["--slot", String(slot)] }
        if let email, !email.isEmpty { args += ["--email", email] }
        // Tokens contain no whitespace; stripping it all keeps an inner newline from becoming
        // the answer to the overwrite prompt.
        let trimmed = token.filter { !$0.isWhitespace }
        // add-token reads the token line first; a following y/n answers any overwrite prompt.
        await perform("Adding token", args, stdin: trimmed + "\n" + (overwrite ? "y\n" : "n\n"), extras: true)
    }

    func remove(_ a: Account) async {
        await perform("Removing", ["remove", ref(a)], stdin: "y\n", success: "Removed \(a.displayName)", extras: true)
    }

    func setDisabled(_ a: Account, _ disabled: Bool) async {
        await perform(disabled ? "Disabling" : "Enabling", [disabled ? "disable" : "enable", ref(a)])
    }

    // MARK: wake (`haryx ai wake`)

    func setWake(_ a: Account, _ on: Bool) async {
        await perform(on ? "Wake on" : "Wake off", ["ai", "wake", on ? "on" : "off", ref(a)], extras: true)
    }

    func setWakeAll(_ on: Bool) async {
        await perform(on ? "Wake on for all" : "Wake off for all", ["ai", "wake", on ? "on" : "off", "all"], extras: true)
    }

    func setWakeAgent(installed: Bool) async {
        await perform(installed ? "Installing wake agent" : "Removing wake agent",
                      ["ai", "wake", installed ? "install" : "uninstall"], extras: true)
    }

    func setWakeConfig(quietHours: String?, weeklyCapPct: Double) async {
        let quiet = quietHours.map { ["--quiet", $0] } ?? ["--no-quiet"]
        await perform("Wake settings", ["ai", "wake", "config"] + quiet + ["--weekly-cap", String(Int(weeklyCapPct))],
                      success: "Wake settings saved", extras: true)
    }

    func setAlias(_ a: Account, _ alias: String?) async {
        let args = alias.map { ["alias", ref(a), $0] } ?? ["alias", ref(a), "--unset"]
        await perform("Alias", args)
    }

    func setGroup(_ a: Account, _ group: String?) async {
        let args = group.map { ["group", ref(a), $0] } ?? ["group", ref(a), "--unset"]
        await perform("Group", args)
    }

    func setThreshold(_ a: Account, _ pct: Double?) async {
        let args = pct.map { ["threshold", ref(a), String(format: "%.1f", $0)] } ?? ["threshold", ref(a), "--unset"]
        await perform("Threshold", args)
    }

    func setPriority(_ a: Account, _ tier: Int?) async {
        let args = tier.map { ["priority", ref(a), String($0)] } ?? ["priority", ref(a), "--unset"]
        await perform("Priority", args, extras: true)
    }

    func move(_ a: Account, to slot: Int) async {
        await perform("Moving", ["move", ref(a), String(slot)], extras: true)
    }

    func swap(_ a: Account, _ b: Account) async {
        await perform("Swapping", ["swap", ref(a), ref(b)], extras: true)
    }

    // MARK: mappings

    func map(_ a: Account, path: String) async {
        await perform("Mapping", ["map", ref(a), path], extras: true)
    }

    func unmap(_ path: String) async {
        await perform("Unmapping", ["unmap", path], extras: true)
    }

    // MARK: config

    func configSet(_ key: String, _ value: String) async {
        await perform("Saving", ["config", "set", key, value], extras: true)
    }

    func configUnset(_ key: String) async {
        await perform("Resetting", ["config", "unset", key], extras: true)
    }

    func configValue(_ key: String) -> JSONScalar? { config.first { $0.key == key }?.value }

    // MARK: maintenance

    /// Exports `accounts`; empty means every account.
    func export(to url: URL, accounts: [Account], full: Bool) async {
        if accounts.count > 1, cli?.flavor == .cswap {
            return flash("cswap exports one account or all of them; pick one, or every account.", error: true)
        }
        var args = ["export", url.path]
        if !accounts.isEmpty { args += ["--account", accounts.map(ref).joined(separator: ",")] }
        if full { args.append("--full") }
        if await perform("Exporting", args, success: "Exported to \(url.lastPathComponent)") {
            // The export holds live tokens: keep it owner-only.
            try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        }
    }

    func importAccounts(from url: URL, force: Bool) async {
        await perform("Importing", ["import", url.path] + (force ? ["--force"] : []), extras: true)
    }

    func importUsage(from url: URL, hold: Int?) async {
        await perform("Importing usage", ["import-usage", url.path] + (hold.map { ["--hold", String($0)] } ?? []))
    }

    func purgeUnclaimed(_ id: String) async {
        await perform("Purging", ["unclaimed", "--purge", id], extras: true)
    }

    /// haryx keeps its own store; on first run, copy cswap's accounts in (cswap is left untouched).
    var canImportFromCswap: Bool {
        cli?.flavor == .haryx && accounts.isEmpty
            && FileManager.default.fileExists(atPath: CLIFlavor.cswap.storeRoot + "/sequence.json")
    }

    func importFromCswap() async {
        await perform("Importing from cswap", ["import", "--from-cswap"], success: "Accounts imported from cswap", extras: true)
    }

    func upgrade() async {
        await perform("Upgrading \(CSwapCLI.selectedFlavor.rawValue)", ["upgrade"], extras: true)
    }

    func purgeAll() async {
        Prefs.d.set(false, forKey: Prefs.autoWasOn)
        await auto.stopAndWait()
        await perform("Purging all data", ["purge"], stdin: "y\n", success: "All account data removed", extras: true)
    }

    func runAutoOnce() async {
        guard let cli else { return }
        busy = "Checking"
        defer { busy = nil }
        do {
            let r = try await cli.run(["auto", "--once", "--json"] + (Prefs.d.bool(forKey: Prefs.autoDryRun) ? ["--dry-run"] : []))
            let events = r.text.split(separator: "\n").compactMap { AutoEvent.parse(line: String($0)) }
            let outcome = switch r.status {
            case 0: "Switched"
            case 2: "No action needed"
            case 3: "Blocked: no viable account"
            default: "Error: \(r.stderr.split(separator: "\n").last ?? "")"
            }
            flash(([outcome] + events.filter { $0.kind != "poll" }.map(\.summary)).joined(separator: " · "), error: r.status == 1)
            await refresh()
        } catch {
            flash(error.localizedDescription, error: true)
        }
    }

    // MARK: terminals

    func openTerminal(_ app: TerminalApp = .preferred, command: String? = nil, folder: String? = nil) {
        let dir = folder ?? Prefs.d.string(forKey: Prefs.claudeFolder) ?? CSwapCLI.home
        do {
            try TerminalLauncher.open(app, command: command, in: dir)
        } catch {
            flash(error.localizedDescription, error: true)
        }
    }

    /// `cswap run N` = Claude as that account in this terminal only (session mode).
    func runClaude(as a: Account?, app: TerminalApp = .preferred, folder: String? = nil) {
        guard let cli else { return flash(CLIError.notFound.localizedDescription, error: true) }
        let command = a.map { "\(shellQuote(cli.executable.path)) run \(shellQuote(ref($0)))" } ?? "claude"
        openTerminal(app, command: command, folder: folder)
    }

    /// Opens a terminal running Anthropic's Claude Code installer (the command the CLI reports).
    func installClaude() {
        guard let command = claudeSetup?.installCommand else { return }
        openTerminal(command: command)
    }

    /// Opens a terminal running `claude`, which asks to sign in the first time.
    func signInToClaude() { openTerminal(command: "claude") }

    /// Mapped folder: plain `cswap run` picks the account from the mapping.
    func runMapped(_ mapping: DirectoryMapping, app: TerminalApp = .preferred) {
        guard let cli else { return }
        openTerminal(app, command: "\(shellQuote(cli.executable.path)) run", folder: mapping.path)
    }

    static func chooseFolder(prompt: String = "Choose") -> String? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = prompt
        NSApp.activate()
        return panel.runModal() == .OK ? panel.url?.path : nil
    }
}
