import HaryXCore
import SwiftUI

/// HaryX › HaryX Account: sign in, unlock sync with the passphrase, and sync settings between
/// machines. Claude logins never leave this Mac.
struct HaryXAccountPane: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let sync = store.accountSync
        Form {
            if sync.cli == nil {
                Section {
                    Text("HaryX accounts need the haryx command-line backend.").foregroundStyle(.secondary)
                }
            } else if let user = sync.user {
                SignedInSection(sync: sync, user: user)
                if sync.isUnlocked {
                    SyncSection(sync: sync)
                } else {
                    PassphraseSection(sync: sync)
                }
                AccountActionsSection(sync: sync)
            } else {
                SignedOutSection(sync: sync)
            }
        }
        .formStyle(.grouped)
        .paneFrame()
        .task { await sync.reload() }
    }
}

private struct SignedOutSection: View {
    let sync: AccountSync

    var body: some View {
        Section {
            Text("Sign in to sync settings, preferences and account setup between your Macs and PCs. Your Claude logins never leave this Mac.")
                .fixedSize(horizontal: false, vertical: true)
            switch sync.signIn {
            case .waiting(let url, let opened):
                HStack {
                    ProgressView().controlSize(.small)
                    Text(opened ? "Finish signing in with \(sync.signInProvider.label) in your browser…" : "Open the sign-in page to continue…")
                        .foregroundStyle(.secondary)
                    Spacer()
                    if let url { Link(opened ? "Open Again" : "Open Sign-In Page", destination: url) }
                    Button("Cancel") { sync.cancelSignIn() }
                }
            case .idle, .failed:
                if case .failed(let message) = sync.signIn {
                    Label(message, systemImage: "exclamationmark.triangle.fill").foregroundStyle(Brand.warn)
                }
                SignInMenu(sync: sync, title: sync.signIn == .idle ? "Sign In…" : "Try Again")
                    .buttonStyle(.glassProminent)
                    .fixedSize()
            }
            if let error = sync.loadError {
                Text(error).font(.caption).foregroundStyle(.secondary)
            }
        } header: { Text("HaryX account") }
    }
}

private struct SignedInSection: View {
    let sync: AccountSync
    let user: HaryXUser

    var body: some View {
        Section("HaryX account") {
            LabeledContent("Signed in as", value: user.email)
            LabeledContent("With", value: user.providerName)
        }
    }
}

/// Set a passphrase (typed twice) when the account has none, else enter it.
private struct PassphraseSection: View {
    let sync: AccountSync
    @State private var passphrase = ""
    @State private var confirm = ""
    @State private var error: String?

    var body: some View {
        Section {
            if let creating = sync.hasPassphrase.map(!) {
                if creating {
                    Text("Choose a sync passphrase. It encrypts everything HaryX syncs, so only your machines can read it. HaryX cannot recover this passphrase. Any Mac or PC that is still unlocked can set a new one.")
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    Text(sync.problem == .keyChanged ? SyncProblem.keyChanged.summary : "Enter your sync passphrase to sync this Mac.")
                        .fixedSize(horizontal: false, vertical: true)
                }
                SecureField("Passphrase", text: $passphrase)
                    .onSubmit { if !creating { submit(creating: false) } }
                if creating { SecureField("Confirm passphrase", text: $confirm).onSubmit { submit(creating: true) } }
                if let error { Text(error).foregroundStyle(Brand.limit) }
                HStack {
                    Spacer()
                    if sync.working { ProgressView().controlSize(.small) }
                    Button(creating ? "Set Passphrase" : "Unlock") { submit(creating: creating) }
                        .buttonStyle(.glassProminent)
                        .disabled(sync.working || passphrase.isEmpty)
                }
            } else if let problem = sync.passphraseCheckError {
                HStack {
                    Label(problem, systemImage: "exclamationmark.triangle.fill").foregroundStyle(Brand.warn)
                    Spacer()
                    Button("Try Again") { Task { await sync.checkPassphrase() } }
                }
            } else {
                HStack { ProgressView().controlSize(.small); Text("Checking…").foregroundStyle(.secondary) }
            }
        } header: { Text("Sync passphrase") }
    }

    private func submit(creating: Bool) {
        if creating, let problem = SyncPassphrase.problem(passphrase, confirm: confirm) { error = problem; return }
        Task {
            error = await sync.unlock(passphrase)
            if error == nil { passphrase = ""; confirm = "" }
        }
    }
}

private struct SyncSection: View {
    let sync: AccountSync
    @State private var changingPassphrase = false

    var body: some View {
        Section {
            Toggle("Sync this Mac", isOn: Binding(get: { sync.syncEnabled }, set: { sync.syncEnabled = $0 }))
            HStack {
                statusLine
                Spacer()
                Button("Sync Now") { Task { await sync.syncNow() } }
                    .disabled(!sync.canSync || sync.isSyncing)
            }
            Button("Change Passphrase…") { changingPassphrase = true }
        } header: { Text("Sync") } footer: {
            Text("Settings, app preferences, account names and API keys sync end-to-end encrypted. Each machine signs in to Claude itself.")
        }
        .sheet(isPresented: $changingPassphrase) { ChangePassphraseSheet(sync: sync) }

        if let overwritten = sync.status?.overwritten, !overwritten.isEmpty {
            Section("Recent overwrites") {
                ForEach(overwritten) { o in
                    HStack {
                        Text("\(o.label): kept \(o.keptLocal ? "this Mac's" : "the other machine's") value")
                        Spacer()
                        Text(o.at.formatted(.relative(presentation: .named))).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    @ViewBuilder private var statusLine: some View {
        if sync.isSyncing {
            HStack { ProgressView().controlSize(.small); Text("Syncing…").foregroundStyle(.secondary) }
        } else if let problem = sync.problem {
            Label(problem.summary, systemImage: "exclamationmark.triangle.fill").foregroundStyle(Brand.warn)
        } else if !sync.syncEnabled {
            Text("Sync is off on this Mac.").foregroundStyle(.secondary)
        } else if let last = sync.status?.lastSyncAt {
            Text("Last synced \(last.formatted(.relative(presentation: .named)))").foregroundStyle(.secondary)
        } else {
            Text("Not synced yet.").foregroundStyle(.secondary)
        }
    }
}

private struct AccountActionsSection: View {
    let sync: AccountSync
    @State private var confirmDelete = false
    @State private var error: String?

    var body: some View {
        Section {
            HStack {
                Button("Sign Out") { Task { error = await sync.signOut() } }
                Spacer()
                Button("Delete Account…", role: .destructive) { confirmDelete = true }
            }
            .disabled(sync.working)
            if let error { Text(error).foregroundStyle(Brand.limit) }
        } footer: {
            Text("Signing out forgets this Mac's sync key. Your Claude accounts on this Mac are not touched.")
        }
        .alert("Delete your HaryX account?", isPresented: $confirmDelete) {
            Button("Delete Account", role: .destructive) { Task { error = await sync.deleteAccount() } }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes everything synced, on every machine. Your Claude accounts on this Mac are not touched.")
        }
    }
}

private struct ChangePassphraseSheet: View {
    let sync: AccountSync
    @Environment(\.dismiss) private var dismiss
    @State private var passphrase = ""
    @State private var confirm = ""
    @State private var error: String?

    var body: some View {
        Form {
            Section {
                Text("The new passphrase replaces the old one on every machine; the others ask for it on their next sync. HaryX cannot recover this passphrase. Any Mac or PC that is still unlocked can set a new one.")
                    .fixedSize(horizontal: false, vertical: true)
                SecureField("New passphrase", text: $passphrase)
                SecureField("Confirm passphrase", text: $confirm).onSubmit(submit)
                if let error { Text(error).foregroundStyle(Brand.limit) }
            }
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button("Change", action: submit).disabled(sync.working || passphrase.isEmpty)
            }
        }
    }

    private func submit() {
        if let problem = SyncPassphrase.problem(passphrase, confirm: confirm) { error = problem; return }
        Task {
            error = await sync.changePassphrase(passphrase)
            if error == nil { dismiss() }
        }
    }
}

/// Accounts › "On your other machines": Claude accounts another machine has and this Mac does not.
/// Sign in here runs the usual Claude sign-in; the sync after it applies the alias and the rest.
struct OtherMachineAccountsSection: View {
    @Environment(AppStore.self) private var store
    @State private var forgetting: OtherMachineAccount?

    var body: some View {
        let accounts = store.accountSync.otherMachineAccounts
        if !accounts.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                Text("On your other machines").font(.headline).padding(.bottom, 6)
                ForEach(accounts) { a in
                    Divider()
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(a.alias ?? a.email).lineLimit(1)
                            Text([a.alias != nil ? a.email : nil, a.organizationName].compactMap { $0 }.joined(separator: " · "))
                                .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                        Spacer()
                        Button("Sign In Here") { store.signInNew() }
                            .buttonStyle(.glass)
                            .disabled(store.addLoginPending)
                            .help("Sign in to \(a.email) on this Mac with Claude's own sign-in")
                        Button("Forget") { forgetting = a }
                            .help("Stop offering \(a.email) to your machines")
                    }
                    .controlSize(.small)
                    .padding(.vertical, 6)
                }
            }
            .padding(.horizontal, 20).padding(.bottom, 20)
            .alert("Forget \(forgetting?.email ?? "this account")?", isPresented: Binding(get: { forgetting != nil }, set: { if !$0 { forgetting = nil } }),
                   presenting: forgetting) { a in
                Button("Forget", role: .destructive) {
                    Task { if let error = await store.accountSync.forget(a) { store.flash(error, error: true) } }
                }
                Button("Cancel", role: .cancel) {}
            } message: { _ in
                Text("HaryX removes it, and its synced API key or token if it has one, so your machines stop offering it. Machines already signed in to it keep it.")
            }
        }
    }
}
