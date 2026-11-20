import CSwapCore
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
                    Text("No folders mapped. Mapped folders run Claude as their account with plain `cswap run`.")
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
        .onAppear { store.loadMappings() }
    }
}

struct MaintenancePane: View {
    @Environment(AppStore.self) private var store
    @State private var exportSlot: Int?
    @State private var exportFull = false
    @State private var importForce = false
    @State private var holdSeconds = 0
    @State private var confirmPurge = false
    @State private var purgeText = ""

    var body: some View {
        Form {
            Section {
                Picker("Accounts", selection: $exportSlot) {
                    Text("All accounts").tag(Int?.none)
                    ForEach(store.accounts) { Text("\($0.number): \($0.displayName)").tag(Int?.some($0.number)) }
                }
                Toggle("Full export (whole ~/.claude.json + credentials, same-machine backup)", isOn: $exportFull)
                Button("Export…") {
                    guard let url = savePanel() else { return }
                    Task { await store.export(to: url, account: exportSlot.flatMap(store.account(slot:)), full: exportFull) }
                }
            } header: { Text("Export") } footer: {
                Label("Exports are plain JSON that includes login tokens. The file is saved owner-only (600); keep it somewhere safe or encrypt it.",
                      systemImage: "lock.trianglebadge.exclamationmark")
            }

            Section("Import") {
                Toggle("Overwrite existing accounts (--force)", isOn: $importForce)
                Button("Import Accounts…") {
                    guard let url = openPanel() else { return }
                    Task { await store.importAccounts(from: url, force: importForce) }
                }
                Stepper("Hold imported usage for \(holdSeconds) s", value: $holdSeconds, in: 0...86_400, step: 300)
                Button("Import Usage from Another Machine…") {
                    guard let url = openPanel() else { return }
                    Task { await store.importUsage(from: url, hold: holdSeconds > 0 ? holdSeconds : nil) }
                }
                .help("Feed `cswap list --json` output from another machine so both don't poll the usage endpoint")
            }

            Section("Unclaimed credentials") {
                if store.unclaimed.isEmpty { Text("None").foregroundStyle(.secondary) }
                ForEach(store.unclaimed) { u in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(u.id).font(.caption.monospaced())
                            Text("slot \(u.slot) · \(u.reason)").font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Purge", role: .destructive) { Task { await store.purgeUnclaimed(u.id) } }
                    }
                }
            }

            Section("cswap") {
                LabeledContent("Version", value: store.version.isEmpty ? "—" : store.version)
                LabeledContent("Executable", value: store.cli?.executable.path ?? "not found")
                HStack {
                    Button("Upgrade cswap") { Task { await store.upgrade() } }
                    Button("Open Data Folder") { NSWorkspace.shared.open(URL(fileURLWithPath: CSwapCLI.backupRoot)) }
                    Button("Show Log") {
                        NSWorkspace.shared.selectFile(CSwapCLI.backupRoot + "/claude-swap.log", inFileViewerRootedAtPath: "")
                    }
                }
            }

            Section {
                Button("Purge All claude-swap Data…", role: .destructive) { confirmPurge = true }
            } footer: {
                Text("Removes every stored account, backup, and setting. Your current Claude Code login stays as it is.")
            }
        }
        .formStyle(.grouped)
        .task { await store.loadExtras() }
        .sheet(isPresented: $confirmPurge) {
            VStack(alignment: .leading, spacing: 12) {
                Text("Purge all claude-swap data?").font(.title3.bold())
                Text("This can't be undone. Type PURGE to confirm.").foregroundStyle(.secondary)
                TextField("PURGE", text: $purgeText)
                HStack {
                    Spacer()
                    Button("Cancel") { confirmPurge = false; purgeText = "" }
                    Button("Purge", role: .destructive) {
                        confirmPurge = false
                        purgeText = ""
                        Task { await store.purgeAll() }
                    }
                    .disabled(purgeText != "PURGE")
                }
            }
            .padding(20).frame(width: 380)
        }
    }

    private func savePanel() -> URL? {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.json]
        panel.nameFieldStringValue = "cswap-export-\(Date().formatted(.iso8601.year().month().day())).json"
        return panel.runModal() == .OK ? panel.url : nil
    }

    private func openPanel() -> URL? {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        return panel.runModal() == .OK ? panel.url : nil
    }
}
