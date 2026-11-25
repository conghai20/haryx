import Foundation
import HaryXCore
import Observation

/// The HaryX account and encrypted sync. Everything goes through the `haryx` CLI (`whoami`,
/// `login`, `sync …`); the app never holds a HaryX token or the sync key.
///
/// Sync runs at launch, every 15 minutes, and 5 s after a synced preference changes, only while
/// signed in, unlocked and the "Sync" toggle is on. Failures are quiet: `problem` is a status line.
@MainActor @Observable
final class AccountSync {
    static let interval: Duration = .seconds(15 * 60)
    static let debounce: Duration = .seconds(5)

    enum SignIn: Equatable {
        case idle
        /// `haryx login` is waiting for the browser; `url` is the page, when the CLI said it.
        case waiting(url: URL?, opened: Bool)
        case failed(String)
    }

    /// How to sign in to HaryX; the raw value is `haryx login --provider`.
    enum SignInProvider: String, CaseIterable, Identifiable {
        case github, google
        var id: String { rawValue }

        var label: String {
            switch self {
            case .github: "GitHub"
            case .google: "Google"
            }
        }

        /// Google waits for its OAuth client on the hosted project.
        var isAvailable: Bool { self == .github }
    }

    /// nil until first read (or with a cswap backend).
    private(set) var whoami: WhoAmI?
    /// Why `whoami` could not be read (an older haryx without accounts).
    private(set) var loadError: String?
    private(set) var status: SyncStatus?
    /// `sync has-passphrase`: decides "set" vs "enter"; nil until known.
    private(set) var hasPassphrase: Bool?
    private(set) var passphraseCheckError: String?
    private(set) var signIn = SignIn.idle
    /// The provider of the sign-in in progress, or of the last one.
    private(set) var signInProvider = SignInProvider.github
    private(set) var isSyncing = false
    /// Why the last sync failed; nil after a good one.
    private(set) var problem: SyncProblem?
    /// An account action (unlock, sign out, delete…) is running.
    private(set) var working = false

    /// App-local, never synced: whether this Mac syncs at all.
    var syncEnabled = Prefs.d.bool(forKey: Prefs.syncEnabled) {
        didSet {
            Prefs.d.set(syncEnabled, forKey: Prefs.syncEnabled)
            if syncEnabled { Task { await syncNow() } }
        }
    }

    @ObservationIgnored var cliProvider: () -> CSwapCLI? = { nil }
    /// Told which UserDefaults keys a pull changed, so their side effects run.
    @ObservationIgnored var onPrefsApplied: (Set<String>) -> Void = { _ in }
    /// A sync changed settings or accounts on this Mac.
    @ObservationIgnored var onLocalChanged: () -> Void = {}

    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored private var debounceTask: Task<Void, Never>?
    @ObservationIgnored private var observer: NSObjectProtocol?
    @ObservationIgnored private var loginProcess: Process?
    @ObservationIgnored private var loginCancelled = false
    @ObservationIgnored private var rerun = false
    @ObservationIgnored private var applying = false
    @ObservationIgnored private var lastSnapshot: [String: Any] = [:]

    /// Accounts need the haryx backend; cswap has none.
    var cli: CSwapCLI? { cliProvider().flatMap { $0.flavor == .haryx ? $0 : nil } }
    var user: HaryXUser? { whoami?.signedIn == true ? whoami?.user : nil }
    var isUnlocked: Bool { user != nil && status?.unlocked == true }
    var canSync: Bool { isUnlocked && syncEnabled }
    var otherMachineAccounts: [OtherMachineAccount] { user == nil ? [] : status?.otherMachineAccounts ?? [] }

    // MARK: lifecycle

    func start() {
        guard loop == nil else { return }
        lastSnapshot = AppPrefsSync.export(Self.storedPrefs())
        observer = NotificationCenter.default.addObserver(forName: UserDefaults.didChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.defaultsChanged() }
        }
        loop = Task { [weak self] in
            while !Task.isCancelled {
                await self?.reload()
                await self?.syncNow()
                try? await Task.sleep(for: Self.interval)
            }
        }
    }

    func stop() {
        loop?.cancel()
        debounceTask?.cancel()
        loginProcess?.terminate()
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }

    /// Re-read who is signed in and the local sync status (no network), then whether a passphrase
    /// exists when the form needs to know.
    func reload() async {
        guard let cli else { return }
        switch await cli.reply(WhoAmI.self, ["whoami", "--json"]) {
        case .success(let w): whoami = w; loadError = nil
        case .failure(let f): loadError = f.message
        }
        await reloadStatus()
        if user != nil, status?.unlocked == false, hasPassphrase == nil { await checkPassphrase() }
        if user == nil { hasPassphrase = nil; problem = nil }
    }

    private func reloadStatus() async {
        guard let cli, user != nil else { status = nil; return }
        if case .success(let s) = await cli.reply(SyncStatus.self, ["sync", "status", "--json"]) { status = s }
    }

    func checkPassphrase() async {
        guard let cli else { return }
        switch await cli.reply(PassphraseCheck.self, ["sync", "has-passphrase", "--json"]) {
        case .success(let r): hasPassphrase = r.hasPassphrase; passphraseCheckError = nil
        case .failure(let f): passphraseCheckError = SyncProblem(f) == .unavailable ? SyncProblem.unavailable.summary : f.message
        }
    }

    // MARK: sync

    /// One sync, never two at once: a request arriving mid-sync runs once more afterwards.
    func syncNow() async {
        guard let cli, canSync else { return }
        if isSyncing { rerun = true; return }
        isSyncing = true
        await pushPrefs(cli)
        switch await cli.reply(SyncResult.self, ["sync", "--json"]) {
        case .success(let result):
            problem = nil
            if result.changed.contains("app-prefs") { await pullPrefs(cli) }
            if !Set(result.changed).isDisjoint(with: ["settings", "account", "credential"]) { onLocalChanged() }
        case .failure(let failure):
            let p = SyncProblem(failure)
            problem = p
            if p == .signedOut { await reload() }
            if p.needsUnlock { hasPassphrase = true }
        }
        await reloadStatus()
        isSyncing = false
        if rerun {
            rerun = false
            await syncNow()
        }
    }

    private func defaultsChanged() {
        guard !applying else { return }
        let snapshot = AppPrefsSync.export(Self.storedPrefs())
        guard !AppPrefsSync.same(snapshot, lastSnapshot) else { return }
        lastSnapshot = snapshot
        debounceTask?.cancel()
        debounceTask = Task { [weak self] in
            try? await Task.sleep(for: Self.debounce)
            guard !Task.isCancelled else { return }
            await self?.syncNow()
        }
    }

    /// Writes this Mac's synced preferences into the CLI's copy, when they differ from it.
    private func pushPrefs(_ cli: CSwapCLI) async {
        guard let current = await Self.prefsGet(cli) else { return }
        let merged = AppPrefsSync.overlay(AppPrefsSync.export(Self.storedPrefs()), on: current)
        guard !AppPrefsSync.same(merged, current),
              let data = try? JSONSerialization.data(withJSONObject: merged, options: .sortedKeys) else { return }
        if case .failure(let f) = await cli.reply(OKReply.self, ["sync", "prefs", "set"], stdin: String(decoding: data, as: UTF8.self)) {
            problem = .failed(f.message)
        }
    }

    /// Applies the merged preferences another machine changed.
    private func pullPrefs(_ cli: CSwapCLI) async {
        guard let synced = await Self.prefsGet(cli) else { return }
        var changed = Set<String>()
        applying = true
        for (key, value) in AppPrefsSync.importValues(synced) {
            let current = Prefs.d.object(forKey: key)
            if let current, AppPrefsSync.same(["v": current], ["v": value]) { continue }
            Prefs.d.set(value, forKey: key)
            changed.insert(key)
        }
        applying = false
        lastSnapshot = AppPrefsSync.export(Self.storedPrefs())
        if !changed.isEmpty { onPrefsApplied(changed) }
    }

    private static func prefsGet(_ cli: CSwapCLI) async -> [String: Any]? {
        guard let r = try? await cli.run(["sync", "prefs", "get"]), r.status == 0 else { return nil }
        return (try? JSONSerialization.jsonObject(with: r.stdout)) as? [String: Any]
    }

    /// The preferences this Mac has set (not the registered defaults).
    private static func storedPrefs() -> [String: Any] {
        Prefs.d.persistentDomain(forName: Bundle.main.bundleIdentifier ?? ProcessInfo.processInfo.processName) ?? [:]
    }

    // MARK: sign-in

    /// `haryx login --provider <id> --json`: it opens the browser and waits up to 5 minutes.
    func signIn(with provider: SignInProvider) {
        guard let cli, loginProcess == nil, provider.isAvailable else { return }
        signInProvider = provider
        let process = Process()
        process.executableURL = cli.executable
        process.arguments = ["login", "--provider", provider.rawValue, "--json"]
        process.environment = CSwapCLI.environment
        process.standardInput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        let out = Pipe()
        process.standardOutput = out
        do {
            try process.run()
        } catch {
            signIn = .failed(error.localizedDescription)
            return
        }
        loginProcess = process
        loginCancelled = false
        signIn = .waiting(url: nil, opened: true)
        Task {
            do {
                for try await line in out.fileHandleForReading.bytes.lines {
                    if let event = LoginEvent.parse(line: line) { await handle(event) }
                }
            } catch {}
            loginFinished()
        }
    }

    func cancelSignIn() {
        loginCancelled = true
        loginProcess?.terminate()
    }

    private func handle(_ event: LoginEvent) async {
        switch event {
        case .browser(let url, let opened):
            signIn = .waiting(url: url, opened: opened)
        case .signedIn(let user):
            signIn = .idle
            whoami = WhoAmI(signedIn: true, user: user)
            hasPassphrase = nil
            await reload()
        case .error(_, let message):
            signIn = .failed(message)
        }
    }

    private func loginFinished() {
        loginProcess = nil
        guard case .waiting = signIn else { return }
        signIn = loginCancelled ? .idle : .failed("Sign-in stopped before it finished.")
    }

    // MARK: account actions (each returns an error message, or nil on success)

    /// Enters the passphrase, or sets it when there is none yet. It travels on stdin, never in argv.
    func unlock(_ passphrase: String) async -> String? {
        await perform(["sync", "unlock", "--passphrase-stdin", "--json"], stdin: passphrase + "\n") {
            self.hasPassphrase = true
            self.problem = nil
            await self.reloadStatus()
            Task { await self.syncNow() }
        }
    }

    func changePassphrase(_ passphrase: String) async -> String? {
        await perform(["sync", "passphrase", "--passphrase-stdin", "--json"], stdin: passphrase + "\n") {}
    }

    /// Signs out and forgets this Mac's sync key.
    func signOut() async -> String? {
        await perform(["logout", "--json"]) { await self.reload() }
    }

    func deleteAccount() async -> String? {
        await perform(["account", "delete", "--yes", "--json"]) { await self.reload() }
    }

    func forget(_ account: OtherMachineAccount) async -> String? {
        await perform(["sync", "forget", account.email, "--json"]) { await self.reloadStatus() }
    }

    private func perform(_ args: [String], stdin: String? = nil, then: () async -> Void) async -> String? {
        guard let cli else { return CLIError.notFound.localizedDescription }
        working = true
        defer { working = false }
        switch await cli.reply(OKReply.self, args, stdin: stdin) {
        case .success:
            await then()
            return nil
        case .failure(let f):
            return f.code == "wrong_passphrase" ? "That passphrase is not right." : f.message
        }
    }
}
