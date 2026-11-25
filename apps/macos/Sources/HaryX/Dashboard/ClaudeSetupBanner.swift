import HaryXCore
import SwiftUI

/// Shown while Claude Code is missing or has never signed in: nothing in the AI section works until
/// it is fixed, so say what is wrong and offer the fix in one click.
struct ClaudeSetupBanner: View {
    @Environment(AppStore.self) private var store
    /// The menu bar panel shows the problem and its main button only.
    var compact = false
    @State private var checking = false

    var body: some View {
        if let setup = store.claudeSetup, !setup.ready, let problem = setup.next {
            VStack(alignment: .leading, spacing: 8) {
                Label(problem.message, systemImage: "exclamationmark.triangle.fill")
                    .font(compact ? .caption.weight(.semibold) : .headline)
                    .foregroundStyle(Brand.warn)
                if !compact {
                    // The fix carries commands in backticks: render them as code.
                    Text(LocalizedStringKey(problem.fix)).font(.callout).textSelection(.enabled)
                }
                HStack {
                    switch problem.kind {
                    case .claudeMissing:
                        Button("Install Claude Code") { store.installClaude() }.buttonStyle(.glassProminent)
                    case .notSignedIn:
                        Button("Open Claude to Sign In") { store.signInToClaude() }.buttonStyle(.glassProminent)
                    case .other:
                        EmptyView()
                    }
                    Button("Check Again") {
                        checking = true
                        Task { await store.checkClaudeSetup(); checking = false }
                    }
                    .buttonStyle(.glass)
                    .disabled(checking)
                }
                .controlSize(compact ? .small : .regular)
            }
            .padding(compact ? 10 : 14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassEffect(.regular.tint(Brand.warn.opacity(0.1)), in: .rect(cornerRadius: 16))
        }
    }
}
