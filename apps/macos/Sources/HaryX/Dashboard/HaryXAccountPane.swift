import HaryXCore
import SwiftUI
import UniformTypeIdentifiers

/// HaryX › HaryX Account: sign in, consent to sync, unlock it with the passphrase, and sync
/// settings between machines; then privacy: showing this Mac on HaryX web, withdrawing a consent,
/// exporting. Claude logins never leave this Mac. Mirrors Windows `panes/Account.svelte`.
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
                if sync.needsSyncConsent {
                    SyncConsentSection(sync: sync)
                } else if sync.isUnlocked {
                    SyncSection(sync: sync)
                } else {
                    PassphraseSection(sync: sync)
                }
                PrivacySection(sync: sync)
                AccountActionsSection(sync: sync)
            } else {
                SignedOutSection(sync: sync)
            }
        }
        .formStyle(.grouped)
        .paneFrame()
        .task { await sync.reload() }
        .sheet(item: Binding(get: { sync.consentRequest }, set: { sync.consentRequest = $0 })) { kind in
            ConsentSheet(sync: sync, kind: kind)
        }
    }
}

private struct SignedOutSection: View {
    let sync: AccountSync

    var body: some View {
        Section {
            if sync.signedOutFromWeb {
                Label("Signed out of HaryX from the web", systemImage: "person.crop.circle.badge.xmark")
                    .foregroundStyle(Brand.warn)
                Text("Sign in again to keep syncing. Everything on this Mac keeps working.")
                    .foregroundStyle(.secondary)
            }
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
            Link("Open HaryX on the Web", destination: HaryXLinks.web)
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

/// Asks for the Sync consent before the passphrase and the first sync.
private struct SyncConsentSection: View {
    let sync: AccountSync

    var body: some View {
        Section {
            if sync.problem == .consentRequired, sync.syncConsent != false {
                Label(SyncProblem.consentRequired.summary, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Brand.warn)
            }
            Text("Sync keeps settings, app preferences and account setup the same on your Macs and PCs. HaryX asks for your consent first.")
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Turn On Sync…") { sync.consentRequest = .sync }
                    .buttonStyle(.glassProminent)
            }
        } header: { Text("Sync") }
    }
}

/// Showing this Mac on HaryX web, the consents and their withdrawal, and the data export.
private struct PrivacySection: View {
    let sync: AccountSync
    @State private var withdrawing: ConsentKind?
    @State private var message: String?
    @State private var error: String?

    var body: some View {
        Section {
            Toggle("Show this Mac on HaryX web", isOn: Binding(
                get: { sync.machinesConsent == true },
                set: { $0 ? (sync.consentRequest = .machines) : (withdrawing = .machines) }
            ))
            .disabled(sync.machinesConsent == nil || sync.working)
            if sync.syncConsent == true {
                HStack {
                    Text("Sync consent given")
                    Spacer()
                    Button("Withdraw…") { withdrawing = .sync }.disabled(sync.working)
                }
            }
            HStack {
                Text("A copy of everything HaryX keeps about your account")
                Spacer()
                Button("Export…", action: export).disabled(sync.working)
            }
            if let message { Text(message).foregroundStyle(.secondary) }
            if let error { Text(error).foregroundStyle(Brand.limit) }
        } header: { Text("Privacy") } footer: {
            VStack(alignment: .leading, spacing: 4) {
                if sync.machinesConsent == nil {
                    Text("HaryX could not read your consents. Check your connection.")
                }
                Link("Privacy policy, version \(privacyPolicyVersion)", destination: HaryXLinks.privacyPolicy)
            }
        }
        .alert(withdrawing?.withdrawTitle ?? "", isPresented: Binding(get: { withdrawing != nil }, set: { if !$0 { withdrawing = nil } }),
               presenting: withdrawing) { kind in
            Button(kind.withdrawButton, role: .destructive) {
                Task {
                    error = await sync.setConsent(kind, granted: false)
                    message = error == nil ? "Withdrawn. Its data is gone from HaryX's servers." : nil
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: { kind in
            Text(kind.withdrawMessage)
        }
    }
}

extension PrivacySection {
    /// Asks where to save, then `haryx account export PATH` writes the file there.
    fileprivate func export() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.json]
        panel.nameFieldStringValue = "haryx-export-\(Date().formatted(.iso8601.year().month().day())).json"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task {
            error = await sync.export(to: url)
            message = error == nil ? "Saved to \(url.lastPathComponent). Synced data in it stays encrypted: only your sync passphrase opens it." : nil
        }
    }
}

/// The consent sheet: what the user agrees to, the policy version and a link to it.
private struct ConsentSheet: View {
    let sync: AccountSync
    let kind: ConsentKind
    @Environment(\.dismiss) private var dismiss
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(kind.title(device: "Mac")).font(.title3.weight(.semibold))
            ForEach(kind.points(device: "Mac"), id: \.self) { point in
                Label { Text(point).fixedSize(horizontal: false, vertical: true) } icon: { Image(systemName: "circle.fill").font(.system(size: 5)) }
            }
            Link("Privacy policy, version \(privacyPolicyVersion)", destination: HaryXLinks.privacyPolicy)
            if let error { Text(error).foregroundStyle(Brand.limit) }
            HStack {
                Spacer()
                if sync.working { ProgressView().controlSize(.small) }
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(kind.agree) {
                    Task {
                        error = await sync.setConsent(kind, granted: true)
                        if error == nil { dismiss() }
                    }
                }
                .buttonStyle(.glassProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(sync.working)
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}

private struct AccountActionsSection: View {
    let sync: AccountSync
    @Environment(\.openURL) private var openURL
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
            Button("Open WeCoAp Account") { openURL(HaryXLinks.wecoapAccount) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Your HaryX account is your WeCoAp ID, and it is deleted on the WeCoAp account page. To erase HaryX's data and keep your WeCoAp ID, withdraw both consents instead. Your Claude accounts on this Mac are not touched.")
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
