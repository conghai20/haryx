import Foundation
import HaryXCore
import Observation

/// The Usage section's "where it went" data: `haryx ai usage … --json`, loaded on demand (plan:
/// `docs/usage-history-plan.md`). The CLI indexes new transcript lines on every call.
@MainActor @Observable
final class UsageModel {
    enum Range: String, CaseIterable, Identifiable {
        case day = "24h", week = "7d", month = "30d", all = "all"
        var id: String { rawValue }
        var title: String {
            switch self {
            case .day: "24 hours"
            case .week: "7 days"
            case .month: "30 days"
            case .all: "Everything kept"
            }
        }
    }

    var range: Range = .week { didSet { reload() } }
    var account: Int? { didSet { reload() } }
    var showCost = false

    private(set) var status: UsageHistory.Status?
    private(set) var byAccount: UsageHistory.Report?
    /// Per account, its tokens per day (for the stacked chart).
    private(set) var byDay: [Int: UsageHistory.Report] = [:]
    private(set) var projects: UsageHistory.Report?
    private(set) var sessions: UsageHistory.Report?
    private(set) var loading = false
    private(set) var error: String?

    @ObservationIgnored var cliProvider: () -> CSwapCLI? = { nil }
    @ObservationIgnored private var loadTask: Task<Void, Never>?
    /// Bumped on every load; a load that is no longer the latest drops its results.
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var weeklyLoop: Task<Void, Never>?

    func reload() {
        loadTask?.cancel()
        loadTask = Task { await load() }
    }

    private func load() async {
        guard let cli = cliProvider(), cli.flavor == .haryx else {
            error = "Usage history needs the haryx command."
            return
        }
        generation += 1
        let mine = generation
        let current = { [weak self] in self?.generation == mine }
        loading = true
        defer { if current() { loading = false } }
        do {
            var base = ["--since", range.rawValue]
            if let account { base += ["--account", String(account)] }
            let first = try await Self.json(UsageHistory.Report.self, cli, ["ai", "usage", "report", "--by", "account"] + base)
            guard current() else { return }
            status = first.history
            guard first.history?.needsChoice == false, first.history?.enabled == true else {
                byAccount = nil
                return
            }
            async let projects = Self.json(UsageHistory.Report.self, cli, ["ai", "usage", "report", "--by", "project"] + base)
            async let sessions = Self.json(UsageHistory.Report.self, cli, ["ai", "usage", "report", "--by", "session"] + base)
            var days: [Int: UsageHistory.Report] = [:]
            for n in first.rows.compactMap({ $0.key.flatMap(Int.init) }) {
                days[n] = try await Self.json(UsageHistory.Report.self, cli,
                                              ["ai", "usage", "report", "--by", "day", "--since", range.rawValue, "--account", String(n)])
            }
            let (p, s) = try await (projects, sessions)
            guard current() else { return }
            byAccount = first
            byDay = days
            self.projects = p
            self.sessions = s
            error = nil
        } catch is CancellationError {
        } catch {
            if current() { self.error = error.localizedDescription }
        }
    }

    /// The first-run answer: read past transcripts (`all`) or start now (`none`).
    func choose(importAll: Bool) {
        Task {
            guard let cli = cliProvider() else { return }
            _ = try? await cli.run(["ai", "usage", "index", "--import", importAll ? "all" : "none", "--json"])
            await load()
        }
    }

    func setKeepDays(_ days: Int) {
        Task {
            guard let cli = cliProvider() else { return }
            _ = try? await cli.run(["config", "set", "history.keepDays", String(days)])
            await load()
        }
    }

    static func json<T: Decodable>(_ type: T.Type, _ cli: CSwapCLI, _ args: [String]) async throws -> T {
        let result = try await cli.run(args + ["--json"])
        do {
            return try UsageHistory.decode(T.self, result.stdout)
        } catch {
            if result.status != 0 { throw CLIError.failed(command: args.joined(separator: " "), message: result.stderr) }
            throw CLIError.decode(command: args.joined(separator: " "), message: "\(error)")
        }
    }

    // MARK: weekly summary

    /// Once a week (when turned on), post a notification summing up the last 7 days.
    func startWeeklySummary(notify: @escaping (String, String) -> Void, name: @escaping (Int) -> String) {
        guard weeklyLoop == nil else { return }
        weeklyLoop = Task { [weak self] in
            while !Task.isCancelled {
                await self?.weeklyTick(notify: notify, name: name)
                try? await Task.sleep(for: .seconds(3600))
            }
        }
    }

    private func weeklyTick(notify: (String, String) -> Void, name: (Int) -> String) async {
        guard Prefs.d.bool(forKey: Prefs.weeklySummary), let cli = cliProvider(), cli.flavor == .haryx else { return }
        let last = Prefs.d.object(forKey: Prefs.lastWeeklySummary) as? Date ?? .distantPast
        guard Date().timeIntervalSince(last) >= 7 * 86_400 else { return }
        guard let summary = try? await Self.json(UsageHistory.Summary.self, cli, ["ai", "usage", "summary", "--since", "7d"]),
              summary.history?.needsChoice == false, !summary.accounts.isEmpty else { return }
        Prefs.d.set(Date(), forKey: Prefs.lastWeeklySummary)
        notify("Your week in Claude Code", summary.lines(name: name).joined(separator: "\n"))
    }
}
