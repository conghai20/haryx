import CSwapCore
import SwiftUI

struct AccountInspector: View {
    @Environment(AppStore.self) private var store
    let account: Account

    @State private var alias = ""
    @State private var thresholdOn = false
    @State private var threshold = 90.0
    @State private var priority = 0
    @State private var moveSlot = 1
    @State private var swapWith: Int?
    @State private var confirmRemove = false
    @State private var confirmForce = false

    var body: some View {
        Form {
            MascotSection(account: account)

            Section("Actions") {
                HStack {
                    Button("Switch") { Task { await store.switchTo(account) } }
                        .buttonStyle(.glassProminent).disabled(account.active)
                    Button("Force Switch…") { confirmForce = true }
                        .help("Activate stored credentials without backing up the current login (--force)")
                }
                Menu("Run Claude as This Account") { TerminalMenuItems { store.runClaude(as: account, app: $0) } }
                Button("Run Claude in Folder…") {
                    if let dir = AppStore.chooseFolder(prompt: "Run Claude Here") { store.runClaude(as: account, folder: dir) }
                }
            }

            UsageSection(account: account)

            Section("Identity") {
                LabeledContent("Email", value: account.email)
                if let org = account.organizationName { LabeledContent("Organization", value: org) }
                HStack {
                    TextField("Alias", text: $alias, prompt: Text("letters, digits, . - _"))
                    Button("Save") { Task { await store.setAlias(account, alias) } }
                        .disabled(alias.isEmpty || alias == account.alias)
                    if account.alias != nil {
                        Button("Clear") { Task { await store.setAlias(account, nil) } }
                    }
                }
            }

            Section("Rotation") {
                Toggle("Included in rotation", isOn: Binding(get: { !account.isDisabled },
                                                              set: { on in Task { await store.setDisabled(account, !on) } }))
                Toggle("Custom switch-away threshold", isOn: $thresholdOn)
                if thresholdOn {
                    HStack {
                        Slider(value: $threshold, in: 50...99.9, step: 0.5)
                        Text("\(threshold, specifier: "%.1f")%").monospacedDigit().frame(width: 52)
                    }
                }
                Button("Apply Threshold") { Task { await store.setThreshold(account, thresholdOn ? threshold : nil) } }
                    .disabled(thresholdOn ? threshold == account.threshold : account.threshold == nil)
                Stepper("Priority tier: \(priority)", value: $priority, in: 0...1000, step: 10)
                HStack {
                    Button("Apply Priority") { Task { await store.setPriority(account, priority) } }
                        .disabled(priority == (store.priorities[account.number] ?? 0))
                    Button("Reset") { Task { await store.setPriority(account, nil) } }
                }
                Text("Priority is used by the priority-consume-first strategy.").font(.caption).foregroundStyle(.secondary)
            }

            Section("Slot") {
                HStack {
                    Stepper("Move to slot \(moveSlot)", value: $moveSlot, in: 1...99)
                    Button("Move") { Task { await store.move(account, to: moveSlot) } }.disabled(moveSlot == account.number)
                }
                HStack {
                    Picker("Swap with", selection: $swapWith) {
                        Text("—").tag(Int?.none)
                        ForEach(store.accounts.filter { $0.number != account.number }) { Text("\($0.number): \($0.displayName)").tag(Int?.some($0.number)) }
                    }
                    Button("Swap") {
                        if let other = swapWith.flatMap(store.account(slot:)) { Task { await store.swap(account, other) } }
                    }
                    .disabled(swapWith == nil)
                }
            }

            SessionsSection(account: account)

            Section {
                Button("Remove Account…", role: .destructive) { confirmRemove = true }
            }
        }
        .formStyle(.grouped)
        .onAppear(perform: load)
        .onChange(of: account) { load() }
        .confirmationDialog("Remove \(account.displayName)?", isPresented: $confirmRemove) {
            Button("Remove", role: .destructive) { Task { await store.remove(account); DashboardRouter.shared.selectedSlot = nil } }
        } message: {
            Text("Deletes its stored credentials and config backup. The Claude account itself is untouched.")
        }
        .confirmationDialog("Force switch to \(account.displayName)?", isPresented: $confirmForce) {
            Button("Force Switch", role: .destructive) { Task { await store.switchTo(account, force: true) } }
        } message: {
            Text("Activates the stored credentials without first backing up the current login.")
        }
    }

    private func load() {
        alias = account.alias ?? ""
        thresholdOn = account.threshold != nil
        threshold = account.threshold ?? store.configValue("autoswitch.threshold").flatMap { if case .number(let n) = $0 { n } else { nil } } ?? 90
        priority = store.priorities[account.number] ?? 0
        moveSlot = account.number
    }
}

private struct MascotSection: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        let style = store.style(for: account)
        Section {
            HStack {
                Spacer()
                MascotView(style: style, mood: store.mood(for: account), size: 112, phase: store.phase(for: account))
                Spacer()
            }
            Picker("Mascot", selection: Binding(get: { style.species }, set: { store.styles.set(MascotStyle(species: $0, hue: style.hue), for: account.identityKey) })) {
                ForEach(Species.allCases) { Text($0.title).tag($0) }
            }
            HStack {
                Text("Colour")
                Slider(value: Binding(get: { style.hue ?? style.species.baseHSB.h },
                                      set: { store.styles.set(MascotStyle(species: style.species, hue: $0), for: account.identityKey) }),
                       in: 0...1)
                    .tint(Color(hue: style.hue ?? style.species.baseHSB.h, saturation: 0.7, brightness: 0.9))
                Button("Reset") { store.styles.set(nil, for: account.identityKey) }.controlSize(.small)
            }
            Text(moodCaption).font(.caption).foregroundStyle(.secondary)
        }
    }

    private var moodCaption: String {
        switch store.mood(for: account) {
        case .running: "Working: a Claude session is using this account."
        case .idle: "Idle."
        case .tired: "Tired: 90% or more of a quota used."
        case .sleeping: "Sleeping: disabled from rotation."
        case .error: "Needs attention: \(account.statusNote ?? "check login")."
        }
    }
}

private struct UsageSection: View {
    let account: Account

    var body: some View {
        Section("Usage") {
            if let u = account.usage {
                if u.fiveHour != nil { UsageBar(label: "5-HOUR", window: u.fiveHour) }
                UsageBar(label: "WEEKLY", window: u.sevenDay)
                ForEach(u.scoped ?? [], id: \.name) { s in
                    UsageBar(label: s.name.uppercased(), window: UsageWindow(pct: s.pct, resetsAt: s.resetsAt))
                }
                if let w = u.sevenDay, let expected = w.expectedPct {
                    LabeledContent("Weekly pace") {
                        Text(w.aheadOfPace == true ? "Ahead (expected \(Int(expected))%)" : "On track (expected \(Int(expected))%)")
                            .foregroundStyle(w.aheadOfPace == true ? .orange : .secondary)
                    }
                    if let end = w.projectedExhaustionAt, w.willLastToReset == false {
                        LabeledContent("Projected run-out", value: end.formatted(date: .abbreviated, time: .shortened))
                    }
                }
            }
            if let note = account.statusNote { Text(note).foregroundStyle(.orange) }
            if let err = account.usageError { Text(err).font(.caption).foregroundStyle(.secondary) }
            if let exp = account.loginExpiresAt {
                LabeledContent("Login expires", value: exp.formatted(date: .abbreviated, time: .shortened))
            }
            if let age = account.usageAgeSeconds {
                LabeledContent("Usage fetched", value: Date(timeIntervalSinceNow: -age).formatted(.relative(presentation: .named)))
            }
        }
    }
}

private struct SessionsSection: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        let sessions = store.sessions[account.number] ?? []
        if !sessions.isEmpty {
            Section("Live Claude sessions") {
                ForEach(sessions) { s in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(s.cwd.map { ($0 as NSString).abbreviatingWithTildeInPath } ?? "pid \(s.pid)").lineLimit(1)
                        Text("pid \(s.pid) · started \(s.startedAt.formatted(.relative(presentation: .named)))\(s.configDir == nil ? "" : " · session mode")")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }
}

struct AddAccountSheet: View {
    enum Mode: String, Identifiable { case current, token; var id: String { rawValue } }

    @Environment(AppStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let mode: Mode

    @State private var useSlot = false
    @State private var slot = 1
    @State private var alias = ""
    @State private var email = ""
    @State private var token = ""
    @State private var overwrite = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(mode == .current ? "Add Current Claude Login" : "Add Setup Token or API Key").font(.title3.bold())
            Text(mode == .current
                 ? "Captures the account Claude Code is logged into right now. Log in with `claude` first, and don't /logout the previous account or its token may be revoked. Re-adding an existing account refreshes its credentials."
                 : "Registers an `sk-ant-oat01-…` setup token or an `sk-ant-api03-…` API key without a browser login. The token is passed over stdin and never appears in process arguments.")
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            Form {
                if mode == .token {
                    SecureField("Token", text: $token)
                    TextField("Email label (optional)", text: $email)
                } else {
                    TextField("Alias (optional)", text: $alias)
                }
                Toggle("Choose slot", isOn: $useSlot)
                if useSlot {
                    Stepper("Slot \(slot)", value: $slot, in: 1...99)
                    Toggle("Overwrite if the slot is taken", isOn: $overwrite)
                }
            }
            .formStyle(.grouped)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Add") {
                    Task {
                        if mode == .current {
                            await store.addCurrent(slot: useSlot ? slot : nil, alias: alias, overwrite: overwrite)
                        } else {
                            await store.addToken(token, slot: useSlot ? slot : nil, email: email, overwrite: overwrite)
                        }
                        dismiss()
                    }
                }
                .buttonStyle(.glassProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(mode == .token && !token.trimmingCharacters(in: .whitespaces).hasPrefix("sk-ant-"))
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}
