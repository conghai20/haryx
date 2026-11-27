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
        account(slot: number)?.isExhausted ?? false
    }

    /// Sessions to offer a handoff for: `haryx run` sessions on an exhausted account. Those run in
    /// their account's own profile, so a switch does not move them. A default-profile session is
    /// never stuck this way: it picks up a switched account while it runs, so switching is its fix.
    var stuckSessions: [StuckSession] {
        sessions
            .flatMap { slot, procs in
                procs.filter { $0.configDir != nil && isExhausted(slot) }.map { StuckSession(process: $0, account: slot) }
            }
            .sorted { $0.process.startedAt < $1.process.startedAt }
    }

    /// The account "best" would pick for a session on `from`: usable with headroom, not `from`, in
    /// `from`'s group when one there can take it (as the CLI's `--to best` picks).
    func bestTarget(excluding from: Int?) -> Account? {
        AccountGroups.handoffTarget(accounts, from: from.flatMap(account(slot:)))
    }

    /// Open a terminal in the session's folder that ends it and resumes it on `target` (nil = best).
    func continueSession(_ process: ClaudeProcess, on target: Account?) {
        guard let cli, cli.flavor == .haryx else {
            return flash("Continuing a session needs the haryx command.", error: true)
        }
        let to = target.map { String($0.number) } ?? "best"
        let command = "\(shellQuote(cli.executable.path)) ai sessions continue \(process.pid) --to \(to)"
        openTerminal(command: command, folder: process.cwd ?? CSwapCLI.home)
    }

    /// Continue every stuck session on the best account.
    func continueStuck() {
        for s in stuckSessions { continueSession(s.process, on: bestTarget(excluding: s.account)) }
    }
}
