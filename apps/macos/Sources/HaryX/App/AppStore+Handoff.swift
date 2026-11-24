import HaryXCore
import Foundation

/// A live session that cannot go on where it is: its account is at its limit.
struct StuckSession: Identifiable, Hashable {
    let process: ClaudeProcess
    /// The account it ran out on.
    let account: Int
    var id: Int32 { process.pid }
}

/// Continue a session on another account (`haryx ai sessions continue`, plan:
/// `docs/session-handoff-plan.md`). The app only picks the folder and the account; the CLI ends the
/// old process, makes the conversation reachable and resumes it in the terminal it opens.
extension AppStore {
    /// Whether an account has no headroom left.
    func isExhausted(_ number: Int) -> Bool {
        (account(slot: number)?.peakPct ?? 0) >= 100
    }

    /// Sessions to offer a handoff for: session-mode runs on an exhausted account, default-profile
    /// sessions an auto-switch left on an exhausted account, and default-profile sessions on the
    /// active account when it is exhausted itself.
    var stuckSessions: [StuckSession] {
        var out: [StuckSession] = []
        var seen = Set<Int32>()
        for (slot, procs) in sessions {
            for p in procs {
                let ranOn = p.configDir == nil ? (leftBehind[p.pid] ?? slot) : slot
                guard isExhausted(ranOn), seen.insert(p.pid).inserted else { continue }
                out.append(StuckSession(process: p, account: ranOn))
            }
        }
        return out.sorted { $0.process.startedAt < $1.process.startedAt }
    }

    /// The account "best" would pick for a session on `from`: the most headroom, usable, not `from`.
    func bestTarget(excluding from: Int?) -> Account? {
        accounts
            .filter { $0.number != from && !$0.isDisabled && !$0.needsRelogin }
            .filter { ($0.peakPct ?? 100) < 100 }
            .min { ($0.peakPct ?? 100) < ($1.peakPct ?? 100) }
    }

    /// Open a terminal in the session's folder that ends it and resumes it on `target` (nil = best).
    func continueSession(_ process: ClaudeProcess, on target: Account?) {
        guard let cli, cli.flavor == .haryx else {
            return flash("Continuing a session needs the haryx command.", error: true)
        }
        let to = target.map { String($0.number) } ?? "best"
        let command = "\(shellQuote(cli.executable.path)) ai sessions continue \(process.pid) --to \(to)"
        openTerminal(command: command, folder: process.cwd ?? CSwapCLI.home)
        leftBehind[process.pid] = nil
    }

    /// Continue every stuck session on the best account.
    func continueStuck() {
        for s in stuckSessions { continueSession(s.process, on: bestTarget(excluding: s.account)) }
    }

    /// Called when auto-switch leaves `from`: remember the default-profile sessions still running on
    /// it (they keep the old login), and offer to continue them when it was exhausted.
    func noteSwitch(from: Int?) {
        guard let from else { return }
        let left = (sessions[from] ?? []).filter { $0.configDir == nil }
        for p in left { leftBehind[p.pid] = from }
        guard !left.isEmpty, isExhausted(from) else { return }
        let count = left.count == 1 ? "1 session" : "\(left.count) sessions"
        notifier.post(Notice(key: "handoff:\(from):\(left.map(\.pid).sorted())", category: .autoSwitch,
                             title: "\(count) stopped on account \(from)",
                             body: "Open the HaryX menu to continue \(left.count == 1 ? "it" : "them") on the new account."))
    }

    /// Forget left-behind sessions that have ended.
    func pruneLeftBehind(alive: Set<Int32>) {
        leftBehind = leftBehind.filter { alive.contains($0.key) }
    }
}
