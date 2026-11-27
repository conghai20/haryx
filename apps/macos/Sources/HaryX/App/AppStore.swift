import AppKit
import HaryXCore
import Foundation
import Observation

struct Banner: Equatable {
    let text: String
    let isError: Bool
}

/// Single source of truth. All cswap I/O runs off the main thread; state updates land here.
@MainActor @Observable
final class AppStore {
    static let sessionScanSeconds = 4.0
    /// Session scan cadence while neither the panel nor the dashboard is on screen.
    static let hiddenSessionScanSeconds = 15.0
    /// Each status-item image change costs a menu bar re-snapshot in AppKit, so keep this low.
    static let menuBarFPS = 1.0
    static let bannerSeconds = 5.0

    // cswap state
    private(set) var accounts: [Account] = []
    /// Will the week last: the CLI's verdict over the accounts auto-switch may use.
    private(set) var rotation: Rotation?
    /// Each account's weekly readings over the last 7 days, for its Week Tide (kept only while
    /// usage history is on; empty otherwise, and the tide draws its average instead).
    private(set) var weekReadings: [Int: [(at: Date, pct: Double)]] = [:]
    private(set) var lastRefresh: Date?
    private(set) var refreshError: String?
    /// True while an accounts fetch runs; drives the spinning refresh icons.
    private(set) var isRefreshing = false
    private(set) var config: [ConfigEntry] = []
    private(set) var configPath = ""
    private(set) var mappings: [DirectoryMapping] = []
    private(set) var priorities: [Int: Int] = [:]
    private(set) var unclaimed: [UnclaimedEntry] = []
    private(set) var version = ""
    /// `haryx ai wake status`; nil with a cswap backend or before the first load.
    private(set) var wake: WakeStatus?
    /// Why wake status could not be read (an older haryx without `ai wake`), when it could not.
    private(set) var wakeError: String?
    /// `haryx ai desktop status`; nil while "Claude Desktop per account" is off.
    private(set) var desktop: DesktopStatus?
    /// `haryx ai doctor`: nil until read, or with a cswap backend.
    private(set) var claudeSetup: ClaudeSetup?
    /// Whether Claude Desktop runs, and on which folder; read when the panel opens.
    private(set) var desktopRun: DesktopRun = .notRunning
    private(set) var cli: CSwapCLI?

    // UI state
    var busy: String?
    var banner: Banner?
    /// Background `haryx ai login` runs still going (each has a sign-in window).
    private(set) var loginSessions: [LoginSession] = []
    /// slot → live Claude processes using it
    private(set) var sessions: [Int: [ClaudeProcess]] = [:]
    /// Live Claude processes on no known account (no active account, or a foreign CLAUDE_CONFIG_DIR).
    private(set) var unassignedSessions: [ClaudeProcess] = []
    var panelVisible = false
    /// The dashboard window is on screen (not closed, minimized or fully covered).
    var dashboardVisible = false
    /// The menu bar shows the active account's mascot instead of the Wing H mark (opt-in).
    private(set) var menuBarMascot = Prefs.d.bool(forKey: Prefs.menuBarMascot)
    private(set) var menuBarTick = 0

    let auto = AutoSwitchRunner()
    let updates = UpdateController()
    let usage = UsageModel()
    let styles = MascotStyles()
    let notifier = Notifier()
    let accountSync = AccountSync()

    @ObservationIgnored private var loops: [Task<Void, Never>] = []
    @ObservationIgnored private var menuBarLoop: Task<Void, Never>?
    @ObservationIgnored private var bannerTask: Task<Void, Never>?
    @ObservationIgnored private var watcher: DispatchSourceFileSystemObject?
    @ObservationIgnored private var seenSequenceMTime: Date?
    @ObservationIgnored private var knownProcesses: [Int32: (ClaudeProcess, Int?)] = [:]
    @ObservationIgnored private var refreshTask: Task<Void, Never>?

    init() {
        Prefs.register()
        relocateCLI()
        auto.onEvent = { [weak self] event in self?.handleAutoEvent(event) }
        TerminalLauncher.onError = { [weak self] message in self?.flash(message, error: true) }
        updates.cliProvider = { [weak self] in self?.cli }
        usage.cliProvider = { [weak self] in self?.cli }
        updates.onError = { [weak self] message in self?.flash(message, error: true) }
        wireAccountSync()
    }

    // MARK: lifecycle

    func start() {
        guard loops.isEmpty else { return }
        notifier.requestAuthorization()
        loops.append(Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                let seconds = max(15, Prefs.d.double(forKey: Prefs.refreshSeconds))
                try? await Task.sleep(for: .seconds(seconds))
            }
        })
        loops.append(Task { [weak self] in
            while !Task.isCancelled {
                await self?.scanSessions()
                let onScreen = self.map { $0.panelVisible || $0.dashboardVisible } ?? false
                try? await Task.sleep(for: .seconds(onScreen ? Self.sessionScanSeconds : Self.hiddenSessionScanSeconds))
            }
        })
        watchBackupDir()
        updates.start()
        accountSync.start()
        usage.startWeeklySummary(notify: { [weak self] title, body in
            self?.notifier.post(Notice(key: "weekly:\(Date().timeIntervalSince1970)", category: .usage, title: title, body: body))
        }, name: { [weak self] n in self?.account(slot: n)?.displayName ?? "Account \(n)" })
        Task { await loadExtras() }
        if Prefs.d.bool(forKey: Prefs.autoOnLaunch) || Prefs.d.bool(forKey: Prefs.autoWasOn) { startAuto() }
        DispatchQueue.main.async { [weak self] in self?.offerCLIInstallIfNeeded() }
    }

    func shutdown() {
        auto.stop()
        updates.stop()
        accountSync.stop()
        updates.installOnQuit()
        loops.forEach { $0.cancel() }
        menuBarLoop?.cancel()
    }

    func relocateCLI() {
        // `--cli PATH` (design review) runs another backend without touching the saved preference.
        let args = CommandLine.arguments
        let reviewCLI = args.firstIndex(of: "--cli").flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil }
        cli = CSwapCLI.locate(override: reviewCLI ?? Prefs.d.string(forKey: Prefs.cswapPath)).map(CSwapCLI.init(executable:))
        refreshError = cli == nil ? CLIError.notFound.localizedDescription : nil
    }

    // MARK: derived

    var active: Account? { accounts.first(where: \.active) }

    /// The other accounts in switch order: usable before signed-out before disabled, then the
    /// accounts sharing a group with the active one first (as the CLI's automatic picks prefer them) and the other groups
    /// by name with the ungrouped last, then the most headroom first (the `best` strategy's reading).
    var nextInLine: [Account] {
        func tier(_ a: Account) -> Int { a.isDisabled ? 2 : (a.needsRelogin ? 1 : 0) }
        let active = self.active
        func group(_ a: Account) -> (Int, String) {
            if let active, a.sharesGroup(with: active) { return (0, "") }
            return a.groupNames.isEmpty ? (2, "") : (1, a.groupKeys.min() ?? "")
        }
        return accounts.filter { !$0.active }.sorted {
            if tier($0) != tier($1) { return tier($0) < tier($1) }
            if group($0) != group($1) { return group($0) < group($1) }
            let l = $0.peakPct ?? 101, r = $1.peakPct ?? 101
            return l != r ? l < r : $0.number < $1.number
        }
    }
    var styleMap: [String: MascotStyle] { styles.resolve(accounts) }

    func style(for a: Account) -> MascotStyle {
        styleMap[a.identityKey] ?? MascotStyle(species: Species.defaultFor(key: a.identityKey))
    }

    func isRunning(_ a: Account) -> Bool { !(sessions[a.number] ?? []).isEmpty }

    func mood(for a: Account) -> Mood { Mood.of(a, running: isRunning(a)) }

    func phase(for a: Account) -> Int { Int(stableHash(a.identityKey) % 13) }

    func account(slot: Int) -> Account? { accounts.first { $0.number == slot } }

    func account(for mapping: DirectoryMapping) -> Account? {
        accounts.first { $0.identityKey == mapping.identityKey }
    }

    /// The Wing H mark as a template image; a dot joins it while the active account runs a session.
    /// With the mascot preference on, the active account's mascot walks while a session runs instead.
    var menuBarImage: NSImage {
        let running = active.map(isRunning) ?? false
        guard menuBarMascot, let a = active else { return running ? Self.markRunning : Self.mark }
        let mood = mood(for: a)
        let pose = running && menuBarLoop != nil
            ? Pose(altLegs: menuBarTick % 2 == 1, squat: menuBarTick % 2 == 0, mood: .running)
            : Pose(mood: mood == .running ? .idle : mood)
        return MascotCache.menuBarImage(style(for: a), pose: pose)
    }
    private static let mark = WingH.templateImage()
    private static let markRunning = WingH.templateImage(running: true)

    // MARK: sign-in

    func loginPending(for a: Account) -> Bool {
        loginSessions.contains { $0.identityKey == a.identityKey }
    }
    var addLoginPending: Bool { loginSessions.contains { $0.identityKey == nil } }

    func trackLogin(_ session: LoginSession) {
        loginSessions.append(session)
        session.onFinish = { [weak self] finished in
            guard let self else { return }
            self.loginSessions.removeAll { $0.id == finished.id }
            switch finished.state {
            case .succeeded(let message):
                self.flash(message)
                // A new login here takes the synced alias, priority and so on.
                Task { await self.accountSync.syncNow() }
            case .failed(let message): self.flash(message, error: true)
            case .cancelled, .running: break
            }
            Task { await self.refresh(); await self.loadExtras() }
        }
    }

    // MARK: refresh

    /// Serialised, never dropped: a caller arriving mid-refresh waits, then runs its own, so
    /// post-action refreshes always observe the action's result.
    func refresh() async {
        while let running = refreshTask { await running.value }
        // The task clears itself *before* completing, so a woken waiter never sees a finished
        // task still installed (which would make it spin on an already-completed `value`).
        let task = Task {
            self.isRefreshing = true
            await self.fetchAccounts()
            self.isRefreshing = false
            self.refreshTask = nil
        }
        refreshTask = task
        await task.value
    }

    private func fetchAccounts() async {
        guard let cli else { return }
        if watcher == nil { watchBackupDir() } // first run, or the dir was purged and recreated
        do {
            let list = try await cli.json(AccountList.self, ["list", "--json"])
            let previous = accounts
            accounts = list.accounts
            rotation = list.rotation
            if let readings = await Self.loadWeekReadings(cli) { weekReadings = readings }
            lastRefresh = Date()
            refreshError = nil
            seenSequenceMTime = Self.sequenceMTime()
            notifier.handle(NotificationRules.evaluate(previous: previous, current: list.accounts))
        } catch {
            refreshError = error.localizedDescription
        }
    }

    /// `haryx ai usage quota --since 7d`: a file read, cheap enough for every refresh. nil on a
    /// cswap backend or a failed read, which keeps the last readings.
    private static func loadWeekReadings(_ cli: CSwapCLI) async -> [Int: [(at: Date, pct: Double)]]? {
        guard cli.flavor == .haryx,
              let quota = try? await UsageModel.json(UsageHistory.Quota.self, cli,
                                                     ["ai", "usage", "quota", "--since", "7d"])
        else { return nil }
        return Dictionary(grouping: quota.readings.compactMap { r in r.sevenDay.map { (r.account, (at: r.date, pct: $0)) } },
                          by: \.0).mapValues { $0.map(\.1) }
    }

    /// Settings, mappings, priorities, unclaimed, version: cheap, refreshed on demand.
    /// `haryx ai wake status`; nil with a cswap backend.
    private static func loadWake(_ cli: CSwapCLI) async -> Result<WakeStatus, Error>? {
        guard CSwapCLI.selectedFlavor == .haryx else { return nil }
        do { return .success(try await cli.json(WakeStatus.self, ["ai", "wake", "status", "--json"])) }
        catch { return .failure(error) }
    }

    func loadExtras() async {
        guard let cli else { return }
        async let configList = try? cli.json(ConfigList.self, ["config", "list", "--json"])
        async let prio = try? cli.checked(["priority"])
        async let uncl = try? cli.checked(["unclaimed"])
        async let ver = try? cli.checked(["--version"])
        async let wakeStatus = Self.loadWake(cli)
        // Keep the previous values when a call fails rather than blanking the UI.
        if let c = await configList { config = c.settings; configPath = c.path }
        if let p = await prio { priorities = TextParsers.priorities(p) }
        if let u = await uncl { unclaimed = TextParsers.unclaimed(u) }
        if let v = await ver { version = v.trimmingCharacters(in: .whitespacesAndNewlines) }
        apply(await wakeStatus)
        loadMappings()
        await reloadDesktop()
        await checkClaudeSetup()
    }

    /// Re-read whether Claude Code is installed and signed in. A failed read keeps the last answer.
    func checkClaudeSetup() async {
        guard let cli, cli.flavor == .haryx else { claudeSetup = nil; return }
        if let setup = try? await cli.json(ClaudeSetup.self, ["ai", "doctor", "--json"]) { claudeSetup = setup }
    }

    /// Re-read `haryx ai wake status` alone (local files only), so the panel shows what the
    /// background agent did since the last load.
    func reloadWake() async {
        guard let cli else { return }
        apply(await Self.loadWake(cli))
    }

    private func apply(_ wakeStatus: Result<WakeStatus, Error>?) {
        switch wakeStatus {
        case .success(let w)?: wake = w; wakeError = nil
        case .failure(let e)?: wake = nil; wakeError = e.localizedDescription
        case nil: wake = nil; wakeError = nil
        }
    }

    func setDesktop(_ status: DesktopStatus?, run: DesktopRun) {
        desktop = status
        desktopRun = run
    }

    func loadMappings() {
        let url = URL(fileURLWithPath: CSwapCLI.backupRoot).appendingPathComponent("mappings.json")
        mappings = (try? Data(contentsOf: url)).map(CSwapJSON.mappings(from:)) ?? []
    }

    // MARK: file watching

    static func sequenceMTime() -> Date? {
        let path = CSwapCLI.backupRoot + "/sequence.json"
        return (try? FileManager.default.attributesOfItem(atPath: path))?[.modificationDate] as? Date
    }

    /// cswap writes sequence.json atomically (rename), so watch the directory and compare mtimes;
    /// our own `list` refreshes record the mtime, which prevents feedback loops.
    private func watchBackupDir() {
        let fd = open(CSwapCLI.backupRoot, O_EVTONLY)
        guard fd >= 0 else { return }
        let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: [.write, .delete, .rename], queue: .main)
        source.setEventHandler { [weak self, weak source] in
            MainActor.assumeIsolated {
                guard let self else { return }
                if let events = source?.data, !events.isDisjoint(with: [.delete, .rename]) {
                    // Dir removed (purge): drop the stale fd; the next refresh re-installs the watch.
                    source?.cancel()
                    self.watcher = nil
                    return
                }
                guard Self.sequenceMTime() != self.seenSequenceMTime else { return }
                self.seenSequenceMTime = Self.sequenceMTime()
                Task {
                    try? await Task.sleep(for: .milliseconds(400))
                    await self.refresh()
                    self.loadMappings()
                }
            }
        }
        source.setCancelHandler { close(fd) }
        source.resume()
        watcher = source
    }

    // MARK: sessions

    private func scanSessions() async {
        guard let procs = await Task.detached(priority: .utility, operation: { ProcessScanner.scan() }).value else { return }
        let activeSlot = active?.number
        var bySlot: [Int: [ClaudeProcess]] = [:]
        var current: [Int32: (ClaudeProcess, Int?)] = [:]
        for p in procs {
            let slot = ProcessScanner.slot(forConfigDir: p.configDir, backupRoot: CSwapCLI.backupRoot,
                                           home: CSwapCLI.home, activeSlot: activeSlot)
            current[p.pid] = (p, slot)
            if let slot { bySlot[slot, default: []].append(p) }
        }
        for (pid, (proc, slot)) in knownProcesses where current[pid] == nil {
            sessionEnded(proc, slot: slot)
        }
        knownProcesses = current
        if bySlot != sessions { sessions = bySlot }
        let unassigned = current.values.filter { $0.1 == nil }.map(\.0)
        if Set(unassigned) != Set(unassignedSessions) { unassignedSessions = unassigned }
        updateMenuBarAnimation()
    }

    func setMenuBarMascot(_ on: Bool) {
        Prefs.d.set(on, forKey: Prefs.menuBarMascot)
        menuBarMascot = on
        updateMenuBarAnimation()
    }

    /// The walk timer runs only while the mascot preference is on and the active account runs a session.
    private func updateMenuBarAnimation() {
        let animate = menuBarMascot && (active.map(isRunning) ?? false)
        if animate, menuBarLoop == nil {
            menuBarLoop = Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(1 / Self.menuBarFPS))
                    self?.menuBarTick &+= 1
                }
            }
        } else if !animate, let loop = menuBarLoop {
            loop.cancel()
            menuBarLoop = nil
            menuBarTick = 0
        }
    }

    private func sessionEnded(_ p: ClaudeProcess, slot: Int?) {
        let ran = Date().timeIntervalSince(p.startedAt)
        guard ran >= Prefs.sessionMinSeconds else { return }
        let who = slot.flatMap(account(slot:))?.displayName ?? "Claude"
        let folder = p.cwd.map { ($0 as NSString).abbreviatingWithTildeInPath } ?? ""
        let duration = Duration.seconds(ran).formatted(.units(allowed: [.hours, .minutes], width: .abbreviated))
        notifier.post(Notice(key: "session:\(p.pid):\(p.startedAt.timeIntervalSince1970)", category: .session,
                             title: "Claude session finished", body: "\(who) · ran \(duration)\(folder.isEmpty ? "" : " · \(folder)")"))
    }

    // MARK: auto-switch

    func startAuto() {
        guard let cli else { return flash(CLIError.notFound.localizedDescription, error: true) }
        Prefs.d.set(true, forKey: Prefs.autoWasOn)
        auto.start(cli: cli, dryRun: Prefs.d.bool(forKey: Prefs.autoDryRun))
    }

    /// The user turning auto-switch off. Quitting stops the child too but keeps `autoWasOn`,
    /// so the next launch picks up where the user left it.
    func stopAuto() {
        Prefs.d.set(false, forKey: Prefs.autoWasOn)
        auto.stop()
    }

    func toggleAuto() { auto.isRunning ? stopAuto() : startAuto() }

    private func handleAutoEvent(_ event: AutoEvent) {
        if let title = event.notificationTitle {
            notifier.post(Notice(key: "auto:\(event.id)", category: .autoSwitch, title: title, body: event.summary))
        }
        if event.kind == "switch" { Task { await refresh() } }
    }

    // MARK: banner

    func flash(_ text: String, error: Bool = false) {
        banner = Banner(text: text, isError: error)
        bannerTask?.cancel()
        bannerTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(Self.bannerSeconds * (error ? 2 : 1)))
            if !Task.isCancelled { self?.banner = nil }
        }
    }
}
