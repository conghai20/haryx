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
        "history.import": ["all", "none", "ask"],
    ]

    /// What each key reads as in the app: a plain label, a caption and the unit after a number field.
    /// The dotted key itself only shows in the hover help. The Windows app uses the same copy.
    struct Copy {
        let label: String
        var caption: String? = nil
        var unit: String? = nil
    }

    static let copy: [String: Copy] = [
        "autoswitch.threshold": Copy(label: "Switch away at", caption: "When the active account's 5-hour window reaches this. 50–99.9.", unit: "%"),
        "autoswitch.weeklyThreshold": Copy(label: "Weekly switch line", caption: "For weekly and per-model limits. Blank uses the line above.", unit: "%"),
        "autoswitch.intervalSeconds": Copy(label: "Check every", caption: "How often auto-switch reads usage. 15–3600.", unit: "s"),
        "autoswitch.cooldownSeconds": Copy(label: "Wait between switches", caption: "0–86400.", unit: "s"),
        "autoswitch.hysteresisPct": Copy(label: "Headroom margin", caption: "Only switch to an account with this many more points of headroom. 0–50.", unit: "pts"),
        "autoswitch.strategy": Copy(label: "Pick the next account by"),
        "autoswitch.includeApiKeyAccounts": Copy(label: "Include API-key accounts", caption: "They bill per token."),
        "autoswitch.unhealthyTicks": Copy(label: "Failed checks before quarantine"),
        "autoswitch.model": Copy(label: "Also watch model limits", caption: "Per-model weekly limits to count, e.g. Fable, Fable,Opus or all."),
        "usage.headerProbe": Copy(label: "Read setup-token usage with a 1-token request", caption: "Uses a little quota."),
        "usage.fableProbe": Copy(label: "Also read Fable's weekly limit", caption: "Needs the setting above. Uses a little Fable quota."),
        "usage.shareBetweenMachines": Copy(label: "Share usage readings with your other machines", caption: "Through HaryX sync, so each account spends one usage budget."),
        "run.shareHistory": Copy(label: "Share one chat history across accounts", caption: "For haryx ai run."),
        "updates.check": Copy(label: "Check for updates daily"),
        "ui.theme": Copy(label: "Terminal colour theme", caption: "Auto follows the terminal background."),
        "ui.orgTag": Copy(label: "Show organisation tag", caption: "Auto shows it only where accounts share an email."),
        "history.enabled": Copy(label: "Keep a usage history", caption: "From Claude Code's transcripts on this Mac. No conversation text is stored."),
        "history.import": Copy(label: "Past transcripts on first read"),
        "history.keepDays": Copy(label: "Keep usage history for", caption: "0 keeps everything.", unit: "days"),
    ]

    /// The Auto-switch rules in the order they read best: when, how often, which account.
    static let ruleOrder = [
        "autoswitch.threshold", "autoswitch.weeklyThreshold", "autoswitch.strategy", "autoswitch.hysteresisPct",
        "autoswitch.intervalSeconds", "autoswitch.cooldownSeconds", "autoswitch.unhealthyTicks",
        "autoswitch.includeApiKeyAccounts", "autoswitch.model",
    ]

    @FocusState private var focused: Bool

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
                Text(Self.copy[entry.key]?.label ?? entry.key)
                if let caption = Self.copy[entry.key]?.caption { Text(caption).font(.caption).foregroundStyle(.secondary) }
            }
        }
        .help(entry.key)
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
            HStack(spacing: 4) {
                TextField("", text: $text, prompt: Text("default"))
                    .frame(width: Self.copy[entry.key]?.unit == nil ? 140 : 80)
                    .multilineTextAlignment(.trailing)
                    .focused($focused)
                    .onSubmit { Task { await commit() } }
                    .onExitCommand { text = entry.value.description }
                    // Saves when you leave the field too, so nothing typed is left unsaved.
                    .onChange(of: focused) { _, isFocused in if !isFocused { Task { await commit() } } }
                if let unit = Self.copy[entry.key]?.unit, !text.isEmpty { Text(unit).foregroundStyle(.secondary) }
            }
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
            ForEach(entries.sorted(by: Self.order)) { entry in
                ConfigRow(entry: entry) { isUnsaved in
                    unsaved = isUnsaved ? unsaved.union([entry.key]) : unsaved.subtracting([entry.key])
                }
            }
        }
    }
}

extension ConfigSection {
    /// Known keys in `ConfigRow.ruleOrder`, the rest after them in key order.
    static func order(_ a: ConfigEntry, _ b: ConfigEntry) -> Bool {
        let rank = { (k: String) in ConfigRow.ruleOrder.firstIndex(of: k) ?? Int.max }
        return (rank(a.key), a.key) < (rank(b.key), b.key)
    }
}

/// Bottom-right notice listing settings typed but not saved yet; hidden while there are none.
struct UnsavedConfigNotice: View {
    let keys: Set<String>

    var body: some View {
        if !keys.isEmpty {
            Label("Not saved: \(keys.sorted().map { ConfigRow.copy[$0]?.label ?? $0 }.joined(separator: ", ")). Press Return to save, Esc to discard.",
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
            // The autoswitch.* rules live in Auto-switch only.
            let groups = Dictionary(grouping: store.config.filter { !$0.key.hasPrefix("autoswitch.") }) {
                String($0.key.split(separator: ".").first ?? "")
            }
            ForEach(groups.keys.sorted(), id: \.self) { group in
                ConfigSection(title: Self.groupTitle(group), entries: groups[group] ?? [], unsaved: $unsaved)
            }
            Section {
                LabeledContent("File", value: store.configPath)
                Button("Reveal in Finder") { NSWorkspace.shared.selectFile(store.configPath, inFileViewerRootedAtPath: "") }
                    .disabled(store.configPath.isEmpty)
            } footer: {
                Text("Auto-switch rules are in Auto-switch. Unknown keys and hand edits in settings.json are preserved.")
            }
        }
        .formStyle(.grouped)
        .paneFrame()
        .overlay(alignment: .bottomTrailing) { UnsavedConfigNotice(keys: unsaved) }
        .animation(.default, value: unsaved)
    }
}

extension ConfigPane {
    /// Section titles for the CLI's setting groups, in sentence case.
    static func groupTitle(_ group: String) -> String {
        switch group {
        case "history": "Usage history"
        case "usage": "Usage readings"
        case "run": "Run"
        case "ui": "Command-line display"
        case "updates": "Updates"
        default: group.prefix(1).uppercased() + group.dropFirst()
        }
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
            .frame(minWidth: 380, maxWidth: 720)

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
