import HaryXCore
import SwiftUI

/// Edits one `cswap config` key with a control matched to its value type.
struct ConfigRow: View {
    @Environment(AppStore.self) private var store
    let entry: ConfigEntry
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
        }
    }

    private func commit() async {
        let value = text.trimmingCharacters(in: .whitespaces)
        if value.isEmpty { await store.configUnset(entry.key) }
        else if value != entry.value.description { await store.configSet(entry.key, value) }
    }
}

struct ConfigPane: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        Form {
            let groups = Dictionary(grouping: store.config) { String($0.key.split(separator: ".").first ?? "") }
            ForEach(groups.keys.sorted(), id: \.self) { group in
                Section(group.capitalized) {
                    ForEach(groups[group] ?? []) { ConfigRow(entry: $0) }
                }
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
    }
}

struct AutoSwitchPane: View {
    @Environment(AppStore.self) private var store
    @AppStorage(Prefs.autoDryRun) private var dryRun = false
    @AppStorage(Prefs.autoOnLaunch) private var onLaunch = false

    var body: some View {
        HSplitView {
            Form {
                Section {
                    HStack(spacing: 14) {
                        Image(systemName: "arrow.triangle.2.circlepath")
                            .font(.largeTitle)
                            .foregroundStyle(store.auto.isRunning ? .green : .secondary)
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
                            store.auto.isRunning ? store.auto.stop() : store.startAuto()
                        }
                        .buttonStyle(.glassProminent)
                    }
                    Toggle("Dry run (log decisions, don't switch)", isOn: $dryRun)
                        .disabled(store.auto.isRunning)
                    Toggle("Start when HaryX launches", isOn: $onLaunch)
                    Button("Check Once Now") { Task { await store.runAutoOnce() } }
                } footer: {
                    Text("Runs `haryx auto --json` inside HaryX. It switches the active account when a quota window nears its threshold, and every decision appears in the log.")
                }
                Section("Rules") {
                    ForEach(store.config.filter { $0.key.hasPrefix("autoswitch.") }) { ConfigRow(entry: $0) }
                }
            }
            .formStyle(.grouped)
            .frame(minWidth: 380)

            List(store.auto.events) { event in
                HStack(alignment: .firstTextBaseline) {
                    Image(systemName: icon(event)).foregroundStyle(color(event))
                    VStack(alignment: .leading, spacing: 2) {
                        Text(event.summary).font(.callout)
                        Text(event.date.formatted(date: .omitted, time: .standard)).font(.caption2).foregroundStyle(.tertiary)
                    }
                }
            }
            .overlay { if store.auto.events.isEmpty { ContentUnavailableView("No events yet", systemImage: "list.bullet.rectangle") } }
            .frame(minWidth: 300)
        }
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
        case .notable: .accentColor
        case .problem: .red
        }
    }
}
