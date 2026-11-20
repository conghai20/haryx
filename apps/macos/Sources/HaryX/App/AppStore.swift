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
    /// Each status-item image change costs a menu bar re-snapshot in AppKit, so keep this low.
    static let menuBarFPS = 2.0
    static let bannerSeconds = 5.0

    // cswap state
    private(set) var accounts: [Account] = []
    private(set) var lastRefresh: Date?
    private(set) var refreshError: String?
    private(set) var config: [ConfigEntry] = []
    private(set) var configPath = ""
    private(set) var mappings: [DirectoryMapping] = []
    private(set) var priorities: [Int: Int] = [:]
    private(set) var unclaimed: [UnclaimedEntry] = []
    private(set) var version = ""
    private(set) var cli: CSwapCLI?

    // UI state
    var busy: String?
    var banner: Banner?
    /// slot → live Claude processes using it
    private(set) var sessions: [Int: [ClaudeProcess]] = [:]
    private(set) var menuBarTick = 0
    var panelVisible = false

    let auto = AutoSwitchRunner()
    let styles = MascotStyles()
    let notifier = Notifier()

    @ObservationIgnored private var loops: [Task<Void, Never>] = []
    private var menuBarLoop: Task<Void, Never>?
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
                try? await Task.sleep(for: .seconds(Self.sessionScanSeconds))
            }
        })
        watchBackupDir()
        Task { await loadExtras() }
        if Prefs.d.bool(forKey: Prefs.autoOnLaunch) { startAuto() }
    }

    func shutdown() {
        auto.stop()
        loops.forEach { $0.cancel() }
        menuBarLoop?.cancel()
    }

    func relocateCLI() {
        cli = CSwapCLI.locate(override: Prefs.d.string(forKey: Prefs.cswapPath)).map(CSwapCLI.init(executable:))
        refreshError = cli == nil ? CLIError.notFound.localizedDescription : nil
    }

    // MARK: derived

    var active: Account? { accounts.first(where: \.active) }
    var styleMap: [String: MascotStyle] { styles.resolve(accounts) }

    func style(for a: Account) -> MascotStyle {
        styleMap[a.identityKey] ?? MascotStyle(species: Species.defaultFor(key: a.identityKey))
    }

    func isRunning(_ a: Account) -> Bool { !(sessions[a.number] ?? []).isEmpty }

    func mood(for a: Account) -> Mood {
        if a.isDisabled { return .sleeping }
        if a.needsRelogin { return .error }
        if isRunning(a) { return .running }
        if (a.peakPct ?? 0) >= 90 { return .tired }
        return .idle
    }

    func phase(for a: Account) -> Int { Int(stableHash(a.identityKey) % 13) }

    func account(slot: Int) -> Account? { accounts.first { $0.number == slot } }

    func account(for mapping: DirectoryMapping) -> Account? {
        accounts.first { $0.identityKey == mapping.identityKey }
    }

    var menuBarImage: NSImage {
        guard let a = active else { return MascotCache.menuBarImage(MascotStyle(species: .critter), pose: Pose()) }
        let mood = mood(for: a)
        // Two-frame walk (legs swap each tick) while a session runs; a still frame otherwise.
        let pose = mood == .running && menuBarLoop != nil
            ? Pose(altLegs: menuBarTick % 2 == 1, squat: menuBarTick % 2 == 0, mood: .running)
            : Pose(mood: mood == .running ? .idle : mood)
        return MascotCache.menuBarImage(style(for: a), pose: pose)
    }

    // MARK: refresh

    /// Serialised, never dropped: a caller arriving mid-refresh waits, then runs its own, so
    /// post-action refreshes always observe the action's result.
    func refresh() async {
        while let running = refreshTask { await running.value }
        // The task clears itself *before* completing, so a woken waiter never sees a finished
        // task still installed (which would make it spin on an already-completed `value`).
        let task = Task {
            await self.fetchAccounts()
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
            lastRefresh = Date()
            refreshError = nil
            seenSequenceMTime = Self.sequenceMTime()
            notifier.handle(NotificationRules.evaluate(previous: previous, current: list.accounts))
        } catch {
            refreshError = error.localizedDescription
        }
    }

    /// Settings, mappings, priorities, unclaimed, version: cheap, refreshed on demand.
    func loadExtras() async {
        guard let cli else { return }
        async let configList = try? cli.json(ConfigList.self, ["config", "list", "--json"])
        async let prio = try? cli.checked(["priority"])
        async let uncl = try? cli.checked(["unclaimed"])
        async let ver = try? cli.checked(["--version"])
        // Keep the previous values when a call fails rather than blanking the UI.
        if let c = await configList { config = c.settings; configPath = c.path }
        if let p = await prio { priorities = TextParsers.priorities(p) }
        if let u = await uncl { unclaimed = TextParsers.unclaimed(u) }
        if let v = await ver { version = v.trimmingCharacters(in: .whitespacesAndNewlines) }
        loadMappings()
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
        updateMenuBarAnimation()
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

    func updateMenuBarAnimation() {
        let animate = Prefs.d.bool(forKey: Prefs.menuBarAnimate) && (active.map(isRunning) ?? false)
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
        }
    }

    // MARK: auto-switch

    func startAuto() {
        guard let cli else { return flash(CLIError.notFound.localizedDescription, error: true) }
        auto.start(cli: cli, dryRun: Prefs.d.bool(forKey: Prefs.autoDryRun))
    }

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
