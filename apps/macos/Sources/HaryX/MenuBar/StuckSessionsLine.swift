import HaryXCore
import SwiftUI

/// The menu bar panel's row for sessions stopped on an exhausted account, with one click to
/// continue them all on the best account.
struct StuckSessionsLine: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let stuck = store.stuckSessions
        if let first = stuck.first, let target = store.bestTarget(excluding: first.account) {
            HStack {
                Label(title(stuck), systemImage: "pause.circle.fill")
                    .font(.caption.weight(.medium)).foregroundStyle(.orange)
                    .lineLimit(1)
                Spacer()
                Button("Continue on \(target.number)") { store.continueStuck() }
                    .controlSize(.small)
                    .help("Ends \(stuck.count == 1 ? "it" : "them") and resumes the same conversation\(stuck.count == 1 ? "" : "s") on \(target.displayName).")
            }
            .padding(.top, 8)
        }
    }

    private func title(_ stuck: [StuckSession]) -> String {
        let accounts = Set(stuck.map(\.account)).sorted().map(String.init).joined(separator: ", ")
        return "\(stuck.count == 1 ? "1 session" : "\(stuck.count) sessions") stopped on account \(accounts)"
    }
}
