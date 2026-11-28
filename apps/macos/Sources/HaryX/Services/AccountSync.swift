import Foundation
import HaryXCore
import Observation

/// The HaryX account and encrypted sync. Everything goes through the `haryx` CLI (`whoami`,
/// `login`, `sync …`); the app never holds a HaryX token or the sync key.
///
/// Sync runs at launch, every 15 minutes, 5 s after a synced preference changes, and 1 s after an
/// account's groups change, only while signed in, unlocked and the "Sync" toggle is on. Failures are quiet: `problem` is a status line.
@MainActor @Observable
final class AccountSync {
    static let interval: Duration = .seconds(15 * 60)
    static let debounce: Duration = .seconds(5)
    /// After a group change: short, so it reaches other machines at once, but long enough that a
    /// change made of several CLI calls (New Group with many accounts) syncs once.
    static let changeDebounce: Duration = .seconds(1)

    enum SignIn: Equatable {
        case idle
        /// `haryx login` is waiting for the browser; `url` is the page, when the CLI said it.
        case waiting(url: URL?, opened: Bool)
        case failed(String)
    }

    /// The one way into HaryX: WeCoAp SSO, whose page offers every sign-in method.
    static let signInTitle = "Sign in with WeCoAp"

    /// Consent changes record where they were made.
    static let consentSource = "macos"

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
    private(set) var isSyncing = false
    /// Why the last sync failed; nil after a good one.
    private(set) var problem: SyncProblem?
    /// An account action (unlock, sign out, consent…) is running.
    private(set) var working = false
    /// This Mac's sign-in was ended from elsewhere (HaryX on the web, the WeCoAp account page).
    private(set) var signedOutFromWeb = false
    /// The consent sheet to show; set to ask for a consent.
    var consentRequest: ConsentKind?

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
    /// A sign-out from this app is running, so losing the session is not news.
    @ObservationIgnored private var signingOut = false

    /// Accounts need the haryx backend; cswap has none.
    var cli: CSwapCLI? { cliProvider().flatMap { $0.flavor == .haryx ? $0 : nil } }
    var user: HaryXUser? { whoami?.signedIn == true ? whoami?.user : nil }
    var isUnlocked: Bool { user != nil && status?.unlocked == true }
    var canSync: Bool { isUnlocked && syncEnabled && syncConsent != false }
    /// The Sync consent; nil when the CLI could not say (offline, older haryx).
    var syncConsent: Bool? { user == nil ? nil : whoami?.consent(.sync) }
    /// "Show this Mac on HaryX web"; nil when unknown.
    var machinesConsent: Bool? { user == nil ? nil : whoami?.consent(.machines) }
    /// Ask for the Sync consent before the passphrase and the first sync.
    var needsSyncConsent: Bool { user != nil && (syncConsent == false || problem == .consentRequired) }
    /// Usage totals leave this Mac: signed in, Sync consented and sync on here.
    var usageSyncs: Bool { user != nil && syncConsent == true && syncEnabled }
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
        let wasSignedIn = user != nil
        switch await cli.reply(WhoAmI.self, ["whoami", "--json"]) {
        case .success(let w):
            whoami = w
            loadError = nil
            // whoami renews the session; a refused renewal answers signed out.
            if wasSignedIn, user == nil, !signingOut { signedOutFromWeb = true }
        case .failure(let f): loadError = f.message
        }
        await reloadStatus()
        if user != nil, syncConsent != false, status?.unlocked == false, hasPassphrase == nil { await checkPassphrase() }
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
            if p == .signedOutFromWeb { signedOutFromWeb = true }
            if p.isSignedOut { await reload() }
            if p.needsUnlock { hasPassphrase = true }
        }
        await reloadStatus()
        isSyncing = false
        if rerun {
            rerun = false
            await syncNow()
        }
    }

    /// A synced account setting changed here (a group added, removed, renamed or deleted): sync
    /// shortly, rather than on the 15-minute loop. Several changes in a row sync once.
    func syncSoon() {
        guard canSync else { return }
        debounceTask?.cancel()
        debounceTask = Task { [weak self] in
            try? await Task.sleep(for: Self.changeDebounce)
            guard !Task.isCancelled else { return }
            await self?.syncNow()
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

    /// `haryx login --json`: it opens WeCoAp SSO in the browser and waits up to 5 minutes.
    func startSignIn() {
        guard let cli, loginProcess == nil else { return }
        signedOutFromWeb = false
        let process = Process()
        process.executableURL = cli.executable
        process.arguments = ["login", "--json"]
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
            signedOutFromWeb = false
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
        signingOut = true
        defer { signingOut = false }
        return await perform(["logout", "--json"]) { await self.reload() }
    }

    func dismissSignedOutNotice() { signedOutFromWeb = false }

    /// Grants or withdraws `kind` (withdrawing erases its server data; the UI confirmed first).
    func setConsent(_ kind: ConsentKind, granted: Bool) async -> String? {
        guard let cli else { return CLIError.notFound.localizedDescription }
        working = true
        defer { working = false }
        switch await cli.reply(Consents.self, kind.arguments(grant: granted, source: Self.consentSource)) {
        case .success(let consents):
            whoami = whoami?.with(consents)
            if kind == .sync, problem == .consentRequired { problem = nil }
            if kind == .sync, granted {
                if hasPassphrase == nil, status?.unlocked == false { await checkPassphrase() }
                Task { await self.syncNow() }
            }
            return nil
        case .failure(let f):
            return f.message
        }
    }

    /// `haryx account export PATH`: one JSON file of everything HaryX keeps about this account.
    func export(to url: URL) async -> String? {
        guard let cli else { return CLIError.notFound.localizedDescription }
        working = true
        defer { working = false }
        switch await cli.reply(ExportResult.self, ["account", "export", url.path, "--json"]) {
        case .success: return nil
        case .failure(let f): return f.message
        }
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
