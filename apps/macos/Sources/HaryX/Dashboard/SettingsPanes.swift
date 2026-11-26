import HaryXCore
import SwiftUI

/// Edits one `cswap config` key with a control matched to its value type.
struct ConfigRow: View {
    @Environment(AppStore.self) private var store
    let entry: ConfigEntry
    /// Told whenever the text field starts or stops holding an edit that is not saved yet.
    var onUnsavedChange: (Bool) -> Void = { _ in }
    @State private var text = ""

    static let choices: [String: [String]] = [
        "autoswitch.strategy": ["best", "weekly-first", "consume-first", "priority-consume-first"],
        "ui.theme": ["auto", "dark", "light"],
        "ui.orgTag": ["auto", "on", "off"],
    ]

    static let help: [String: String] = [
        "autoswitch.threshold": "Switch away when a window reaches this % (50–99.9).",
        "autoswitch.weeklyThreshold": "Separate line for weekly and per-model windows (blank = same as threshold).",
        "autoswitch.intervalSeconds": "Seconds between polls (15–3600).",
        "autoswitch.cooldownSeconds": "Minimum seconds between switches (0–86400).",
        "autoswitch.hysteresisPct": "Target must have this much more headroom (0–50).",
        "autoswitch.unhealthyTicks": "Failed polls before an account is quarantined.",
        "autoswitch.model": "Also count per-model weekly limits, e.g. Fable or Fable,Opus or all.",
        "autoswitch.includeApiKeyAccounts": "Let auto-switch pick API-key accounts.",
        "run.shareHistory": "Session-mode runs share chat history with your default profile.",
        "usage.headerProbe": "Read setup-token usage via a 1-token Haiku request (spends quota).",
        "usage.fableProbe": "Also probe Fable limits (spends quota).",
        "usage.shareBetweenMachines": "Share usage readings with your other machines, so they spend one usage budget per account (needs HaryX sync).",
        "updates.check": "Check for new releases of the command-line backend.",
    ]

    var body: some View {
        LabeledContent {
            HStack {
                editor
                if entry.isSet {
                    Button { Task { await store.configUnset(entry.key) } } label: { Image(systemName: "arrow.uturn.backward") }
                        .buttonStyle(.borderless).help("Reset to default")
                }
            }
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.key).font(.body.monospaced())
                if let help = Self.help[entry.key] { Text(help).font(.caption).foregroundStyle(.secondary) }
            }
        }
        .onAppear { text = entry.value.description }
        .onChange(of: entry) { text = entry.value.description }
        .onChange(of: hasUnsavedEdit) { _, unsaved in onUnsavedChange(unsaved) }
    }

    @ViewBuilder private var editor: some View {
        if let options = Self.choices[entry.key] {
            Picker("", selection: Binding(get: { entry.value.description }, set: { v in Task { await store.configSet(entry.key, v) } })) {
                ForEach(options, id: \.self) { Text($0).tag($0) }
            }
            .labelsHidden().fixedSize()
        } else if case .bool(let on) = entry.value {
            Toggle("", isOn: Binding(get: { on }, set: { v in Task { await store.configSet(entry.key, v ? "true" : "false") } }))
                .labelsHidden().toggleStyle(.switch)
        } else {
            TextField("", text: $text, prompt: Text("default"))
                .frame(width: 140)
                .multilineTextAlignment(.trailing)
                .onSubmit { Task { await commit() } }
                .onExitCommand { text = entry.value.description }
        }
    }

    /// The field differs from the stored value: an edit that `commit()` would send but hasn't yet.
    private var hasUnsavedEdit: Bool {
        let value = text.trimmingCharacters(in: .whitespaces)
        return value.isEmpty ? entry.isSet : value != entry.value.description
    }

    private func commit() async {
        let value = text.trimmingCharacters(in: .whitespaces)
        if value.isEmpty { await store.configUnset(entry.key) }
        else if value != entry.value.description { await store.configSet(entry.key, value) }
    }
}

/// A form section of config rows that records which keys hold an unsaved text edit in `unsaved`.
struct ConfigSection: View {
    let title: String
    let entries: [ConfigEntry]
    @Binding var unsaved: Set<String>

    var body: some View {
        Section(title) {
            ForEach(entries) { entry in
                ConfigRow(entry: entry) { isUnsaved in
                    unsaved = isUnsaved ? unsaved.union([entry.key]) : unsaved.subtracting([entry.key])
                }
            }
        }
    }
}

/// Bottom-right notice listing settings typed but not saved yet; hidden while there are none.
struct UnsavedConfigNotice: View {
    let keys: Set<String>

    var body: some View {
        if !keys.isEmpty {
            Label("Not saved: \(keys.sorted().joined(separator: ", ")). Press Return to save, Esc to discard.",
                  systemImage: "exclamationmark.triangle.fill")
                .font(.callout)
                .foregroundStyle(.orange)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .glassEffect(.regular, in: .capsule)
                .padding(16)
                .transition(.opacity)
        }
    }
}

struct ConfigPane: View {
    @Environment(AppStore.self) private var store
    @State private var unsaved: Set<String> = []

    var body: some View {
        Form {
            UsageDisplaySection()
            TerminalSettingsSection()
            DesktopSettingsSection()
            NotificationSettingsSection()
            let groups = Dictionary(grouping: store.config) { String($0.key.split(separator: ".").first ?? "") }
            ForEach(groups.keys.sorted(), id: \.self) { group in
                ConfigSection(title: group.capitalized, entries: groups[group] ?? [], unsaved: $unsaved)
            }
            Section {
                LabeledContent("File", value: store.configPath)
                Button("Reveal in Finder") { NSWorkspace.shared.selectFile(store.configPath, inFileViewerRootedAtPath: "") }
                    .disabled(store.configPath.isEmpty)
            } footer: {
                Text("Press Return to save a text field. Unknown keys and hand edits in settings.json are preserved.")
            }
        }
        .formStyle(.grouped)
        .overlay(alignment: .bottomTrailing) { UnsavedConfigNotice(keys: unsaved) }
        .animation(.default, value: unsaved)
    }
}

struct AutoSwitchPane: View {
    @Environment(AppStore.self) private var store
    @AppStorage(Prefs.autoDryRun) private var dryRun = false
    @AppStorage(Prefs.autoOnLaunch) private var onLaunch = false
    @State private var unsaved: Set<String> = []
    /// Hides the `poll` events (a check that changed nothing), as the Windows app does.
    @State private var hidePolls = true

    private var shownEvents: [AutoEvent] {
        hidePolls ? store.auto.events.filter { $0.kind != "poll" } : store.auto.events
    }

    var body: some View {
        HSplitView {
            Form {
                Section {
                    HStack(spacing: 14) {
                        Image(systemName: "arrow.triangle.2.circlepath")
                            .font(.largeTitle)
                            .foregroundStyle(store.auto.isRunning ? Brand.cyanText : .secondary)
                            .symbolEffect(.rotate, isActive: store.auto.isRunning)
                        VStack(alignment: .leading) {
                            Text(store.auto.isRunning ? "Auto-switch is running" : "Auto-switch is off").font(.headline)
                            if let poll = store.auto.lastPoll {
                                Text("Last poll \(poll.formatted(.relative(presentation: .named)))").font(.caption).foregroundStyle(.secondary)
                            }
                            if let exit = store.auto.lastExit { Text(exit).font(.caption).foregroundStyle(.red) }
                        }
                        Spacer()
                        Button(store.auto.isRunning ? "Stop" : "Start") {
                            store.toggleAuto()
                        }
                        .buttonStyle(.glassProminent)
                    }
                    Toggle("Dry run (log decisions, don't switch)", isOn: $dryRun)
                        .disabled(store.auto.isRunning)
                    Toggle("Always start when HaryX launches", isOn: $onLaunch)
                        .help("Auto-switch already comes back on after a quit if you left it on; this also starts it when you had turned it off.")
                    Button("Check Once Now") { Task { await store.runAutoOnce() } }
                } footer: {
                    Text("Runs `haryx auto --json` inside HaryX. When the active account reaches your threshold it moves to another of your own accounts, and every decision appears in the log. Plan limits are set by Anthropic; use only accounts you hold yourself.")
                }
                ConfigSection(title: "Rules", entries: store.config.filter { $0.key.hasPrefix("autoswitch.") },
                              unsaved: $unsaved)
            }
            .formStyle(.grouped)
            .frame(minWidth: 380)

            VStack(spacing: 0) {
                HStack {
                    Text("Log").font(.headline)
                    Spacer()
                    Toggle("Hide routine checks", isOn: $hidePolls).controlSize(.small)
                }
                .padding(.horizontal, 12).padding(.vertical, 8)
                Divider()
                List(shownEvents) { event in
                    HStack(alignment: .firstTextBaseline) {
                        Image(systemName: icon(event)).foregroundStyle(color(event))
                        VStack(alignment: .leading, spacing: 2) {
                            Text(event.summary).font(.callout)
                            Text(event.date.formatted(date: .omitted, time: .standard)).font(.caption2).foregroundStyle(.tertiary)
                        }
                    }
                }
                .overlay { if shownEvents.isEmpty { ContentUnavailableView("No events yet", systemImage: "list.bullet.rectangle") } }
            }
            .frame(minWidth: 300)
        }
        .overlay(alignment: .bottomTrailing) { UnsavedConfigNotice(keys: unsaved) }
        .animation(.default, value: unsaved)
    }

    private func icon(_ e: AutoEvent) -> String {
        switch e.severity {
        case .info: "circle.dotted"
        case .notable: "arrow.left.arrow.right.circle.fill"
        case .problem: "exclamationmark.triangle.fill"
        }
    }

    private func color(_ e: AutoEvent) -> Color {
        switch e.severity {
        case .info: .secondary
        case .notable: Brand.cyan
        case .problem: .red
        }
    }
}
