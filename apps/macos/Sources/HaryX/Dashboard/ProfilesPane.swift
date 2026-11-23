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
        VStack(spacing: 0) {
            header.padding(16)
            Divider()
            inventoryList
        }
        .toolbar {
            ToolbarItemGroup {
                Button { sheet = .save } label: { Label("Save Snapshot", systemImage: "square.and.arrow.down.on.square") }
                    .help("Save the live ~/.claude as a named profile")
                Button { sheet = .export } label: { Label("Export", systemImage: "square.and.arrow.up") }
                    .help("Write a .haryx-profile file for another machine")
                Button { importFile() } label: { Label("Import", systemImage: "square.and.arrow.down") }
                    .help("Store a .haryx-profile file from another machine")
                Button { Task { await model.reload(store.cli) } } label: { Label("Refresh", systemImage: "arrow.clockwise") }
            }
        }
    }

    private var header: some View {
        HStack(alignment: .center, spacing: 12) {
            Picker("Profile", selection: $model.selected) {
                Text("default (live ~/.claude)").tag(ProfileJSON.defaultName)
                if !model.profiles.isEmpty { Divider() }
                ForEach(model.profiles) { p in
                    Text(p.lastApplied ? "\(p.name) (last applied)" : p.name).tag(p.name)
                }
            }
            .frame(maxWidth: 320)
            if let s = model.selectedSummary {
                Text("\(profileByteString(s.totalBytes)) · from \(s.sourceOs) · secrets \(s.secrets)\(s.hasCredentials ? " · credentials" : "")")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            if model.loading { ProgressView().controlSize(.small) }
            if model.selected != ProfileJSON.defaultName {
                Button("Apply…") { sheet = .apply(model.selected) }.buttonStyle(.glassProminent)
                Button(role: .destructive) { confirmDelete = true } label: { Image(systemName: "trash") }
                    .help("Delete this saved profile")
            }
            Button { confirmRollback = true } label: { Image(systemName: "arrow.uturn.backward") }
                .help("Undo the last apply")
        }
    }

    @ViewBuilder private var inventoryList: some View {
        if let error = model.error {
            ContentUnavailableView("Could not read the profile", systemImage: "exclamationmark.triangle", description: Text(error))
        } else if let inventory = model.inventory {
            List {
                ForEach(inventory.grouped, id: \.category) { group in
                    Section {
                        ForEach(group.items) { ProfileItemRow(item: $0) }
                    } header: {
                        let optional = ProfileCategory.all.first { $0.name == group.category }?.defaultOn == false
                        Text(ProfileCategory.title(of: group.category) + (optional ? " (opt-in)" : ""))
                    }
                }
            }
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
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
            ForEach(item.detail, id: \.self) { line in
                Text(line).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(2)
                    .padding(.leading, 26)
            }
        }
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
