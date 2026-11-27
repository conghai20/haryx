import HaryXCore
import SwiftUI
import UniformTypeIdentifiers

struct MappingsPane: View {
    @Environment(AppStore.self) private var store
    @State private var newSlot: Int?

    var body: some View {
        Form {
            Section {
                ForEach(store.mappings) { m in
                    HStack(spacing: 10) {
                        if let a = store.account(for: m) {
                            MascotView(style: store.style(for: a), mood: store.mood(for: a), size: 28, phase: store.phase(for: a))
                        }
                        VStack(alignment: .leading) {
                            Text((m.path as NSString).abbreviatingWithTildeInPath).font(.body.monospaced())
                            Text(store.account(for: m).map { "Account \($0.number): \($0.displayName)" } ?? "\(m.email) (not managed)")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Menu {
                            TerminalMenuItems { store.runMapped(m, app: $0) }
                        } label: { Label("Run Claude", systemImage: "terminal") } primaryAction: { store.runMapped(m) }
                            .menuStyle(.button).buttonStyle(.glass).fixedSize()
                        Button(role: .destructive) { Task { await store.unmap(m.path) } } label: { Image(systemName: "trash") }
                            .buttonStyle(.borderless).help("Unmap")
                    }
                }
                if store.mappings.isEmpty {
                    Text("No folders mapped. Mapped folders run Claude as their account with plain `haryx ai run`.")
                        .foregroundStyle(.secondary)
                }
            }
            Section("Map a folder") {
                Picker("Account", selection: $newSlot) {
                    Text("Choose…").tag(Int?.none)
                    ForEach(store.accounts) { Text("\($0.number): \($0.displayName)").tag(Int?.some($0.number)) }
                }
                Button("Choose Folder and Map…") {
                    guard let a = newSlot.flatMap(store.account(slot:)), let dir = AppStore.chooseFolder(prompt: "Map") else { return }
                    Task { await store.map(a, path: dir) }
                }
                .disabled(newSlot == nil)
            }
        }
        .formStyle(.grouped)
        .paneFrame()
        .onAppear { store.loadMappings() }
    }
}

struct MaintenancePane: View {
    @Environment(AppStore.self) private var store
    @Bindable private var router = DashboardRouter.shared
    @State private var exportSlots: Set<Int> = []
    @State private var showExportAccounts = false
    @State private var exportFull = false
    @State private var importForce = false
    @State private var holdSeconds = 0
    @State private var confirmPurge = false
    @State private var purgeText = ""
    @State private var purgeEntry: UnclaimedEntry?

    var body: some View {
        Form {
            Section {
                // Folded to one line: twenty accounts would push Export a screen down.
                Fold(isExpanded: $showExportAccounts) {
                    LabeledContent("Accounts", value: exportSummary)
                } content: {
                    ForEach(store.accounts) { a in
                        Toggle(isOn: Binding(
                            get: { exportSlots.contains(a.number) },
                            set: { on in exportSlots = on ? exportSlots.union([a.number]) : exportSlots.subtracting([a.number]) }
                        )) { Text("\(a.number): \(a.displayName)").lineLimit(1).truncationMode(.middle) }
                        .toggleStyle(.checkbox)
                    }
                    HStack {
                        Button("Select All") { exportSlots = Set(store.accounts.map(\.number)) }
                        Button("Select None") { exportSlots = [] }
                    }
                }
                Toggle("Full export (whole ~/.claude.json + credentials, same-machine backup)", isOn: $exportFull)
                Button("Export…") {
                    guard let url = savePanel() else { return }
                    let chosen = chosenForExport
                    // Every account ticked: export all, so accounts added later are not left out.
                    let accounts = chosen.count == store.accounts.count ? [] : chosen
                    Task { await store.export(to: url, accounts: accounts, full: exportFull) }
                }
                .disabled(chosenForExport.isEmpty)
            } header: { Text("Export") } footer: {
                Label("Exports are plain JSON that includes login tokens. The file is saved owner-only (600); keep it somewhere safe or encrypt it.",
                      systemImage: "lock.trianglebadge.exclamationmark")
            }

            Section("Import accounts") {
                Toggle("Replace accounts that already exist", isOn: $importForce)
                    .help("Imports over an account with the same email instead of skipping it (--force).")
                Button("Import Accounts…") {
                    guard let url = openPanel() else { return }
                    Task { await store.importAccounts(from: url, force: importForce) }
                }
                // `haryx import --from-cswap` reads cswap's store; cswap itself has no such command.
                if store.cli?.flavor == .haryx {
                    Button("Import from cswap") { Task { await store.importFromCswap() } }
                        .disabled(store.busy != nil)
                        .help("Copy the accounts stored by cswap into haryx")
                }
            }

            Section {
                Picker("Use imported readings for", selection: $holdSeconds) {
                    ForEach(Self.holdChoices, id: \.seconds) { Text($0.title).tag($0.seconds) }
                }
                .help("Imported readings stand in for this long, so this Mac doesn't also poll the usage endpoint (--hold).")
                Button("Import Usage from Another Machine…") {
                    guard let url = openPanel() else { return }
                    Task { await store.importUsage(from: url, hold: holdSeconds > 0 ? holdSeconds : nil) }
                }
            } header: { Text("Import usage") } footer: {
                Text("Feed `haryx ai list --json` output from another machine so both don't poll the usage endpoint.")
            }

            Section {
                if store.unclaimed.isEmpty { Text("None stashed").foregroundStyle(.secondary) }
                ForEach(store.unclaimed) { u in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(u.id).font(.caption.monospaced())
                            Text("Account \(u.slot) · \(u.reason)").font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Purge", role: .destructive) { purgeEntry = u }.tint(.red)
                    }
                }
            } header: { Text("Unclaimed credentials") } footer: {
                Text("Safety copies of logins kept during a switch. HaryX never uses them on its own; purge one once its account signs in normally.")
            }

            Section {
                Button("Purge All Account Data…", role: .destructive) { confirmPurge = true }.tint(.red)
            } header: {
                Text("Erase")
            } footer: {
                Text("Removes every stored account, backup, and setting. Your current Claude Code login stays as it is.")
            }
        }
        .formStyle(.grouped)
        .paneFrame()
        .task { await store.loadExtras() }
        .onAppear(perform: takeExportSelection)
        .onChange(of: router.exportSelection) { takeExportSelection() }
        .confirmationDialog("Delete the stashed credential \(purgeEntry?.id ?? "")?", isPresented: Binding(
            get: { purgeEntry != nil }, set: { if !$0 { purgeEntry = nil } }
        ), presenting: purgeEntry) { u in
            Button("Purge", role: .destructive) { Task { await store.purgeUnclaimed(u.id) } }
        } message: { u in
            Text("It belongs to account \(u.slot). Recovery is signing in again.")
        }
        .sheet(isPresented: $confirmPurge) {
            VStack(alignment: .leading, spacing: 12) {
                Text("Purge all account data?").font(.title3.bold())
                Text("This can't be undone. Type PURGE to confirm.").foregroundStyle(.secondary)
                TextField("PURGE", text: $purgeText)
                HStack {
                    Spacer()
                    Button("Cancel") { confirmPurge = false; purgeText = "" }
                        .keyboardShortcut(.cancelAction)
                    Button("Purge", role: .destructive) {
                        confirmPurge = false
                        purgeText = ""
                        Task { await store.purgeAll() }
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(purgeText != "PURGE")
                }
            }
            .padding(20).frame(width: 380)
        }
    }

    /// How long imported usage readings stand in before this Mac reads usage itself (`--hold`).
    static let holdChoices: [(seconds: Int, title: String)] = [
        (0, "Don't (read usage again now)"), (300, "5 minutes"), (900, "15 minutes"), (3600, "1 hour"), (86_400, "24 hours"),
    ]

    /// The folded account list's one line: all of them, some, or none.
    private var exportSummary: String {
        let total = store.accounts.count
        return chosenForExport.count == total ? "All \(total)" : "\(chosenForExport.count) of \(total)"
    }

    /// Ticked accounts that still exist, in slot order.
    private var chosenForExport: [Account] { store.accounts.filter { exportSlots.contains($0.number) } }

    /// Picks up a card's "Export…" (that account ticked); otherwise start with all ticked.
    private func takeExportSelection() {
        if let slots = router.exportSelection {
            exportSlots = slots
            showExportAccounts = true // a card's Export… shows what it ticked
            router.exportSelection = nil
        } else if exportSlots.isEmpty {
            exportSlots = Set(store.accounts.map(\.number))
        }
    }

    private func savePanel() -> URL? {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.json]
        panel.nameFieldStringValue = "haryx-export-\(Date().formatted(.iso8601.year().month().day())).json"
        return panel.runModal() == .OK ? panel.url : nil
    }

    private func openPanel() -> URL? {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        return panel.runModal() == .OK ? panel.url : nil
    }
}
