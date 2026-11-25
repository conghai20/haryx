import AppKit
import HaryXCore
import SwiftUI

/// The sheets of the Setup Profiles section.
enum ProfileSheet: Identifiable {
    case save, export, importFile(URL), apply(String), diff(String)

    var id: String {
        switch self {
        case .save: "save"
        case .export: "export"
        case .importFile(let url): "import:\(url.path)"
        case .apply(let name): "apply:\(name)"
        case .diff(let name): "diff:\(name)"
        }
    }
}

struct ProfileSheetView: View {
    let kind: ProfileSheet
    let model: ProfilesModel
    let close: () -> Void

    var body: some View {
        switch kind {
        case .save: SaveProfileSheet(model: model, close: close)
        case .export: ExportProfileSheet(model: model, close: close)
        case .importFile(let url): ImportProfileSheet(file: url, model: model, close: close)
        case .apply(let name): ApplyProfileSheet(name: name, model: model, close: close)
        case .diff(let name): DiffProfileSheet(name: name, close: close)
        }
    }
}

/// The parts toggles shared by Save and Export.
struct ProfilePartsSection: View {
    @Binding var options: ProfileExportOptions

    var body: some View {
        Section("Include") {
            Toggle("History (conversations, prompt history, checkpoints)", isOn: $options.includeHistory)
            Toggle("Plugin files (otherwise only names, reinstalled on apply)", isOn: $options.bundlePlugins)
            Toggle("Accounts and logins (needs encryption)", isOn: $options.credentials)
            Toggle("Whole ~/.claude.json (identity, folder trust)", isOn: $options.claudeJSON)
            Toggle("Anthropic-signed files (policy limits, remote settings)", isOn: $options.signed)
            Toggle("Caches", isOn: $options.cache)
            ForEach(options.includeDirs, id: \.self) { dir in
                HStack {
                    Text((dir as NSString).abbreviatingWithTildeInPath).font(.body.monospaced())
                    Spacer()
                    Button(role: .destructive) { options.includeDirs.removeAll { $0 == dir } } label: { Image(systemName: "minus.circle") }
                        .buttonStyle(.borderless)
                }
            }
            Button("Add Folder…") {
                if let dir = AppStore.chooseFolder(prompt: "Include") { options.includeDirs.append(dir) }
            }
        }
    }
}

struct SaveProfileSheet: View {
    @Environment(AppStore.self) private var store
    let model: ProfilesModel
    let close: () -> Void
    @State private var name = ""
    @State private var options = ProfileExportOptions()
    @State private var force = false
    @State private var working = false

    var body: some View {
        SheetFrame(title: "Save Snapshot", working: working, primary: "Save",
                   canSubmit: ProfileArgs.isValidName(name), close: close, submit: save) {
            Form {
                TextField("Name", text: $name, prompt: Text("work"))
                Toggle("Replace a profile with this name", isOn: $force)
                ProfilePartsSection(options: $options)
            }
            .formStyle(.grouped)
        }
    }

    private func save() async {
        guard let cli = store.cli else { return }
        working = true
        defer { working = false }
        do {
            try await cli.checked(ProfileArgs.save(name, options, force: force))
            store.flash("Saved profile '\(name)'")
            model.selected = name
            await model.reload(cli)
            close()
        } catch { store.flash(error.localizedDescription, error: true) }
    }
}

struct ExportProfileSheet: View {
    @Environment(AppStore.self) private var store
    let model: ProfilesModel
    let close: () -> Void
    @State private var options = ProfileExportOptions()
    @State private var passphrase = ""
    @State private var again = ""
    @State private var working = false

    private var passOK: Bool { !options.encrypt || (passphrase.count >= 8 && passphrase == again) }

    var body: some View {
        SheetFrame(title: "Export '\(model.selected)'", working: working, primary: "Export…",
                   canSubmit: options.isValid && passOK, close: close, submit: export) {
            Form {
                Section {
                    Picker("Secrets", selection: $options.encrypt) {
                        Text("Redact (set them again on the other machine)").tag(false)
                        Text("Encrypt with a passphrase").tag(true)
                    }
                    if options.encrypt {
                        SecureField("Passphrase (8+ characters)", text: $passphrase)
                        SecureField("Again", text: $again)
                    }
                } footer: {
                    Text("API keys and tokens found in MCP servers, settings and history never leave in plain text.")
                }
                ProfilePartsSection(options: $options)
                if options.credentials {
                    Label("Logins rotate: using one login on two machines can sign the other out (fix with Sign In Again).",
                          systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange)
                }
            }
            .formStyle(.grouped)
        }
    }

    private func export() async {
        guard let cli = store.cli else { return }
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(model.selected)-\(Date().formatted(.iso8601.year().month().day())).haryx-profile"
        NSApp.activate()
        guard panel.runModal() == .OK, let url = panel.url else { return }
        working = true
        defer { working = false }
        do {
            let env = options.encrypt ? [ProfileArgs.passphraseEnv: passphrase] : [:]
            try await cli.checked(ProfileArgs.export(model.selected, to: url.path, options), env: env)
            store.flash("Exported to \(url.lastPathComponent)")
            close()
        } catch { store.flash(error.localizedDescription, error: true) }
    }
}

struct ImportProfileSheet: View {
    @Environment(AppStore.self) private var store
    let file: URL
    let model: ProfilesModel
    let close: () -> Void
    @State private var name = ""
    @State private var force = false
    @State private var working = false
    @State private var imported: String?

    var body: some View {
        if let imported {
            ApplyProfileSheet(name: imported, model: model, close: close)
        } else {
            SheetFrame(title: "Import \(file.lastPathComponent)", working: working, primary: "Import",
                       canSubmit: ProfileArgs.isValidName(name), close: close, submit: importIt) {
                Form {
                    TextField("Save as", text: $name, prompt: Text("work"))
                    Toggle("Replace a profile with this name", isOn: $force)
                    Text("Importing only stores the profile. You review what it changes before anything is applied.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                .formStyle(.grouped)
            }
            .onAppear { name = Self.suggestedName(file) }
        }
    }

    static func suggestedName(_ url: URL) -> String {
        let base = url.deletingPathExtension().lastPathComponent
        let cleaned = String(base.unicodeScalars.map { CharacterSet.alphanumerics.contains($0) && $0.isASCII ? Character($0) : "-" })
        return ProfileArgs.isValidName(cleaned) ? cleaned : "imported"
    }

    private func importIt() async {
        guard let cli = store.cli else { return }
        working = true
        defer { working = false }
        do {
            try await cli.checked(ProfileArgs.importFile(name, from: file.path, force: force))
            model.selected = name
            await model.reload(cli)
            imported = name
        } catch { store.flash(error.localizedDescription, error: true) }
    }
}

/// Shows the dry-run plan, the code it installs, and applies on confirmation.
struct ApplyProfileSheet: View {
    @Environment(AppStore.self) private var store
    let name: String
    let model: ProfilesModel
    let close: () -> Void
    @State private var plan: ProfilePlan?
    @State private var planError: String?
    @State private var prune = false
    @State private var includeHistory = true
    @State private var passphrase = ""
    @State private var working = false

    private var encrypted: Bool { model.profiles.first { $0.name == name }?.secrets == "encrypt" }
    private var canApply: Bool { (plan?.changes.isEmpty == false) && (!encrypted || !passphrase.isEmpty) }

    var body: some View {
        SheetFrame(title: "Apply '\(name)'", working: working, primary: "Apply", canSubmit: canApply,
                   close: close, submit: apply) {
            Form {
                Section {
                    Toggle("Include history", isOn: $includeHistory)
                    Toggle("Remove local items the profile does not have (history is always kept)", isOn: $prune)
                    if encrypted { SecureField("Passphrase of this profile", text: $passphrase) }
                }
                planSection
            }
            .formStyle(.grouped)
        }
        .task(id: "\(prune)-\(includeHistory)") { await loadPlan() }
    }

    @ViewBuilder private var planSection: some View {
        if let planError {
            Section { Label(planError, systemImage: "exclamationmark.triangle").foregroundStyle(.red) }
        } else if let plan {
            if !plan.runsCode.isEmpty {
                Section {
                    ForEach(plan.runsCode, id: \.self) { Text($0).font(.caption.monospaced()).lineLimit(3) }
                } header: {
                    Label("Installs code that runs on this Mac", systemImage: "exclamationmark.shield").foregroundStyle(.orange)
                }
            }
            if !plan.missingRequires.isEmpty {
                Section { Text("Not installed here: \(plan.missingRequires.joined(separator: ", "))").foregroundStyle(.orange) }
            }
            Section("\(plan.changes.count) change(s), \(plan.unchangedCount) unchanged") {
                ForEach(plan.changes) { e in
                    HStack {
                        Text(e.action).font(.caption.weight(.semibold)).frame(width: 54, alignment: .leading)
                            .foregroundStyle(e.action == "remove" ? .red : Brand.cyan)
                        Text(e.id).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                        Spacer()
                        if !e.needsSecret.isEmpty {
                            Text("needs \(e.needsSecret.joined(separator: ", "))").font(.caption2).foregroundStyle(.orange)
                        } else if let note = e.note {
                            Text(note).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                }
                if plan.changes.isEmpty { Text("Nothing to change.").foregroundStyle(.secondary) }
            }
            Section {
                Text("A backup is made first; undo with the ↩︎ button.").font(.caption).foregroundStyle(.secondary)
            }
        } else {
            Section { ProgressView() }
        }
    }

    private func loadPlan() async {
        guard let cli = store.cli else { return }
        do {
            plan = try await cli.json(ProfilePlan.self, ProfileArgs.plan(name, prune: prune, includeHistory: includeHistory),
                                      decoder: ProfileJSON.decoder)
            planError = nil
        } catch { planError = error.localizedDescription }
    }

    private func apply() async {
        guard let cli = store.cli else { return }
        working = true
        defer { working = false }
        do {
            let env = encrypted ? [ProfileArgs.passphraseEnv: passphrase] : [:]
            let out = try await cli.checked(ProfileArgs.apply(name, prune: prune, includeHistory: includeHistory), env: env)
            store.flash(out.split(separator: "\n").first.map(String.init) ?? "Applied '\(name)'")
            await model.reload(cli)
            await store.refresh()
            close()
        } catch { store.flash(error.localizedDescription, error: true) }
    }
}

/// `haryx ai profile diff`: what a saved profile has that the live ~/.claude does not, and back.
struct DiffProfileSheet: View {
    @Environment(AppStore.self) private var store
    let name: String
    let close: () -> Void
    @State private var diff: ProfileDiff?
    @State private var error: String?

    var body: some View {
        VStack(spacing: 0) {
            Text("Diff '\(name)'").font(.headline).padding(.top, 16)
            Form {
                if let error {
                    Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.red) }
                } else if let diff {
                    Section {
                        ForEach(diff.differences) { d in
                            HStack {
                                Text(d.stateText(left: name)).font(.caption.weight(.semibold))
                                    .foregroundStyle(d.state == "differs" ? Brand.cyan : .orange)
                                    .frame(width: 110, alignment: .leading)
                                Text(ProfileCategory.title(of: d.category)).font(.caption).foregroundStyle(.tertiary)
                                    .frame(width: 100, alignment: .leading)
                                Text(d.id).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                                Spacer()
                            }
                        }
                        if diff.differences.isEmpty { Text("No differences.").foregroundStyle(.secondary) }
                    } header: {
                        Text("'\(name)' compared with the live ~/.claude: \(diff.sameCount) the same.")
                    }
                } else {
                    Section { ProgressView("Comparing…") }
                }
            }
            .formStyle(.grouped)
            HStack {
                Spacer()
                Button("Close", action: close).keyboardShortcut(.defaultAction)
            }
            .padding(16)
        }
        .frame(width: 560, height: 560)
        .task { await load() }
    }

    private func load() async {
        guard let cli = store.cli else { return }
        do {
            diff = try await cli.json(ProfileDiff.self, ProfileArgs.diff(name), decoder: ProfileJSON.decoder)
            error = nil
        } catch { self.error = error.localizedDescription }
    }
}

/// Title, content, Cancel and a primary action that runs async.
struct SheetFrame<Content: View>: View {
    let title: String
    let working: Bool
    let primary: String
    let canSubmit: Bool
    let close: () -> Void
    let submit: () async -> Void
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(spacing: 0) {
            Text(title).font(.headline).padding(.top, 16)
            content()
            HStack {
                if working { ProgressView().controlSize(.small) }
                Spacer()
                Button("Cancel", action: close).keyboardShortcut(.cancelAction)
                Button(primary) { Task { await submit() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit || working)
            }
            .padding(16)
        }
        .frame(width: 560, height: 560)
    }
}
