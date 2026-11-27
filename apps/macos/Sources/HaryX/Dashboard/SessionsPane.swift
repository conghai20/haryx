import HaryXCore
import SwiftUI

/// Every running Claude Code session, newest first, with its account and "Continue on…"; then a
/// row per account to start Claude in a new terminal.
struct SessionsPane: View {
    @Environment(AppStore.self) private var store

    private struct Row: Identifiable {
        let process: ClaudeProcess
        let slot: Int?
        var id: Int32 { process.pid }
    }

    private var rows: [Row] {
        let assigned = store.sessions.flatMap { slot, procs in procs.map { Row(process: $0, slot: slot) } }
        let other = store.unassignedSessions.map { Row(process: $0, slot: nil) }
        return (assigned + other).sorted { $0.process.startedAt > $1.process.startedAt }
    }

    var body: some View {
        Form {
            Section {
                let rows = rows
                if rows.isEmpty { Text("No Claude Code sessions running.").foregroundStyle(.secondary) }
                ForEach(rows) { row($0) }
            } header: {
                Text("Running")
            } footer: {
                Text("Continue on… ends a session and resumes the same conversation on another account, in a new terminal.")
            }

            Section("Start Claude as an account in a new terminal") {
                ForEach(store.accounts) { a in
                    HStack {
                        Text("\(a.number). \(a.displayName)").lineLimit(1).truncationMode(.middle)
                        Spacer()
                        Menu {
                            TerminalMenuItems { store.runClaude(as: a, app: $0) }
                        } label: { Label("Run Claude", systemImage: "terminal") } primaryAction: {
                            store.runClaude(as: a)
                        }
                        .menuStyle(.button).buttonStyle(.glass).fixedSize()
                    }
                }
                if store.accounts.isEmpty { Text("No accounts yet.").foregroundStyle(.secondary) }
            }
        }
        .formStyle(.grouped)
        .paneFrame()
    }

    private func row(_ row: Row) -> some View {
        let s = row.process
        let account = row.slot.flatMap(store.account(slot:))
        let stuck = store.stuckSessions.contains { $0.process.pid == s.pid }
        return HStack(spacing: 10) {
            if let account {
                MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 28, phase: store.phase(for: account))
            }
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(account?.displayName ?? "no active account").fontWeight(.medium).lineLimit(1).truncationMode(.middle)
                    if s.configDir != nil, account != nil { BadgeText("own profile") }
                    if stuck { Text("out of quota").font(.caption.weight(.semibold)).foregroundStyle(Brand.warn) }
                }
                Text(s.cwd.map { ($0 as NSString).abbreviatingWithTildeInPath } ?? "—")
                    .font(.caption.monospaced()).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
                    .help(s.cwd ?? "")
                Text("pid \(s.pid) · running for \(Text(s.startedAt, style: .relative))")
                    .font(.caption).foregroundStyle(.tertiary).monospacedDigit()
            }
            Spacer()
            ContinueMenu(process: s, from: row.slot)
        }
    }
}

/// A small grey capsule label, like the Windows app's badge.
private struct BadgeText: View {
    let text: String
    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6).padding(.vertical, 1)
            .background(.fill.tertiary, in: Capsule())
            .foregroundStyle(.secondary)
    }
}
