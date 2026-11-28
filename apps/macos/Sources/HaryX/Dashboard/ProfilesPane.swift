import AppKit
import HaryXCore
import SwiftUI
import UniformTypeIdentifiers

/// State of the Setup Profiles section: saved profiles and the inventory of the one selected.
@MainActor @Observable
final class ProfilesModel {
    var profiles: [ProfileSummary] = []
    var selected = ProfileJSON.defaultName
    var inventory: ProfileInventory?
    var loading = false
    var error: String?

    var selectedSummary: ProfileSummary? { profiles.first { $0.name == selected } }

    func reload(_ cli: CSwapCLI?) async {
        guard let cli else { return }
        loading = true
        defer { loading = false }
        do {
            profiles = try await cli.json(ProfileList.self, ProfileArgs.list(), decoder: ProfileJSON.decoder).profiles
            if selected != ProfileJSON.defaultName, !profiles.contains(where: { $0.name == selected }) {
                selected = ProfileJSON.defaultName
            }
            inventory = try await cli.json(ProfileInventory.self, ProfileArgs.show(selected), decoder: ProfileJSON.decoder)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

struct ProfilesPane: View {
    @Environment(AppStore.self) private var store
    @State private var model = ProfilesModel()
    @State private var sheet: ProfileSheet?
    @State private var confirmDelete = false
    @State private var confirmRollback = false
    @State private var toggled: Set<String> = []

    var body: some View {
        Group {
            if store.cli?.flavor == .cswap {
                ContentUnavailableView("Profiles need haryx", systemImage: "shippingbox",
                                       description: Text("Setup profiles are a haryx feature; cswap does not have them."))
            } else {
                content
            }
        }
        .task { await model.reload(store.cli) }
        .onChange(of: model.selected) { Task { await model.reload(store.cli) } }
        .sheet(item: $sheet) { kind in
            ProfileSheetView(kind: kind, model: model) { sheet = nil }
        }
        .confirmationDialog("Delete profile '\(model.selected)'?", isPresented: $confirmDelete) {
            Button("Delete", role: .destructive) { Task { await delete() } }
        } message: { Text("The saved copy is removed. Your live ~/.claude is not touched.") }
        .confirmationDialog("Undo the last apply?", isPresented: $confirmRollback) {
            Button("Restore Backup") { Task { await rollback() } }
        } message: { Text("Files and settings changed by the last apply are restored. Installed plugins and imported accounts stay.") }
    }

    private var content: some View {
        Form {
            if model.profiles.isEmpty, !model.loading, model.error == nil { intro }
            header
            inventorySections
        }
        .formStyle(.grouped)
        .paneFrame()
        .toolbar {
            ToolbarItemGroup {
                Button("Save Snapshot…") { sheet = .save }
                    .help("Save the live ~/.claude as a named profile")
                Button("Export…") { sheet = .export }
                    .help("Write a .haryx-profile file for another machine")
                Button("Import…") { importFile() }
                    .help("Store a .haryx-profile file from another machine")
                Button { Task { await model.reload(store.cli) } } label: {
                    Label("Refresh", systemImage: "arrow.clockwise").spinning(model.loading)
                }
                .help("Read the profiles again")
                .disabled(model.loading)
            }
        }
    }

    /// Shown until the first profile is saved or imported: what a profile is, and the two ways to get one.
    private var intro: some View {
        Section {
            VStack(alignment: .leading, spacing: 10) {
                Text("A profile is a saved copy of your Claude Code setup: settings, hooks, MCP servers and plugins. Save one to restore it later, or export it to set up another machine.")
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button("Save Snapshot…") { sheet = .save }.buttonStyle(.glassProminent)
                    Button("Import…") { importFile() }.buttonStyle(.glass)
                }
            }
            .padding(.vertical, 4)
        }
    }

    private var header: some View {
        Section {
            Picker("Profile", selection: $model.selected) {
                Text("default (live ~/.claude)").tag(ProfileJSON.defaultName)
                if !model.profiles.isEmpty { Divider() }
                ForEach(model.profiles) { p in
                    Text(p.lastApplied ? "\(p.name) (last applied)" : p.name).tag(p.name)
                }
            }
            if let s = model.selectedSummary {
                LabeledContent("Contains", value: "\(profileByteString(s.totalBytes)) · from \(s.sourceOs) · secrets \(s.secrets)\(s.hasCredentials ? " · credentials" : "")")
            }
            if model.selected != ProfileJSON.defaultName || hasAppliedProfile {
                HStack {
                    if model.selected != ProfileJSON.defaultName {
                        Button("Apply…") { sheet = .apply(model.selected) }.buttonStyle(.glassProminent)
                        Button("Diff") { sheet = .diff(model.selected) }
                            .buttonStyle(.glass)
                            .help("Compare this profile with the live ~/.claude")
                    }
                    Spacer()
                    if hasAppliedProfile {
                        Button("Undo Last Apply…") { confirmRollback = true }.buttonStyle(.glass)
                    }
                    if model.selected != ProfileJSON.defaultName {
                        Button("Delete…", role: .destructive) { confirmDelete = true }
                            .buttonStyle(.glass).tint(.red)
                            .help("Delete this saved profile")
                    }
                }
            }
        }
    }

    /// The last apply left a backup to restore: some profile is marked last applied.
    private var hasAppliedProfile: Bool { model.profiles.contains(where: \.lastApplied) }

    @ViewBuilder private var inventorySections: some View {
        if let error = model.error {
            Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.red) }
        } else if let inventory = model.inventory {
            ForEach(inventory.grouped, id: \.category) { group in
                Section {
                    Fold(isExpanded: expansion(group.category, startsOpen: group.items.contains(where: \.isProblem))) {
                        LabeledContent(categoryTitle(group.category), value: ProfileInventory.summary(group.items))
                    } content: {
                        ForEach(group.items) { ProfileItemRow(item: $0) }
                    }
                }
            }
        } else {
            Section { ProgressView().frame(maxWidth: .infinity) }
        }
    }

    private func categoryTitle(_ category: String) -> String {
        let optional = ProfileCategory.all.first { $0.name == category }?.defaultOn == false
        return ProfileCategory.title(of: category) + (optional ? " (opt-in)" : "")
    }

    /// Categories start closed, except one holding an item that needs attention; `toggled` holds the
    /// categories the user has flipped from that starting state.
    private func expansion(_ category: String, startsOpen: Bool) -> Binding<Bool> {
        Binding(
            get: { startsOpen != toggled.contains(category) },
            set: { open in toggled = open == startsOpen ? toggled.subtracting([category]) : toggled.union([category]) }
        )
    }

    private func importFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [UTType(filenameExtension: "haryx-profile") ?? .data]
        panel.allowsOtherFileTypes = true
        NSApp.activate()
        guard panel.runModal() == .OK, let url = panel.url else { return }
        sheet = .importFile(url)
    }

    private func delete() async {
        guard let cli = store.cli else { return }
        do {
            try await cli.checked(ProfileArgs.delete(model.selected))
            store.flash("Deleted profile '\(model.selected)'")
            model.selected = ProfileJSON.defaultName
            await model.reload(cli)
        } catch { store.flash(error.localizedDescription, error: true) }
    }

    private func rollback() async {
        guard let cli = store.cli else { return }
        do {
            let out = try await cli.checked(ProfileArgs.rollback())
            store.flash(AppStore.lastLine(out) ?? "Restored the last backup")
            await model.reload(cli)
        } catch { store.flash(error.localizedDescription, error: true) }
    }
}

struct ProfileItemRow: View {
    let item: ProfileItem

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Image(systemName: icon).foregroundStyle(.secondary).frame(width: 18)
                Text(item.id).font(.body.monospaced()).lineLimit(1).truncationMode(.middle)
                if item.isProblem { StatusBadge(status: item.status) }
                Spacer()
                if item.files > 0 {
                    Text("\(profileByteString(item.size)) · \(item.files) file\(item.files == 1 ? "" : "s")")
                        .font(.caption).foregroundStyle(.secondary).monospacedDigit()
                }
            }
            // A hook registered for several events reads the same each time: show it once, with a count.
            ForEach(ProfileInventory.counted(item.detail.map(Self.shortDetail)), id: \.line) { entry in
                Text(entry.count > 1 ? "\(entry.line) ×\(entry.count)" : entry.line)
                    .font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                    .truncationMode(.middle)
                    .padding(.leading, 26)
                    .help(fullDetail(entry.line))
            }
        }
    }

    /// A hook line names the script it runs, with the home folder as ~; the full command is in the
    /// hover help. Other detail lines only get the ~.
    static func shortDetail(_ line: String) -> String {
        let prefix = "hook: "
        guard line.hasPrefix(prefix) else { return HookSummary.tilde(line) }
        return prefix + HookSummary.short(String(line.dropFirst(prefix.count)))
    }

    /// The first full line behind a shortened one, for the hover help.
    private func fullDetail(_ short: String) -> String {
        HookSummary.tilde(item.detail.first { Self.shortDetail($0) == short } ?? short)
    }

    private var icon: String {
        switch item.kind {
        case "folder": "folder"
        case "mcp": "server.rack"
        case "plugin": "puzzlepiece.extension"
        case "key": "key.horizontal"
        case "accounts": "person.2"
        default: "doc"
        }
    }
}

struct StatusBadge: View {
    let status: String

    var body: some View {
        Text(status)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 6).padding(.vertical, 1)
            .background(color.opacity(0.18), in: Capsule())
            .foregroundStyle(color)
    }

    private var color: Color { status == "broken" ? .red : .orange }
}
