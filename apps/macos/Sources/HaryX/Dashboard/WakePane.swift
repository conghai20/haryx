import HaryXCore
import SwiftUI

/// `haryx ai wake`: the background agent, settings, each account's last wake and the recent events.
struct WakePane: View {
    @Environment(AppStore.self) private var store
    @State private var quietOn = false
    @State private var quietStart = WakePane.time(1, 0)
    @State private var quietEnd = WakePane.time(6, 0)
    @State private var cap = 90.0

    var body: some View {
        Form {
            if CSwapCLI.selectedFlavor != .haryx {
                Section { Text("Wake needs the haryx CLI. The selected backend is cswap.").foregroundStyle(.secondary) }
            } else if let wake = store.wake {
                agentSection(wake)
                settingsSection
                accountsSection(wake)
                eventsSection(wake)
            } else if store.wakeError != nil {
                Section {
                    Label("This haryx has no wake command. Update it (haryx upgrade) to use Wake.",
                          systemImage: "exclamationmark.triangle")
                }
            } else {
                Section { ProgressView("Loading wake status…") }
            }
        }
        .formStyle(.grouped)
        .onAppear(perform: load)
        .onChange(of: store.wake) { load() }
    }

    private func agentSection(_ wake: WakeStatus) -> some View {
        Section {
            LabeledContent("Background check") {
                HStack {
                    Text(wake.agentInstalled ? "Every minute" : "Off").foregroundStyle(.secondary)
                    Button(wake.agentInstalled ? "Remove" : "Install") {
                        Task { await store.setWakeAgent(installed: !wake.agentInstalled) }
                    }
                }
            }
        } footer: {
            Text("When an opted-in account's 5-hour window resets, one 1-token request starts the next window. Checks with nothing due read local files only.")
        }
    }

    private var settingsSection: some View {
        Section("Settings") {
            Toggle("Quiet hours", isOn: $quietOn)
            if quietOn {
                DatePicker("From", selection: $quietStart, displayedComponents: .hourAndMinute)
                DatePicker("Until", selection: $quietEnd, displayedComponents: .hourAndMinute)
            }
            LabeledContent("Skip at weekly use") {
                HStack {
                    Slider(value: $cap, in: 50...100, step: 5)
                    Text("\(Int(cap))%").monospacedDigit().frame(width: 40, alignment: .trailing)
                }
            }
            Button("Save") {
                Task { await store.setWakeConfig(quietHours: quietOn ? quietText : nil, weeklyCapPct: cap) }
            }
            .disabled(store.busy != nil)
        }
    }

    private func accountsSection(_ wake: WakeStatus) -> some View {
        Section {
            let onCount = store.accounts.filter { wake.account($0.number)?.on == true }.count
            Toggle(isOn: Binding(get: { !store.accounts.isEmpty && onCount == store.accounts.count },
                                 set: { on in Task { await store.setWakeAll(on) } })) {
                Text("All accounts").fontWeight(.medium)
                Text("\(onCount) of \(store.accounts.count) on")
            }
            .disabled(store.accounts.isEmpty || store.busy != nil)
            ForEach(store.accounts) { a in
                let row = wake.account(a.number)
                Toggle(isOn: Binding(get: { row?.on ?? false }, set: { on in Task { await store.setWake(a, on) } })) {
                    Text(a.displayName)
                    if let row { statusLine(row) }
                }
            }
        } header: {
            Text("Accounts")
        } footer: {
            if !wake.agentInstalled, wake.accounts.contains(where: \.on) {
                Text("The background check is off, so opted-in accounts are not checked. Install it above.")
            }
        }
    }

    /// "Last wake … · Next due … · Not due", the result in red when the wake failed.
    private func statusLine(_ row: WakeAccount) -> Text {
        let failed = row.lastResult?.hasPrefix("failed") == true
        let result = Text(WakeStatus.resultText(row.lastResult)).foregroundStyle(failed ? Color.red : Color.secondary)
        return Text("Last wake \(Self.when(row.lastWakeAt)) · Next due \(Self.when(row.nextDue)) · \(result)")
    }

    private func eventsSection(_ wake: WakeStatus) -> some View {
        Section("Recent events") {
            let events = wake.recentEvents()
            if events.isEmpty { Text("No wakes yet.").foregroundStyle(.secondary) }
            ForEach(Array(events.enumerated()), id: \.offset) { _, ev in
                HStack {
                    Text(Self.when(ev.date)).monospacedDigit().foregroundStyle(.tertiary).frame(width: 130, alignment: .leading)
                    Text(eventName(ev.account)).lineLimit(1)
                    Spacer()
                    Text(WakeStatus.resultText(ev.result)).foregroundStyle(ev.failed ? Color.red : Color.secondary)
                }
                .font(.callout)
            }
        }
    }

    private func eventName(_ ref: String?) -> String {
        guard let ref else { return "—" }
        return Int(ref).flatMap(store.account(slot:))?.displayName ?? "Account \(ref)"
    }

    static func when(_ date: Date?) -> String {
        date?.formatted(date: .numeric, time: .shortened) ?? "—"
    }

    private var quietText: String {
        let f = { (d: Date) in
            let c = Calendar.current.dateComponents([.hour, .minute], from: d)
            return String(format: "%02d:%02d", c.hour ?? 0, c.minute ?? 0)
        }
        return "\(f(quietStart))-\(f(quietEnd))"
    }

    private func load() {
        guard let wake = store.wake else { return }
        cap = wake.weeklyCapPct
        quietOn = wake.quietHours != nil
        if let q = wake.quietHours, let (s, e) = Self.parse(q) { quietStart = s; quietEnd = e }
    }

    static func time(_ h: Int, _ m: Int) -> Date {
        Calendar.current.date(bySettingHour: h, minute: m, second: 0, of: Date()) ?? Date()
    }

    static func parse(_ text: String) -> (Date, Date)? {
        let parts = text.split(separator: "-").map { $0.split(separator: ":").compactMap { Int($0) } }
        guard parts.count == 2, parts.allSatisfy({ $0.count == 2 }) else { return nil }
        return (time(parts[0][0], parts[0][1]), time(parts[1][0], parts[1][1]))
    }
}

/// The inspector's wake toggle for one account.
struct WakeSection: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        if CSwapCLI.selectedFlavor == .haryx, let wake = store.wake {
            let row = wake.account(account.number)
            Section {
                Toggle("Start the next 5-hour window after each reset",
                       isOn: Binding(get: { row?.on ?? false }, set: { on in Task { await store.setWake(account, on) } }))
                if let s = row?.summary() { LabeledContent("Status", value: s) }
                if row?.on == true, !wake.agentInstalled {
                    LabeledContent("Background check is off") {
                        Button("Install") { Task { await store.setWakeAgent(installed: true) } }
                    }
                }
            } header: {
                Text("Wake")
            }
        }
    }
}
