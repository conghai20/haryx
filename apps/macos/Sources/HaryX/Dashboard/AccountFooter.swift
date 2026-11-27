import HaryXCore
import SwiftUI

/// The dashboard sidebar's footer: the HaryX account. Signed out, a menu of ways to sign in;
/// signing in, progress and Cancel; signed in, who it is and a menu to manage the account.
struct AccountFooter: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let sync = store.accountSync
        if sync.cli != nil {
            VStack(alignment: .leading, spacing: 6) {
                Divider()
                content(sync)
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 10)
            .task { await sync.reload() }
        }
    }

    @ViewBuilder private func content(_ sync: AccountSync) -> some View {
        if let user = sync.user {
            SignedInMenu(sync: sync, user: user)
        } else {
            switch sync.signIn {
            case .waiting(let url, _):
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Signing in with \(sync.signInProvider.label)…")
                        .font(.callout).foregroundStyle(.secondary).lineLimit(1)
                    Spacer(minLength: 0)
                    if let url {
                        Link(destination: url) { Image(systemName: "arrow.up.forward.square") }
                            .help("Open the sign-in page again")
                    }
                    Button { sync.cancelSignIn() } label: { Image(systemName: "xmark.circle.fill") }
                        .buttonStyle(.plain).foregroundStyle(.secondary)
                        .help("Cancel sign-in")
                }
            case .idle, .failed:
                if sync.signedOutFromWeb, sync.signIn == .idle {
                    Label("Signed out of HaryX from the web", systemImage: "person.crop.circle.badge.xmark")
                        .font(.caption).foregroundStyle(Brand.warn).lineLimit(2)
                }
                if case .failed(let message) = sync.signIn {
                    Label(message, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption).foregroundStyle(Brand.warn).lineLimit(3)
                }
                SignInMenu(sync: sync, title: "Sign in to HaryX")
                    .menuStyle(.button).buttonStyle(.glass)
                    .frame(maxWidth: .infinity)
            }
        }
    }
}

/// A menu of the ways to sign in to HaryX. Unavailable providers are listed, disabled.
struct SignInMenu: View {
    let sync: AccountSync
    let title: String

    var body: some View {
        Menu {
            ForEach(AccountSync.SignInProvider.allCases) { provider in
                Button(provider.menuTitle) {
                    sync.signIn(with: provider)
                }
            }
        } label: {
            Label(title, systemImage: "person.crop.circle")
                .frame(maxWidth: .infinity)
        }
        .help("Sign in to sync settings between your machines")
    }
}

private struct SignedInMenu: View {
    let sync: AccountSync
    let user: HaryXUser
    @State private var error: String?

    var body: some View {
        Menu {
            Section("Signed in with \(user.providerName)") {
                Button("Account & Sync…") { DashboardRouter.shared.section = .haryxAccount }
                Link("Open HaryX on the Web", destination: HaryXLinks.web)
                Button("Sign Out") { Task { error = await sync.signOut() } }
            }
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "person.crop.circle.fill").font(.title2).foregroundStyle(.tint)
                VStack(alignment: .leading, spacing: 0) {
                    Text(user.email).font(.callout).lineLimit(1).truncationMode(.middle)
                    Text(statusLine).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 0)
                Image(systemName: "chevron.up.chevron.down").font(.caption).foregroundStyle(.secondary)
            }
            .contentShape(Rectangle())
        }
        .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden)
        .disabled(sync.working)
        if let error { Text(error).font(.caption).foregroundStyle(Brand.limit).lineLimit(2) }
    }

    private var statusLine: String {
        if sync.isSyncing { return "Syncing…" }
        if sync.problem != nil { return "Sync needs attention" }
        if !sync.isUnlocked { return "Sync locked" }
        return sync.syncEnabled ? "Sync on" : "Sync off"
    }
}
