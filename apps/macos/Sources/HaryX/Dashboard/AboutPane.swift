import HaryXCore
import SwiftUI

/// HaryX › About & Updates: the app and CLI versions, how updates are installed, and the latest
/// release's notes.
struct AboutPane: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let updates = store.updates
        Form {
            Section {
                HStack(spacing: 14) {
                    Image(nsImage: NSApp.applicationIconImage).resizable().frame(width: 56, height: 56)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("HaryX").font(.title2.weight(.semibold))
                        Text("Version \(updates.appVersion)").foregroundStyle(.secondary)
                    }
                }
                LabeledContent("Command-line backend", value: store.version.isEmpty ? "not found" : store.version)
                if let path = store.cli?.executable.path {
                    LabeledContent("Using", value: (path as NSString).abbreviatingWithTildeInPath)
                }
                HStack {
                    Link("GitHub", destination: URL(string: "https://github.com/conghai20/haryx")!)
                    Link("All releases", destination: URL(string: "https://github.com/conghai20/haryx/releases")!)
                    Link("Licence (MIT)", destination: URL(string: "https://github.com/conghai20/haryx/blob/main/LICENSE")!)
                }
            }

            UpdateSection(updates: updates)

            if let check = updates.check, let notes = check.notes, !notes.isEmpty {
                Section {
                    ReleaseNotes(markdown: notes)
                } header: {
                    Text(check.isNewer(than: updates.appVersion)
                         ? "What's new in \(check.latest ?? "")" : "Release notes for \(check.latest ?? updates.appVersion)")
                } footer: {
                    if let url = check.htmlUrl { Link("Open on GitHub", destination: url) }
                }
            }
        }
        .formStyle(.grouped)
        .paneFrame()
        .task { if updates.check == nil { await updates.checkNow(refresh: false) } }
    }
}

private struct UpdateSection: View {
    let updates: UpdateController

    var body: some View {
        Section {
            Picker("Updates", selection: Binding(get: { updates.mode }, set: { updates.mode = $0 })) {
                ForEach(UpdateMode.allCases) { Text($0.title).tag($0) }
            }
            status
            HStack {
                Button("Check Now") { Task { await updates.checkNow() } }
                    .disabled(updates.phase == .checking || updates.phase == .downloading)
                if let last = updates.lastChecked {
                    Text("Last checked \(last.formatted(.relative(presentation: .named)))")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        } header: { Text("Updates") } footer: {
            VStack(alignment: .leading, spacing: 4) {
                Text("Updates come from GitHub releases. Each download is checked against its SHA-256 and the app's signature before it replaces HaryX; the bundled haryx command updates with the app.")
                if updates.isAdHocSigned {
                    Text("This build is not signed with a Developer ID, so after an update macOS may ask again for permissions you gave HaryX.")
                }
                if !updates.isAppBundle {
                    Text("This is a development build (`swift run`); only HaryX.app can update itself.")
                }
            }
        }
    }

    @ViewBuilder private var status: some View {
        switch updates.phase {
        case .checking:
            HStack { ProgressView().controlSize(.small); Text("Checking…") }
        case .downloading:
            HStack { ProgressView().controlSize(.small); Text("Downloading \(updates.available?.latest ?? "")…") }
        case .ready(let version):
            HStack {
                Label("HaryX \(version) is ready", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                Spacer()
                Button("Restart Now") { updates.restartNow() }.buttonStyle(.glassProminent)
            }
            Text("It installs when HaryX quits, or now with Restart Now.").font(.caption).foregroundStyle(.secondary)
        case .failed(let message):
            Label(message, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange)
        case .idle:
            if let release = updates.available {
                HStack {
                    Label("HaryX \(release.latest ?? "") is available", systemImage: "arrow.down.circle.fill")
                        .foregroundStyle(.tint)
                    Spacer()
                    Button("Skip This Version") { updates.skip() }
                    Button("Update") { Task { await updates.download() } }
                        .buttonStyle(.glassProminent)
                        .disabled(!updates.isAppBundle)
                }
            } else if let check = updates.check, check.latest != nil {
                if let skipped = updates.skippedVersion, skipped == check.latest,
                   check.isNewer(than: updates.appVersion) {
                    HStack {
                        Text("You skipped \(skipped).").foregroundStyle(.secondary)
                        Button("Show It Again") { updates.clearSkip() }
                    }
                } else {
                    Label("HaryX is up to date", systemImage: "checkmark.circle").foregroundStyle(.secondary)
                }
            }
        }
    }
}

/// The release notes Markdown (from CHANGELOG.md): `###` headings, `-` bullets with wrapped
/// continuation lines, and inline Markdown inside them.
struct ReleaseNotes: View {
    let markdown: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(Self.blocks(markdown).enumerated()), id: \.offset) { _, block in
                switch block {
                case .heading(let text):
                    Text(text).font(.headline).padding(.top, 4)
                case .bullet(let text):
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text("•").foregroundStyle(.secondary)
                        Text(Self.inline(text)).fixedSize(horizontal: false, vertical: true)
                    }
                case .paragraph(let text):
                    Text(Self.inline(text)).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .textSelection(.enabled)
    }

    enum Block: Equatable {
        case heading(String), bullet(String), paragraph(String)
    }

    static func blocks(_ markdown: String) -> [Block] {
        var out: [Block] = []
        for raw in markdown.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.isEmpty { continue }
            if line.hasPrefix("#") {
                out.append(.heading(line.drop(while: { $0 == "#" }).trimmingCharacters(in: .whitespaces)))
            } else if line.hasPrefix("- ") || line.hasPrefix("* ") {
                out.append(.bullet(String(line.dropFirst(2))))
            } else if raw.hasPrefix(" "), case .bullet(let text) = out.last {
                out[out.count - 1] = .bullet(text + " " + line)
            } else if case .paragraph(let text) = out.last {
                out[out.count - 1] = .paragraph(text + " " + line)
            } else {
                out.append(.paragraph(line))
            }
        }
        return out
    }

    static func inline(_ text: String) -> AttributedString {
        (try? AttributedString(markdown: text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(text)
    }
}
