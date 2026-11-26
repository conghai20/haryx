import HaryXCore
import SwiftUI

/// One account in detail. Sections follow how often each part is needed: who it is and its state,
/// usage, live sessions, switching settings, then rarely used housekeeping under Advanced.
/// Every setting saves as soon as it changes; text fields save on Return or when focus leaves.
struct AccountInspector: View {
    @Environment(AppStore.self) private var store
    let account: Account

    @State private var alias = ""
    @State private var group = ""
    /// The account the text fields were filled from, so an edit saves to it even after the selection moves.
    @State private var loaded: Account?
    @State private var savedAlias = ""
    @State private var thresholdOn = false
    @State private var threshold = 90.0
    @State private var priority = 0
    @State private var moveSlot = 1
    @State private var swapWith: Int?
    @State private var showAdvanced = false
    @State private var confirmRemove = false
    @State private var confirmForce = false
    @FocusState private var focus: InspectorField?

    var body: some View {
        Form {
            InspectorHeader(account: account)
            UsageSection(account: account)
            WakeSection(account: account)
            SessionsSection(account: account)
            switchingSection
            advancedSection
            Section {
                Button("Remove Account…", role: .destructive) { confirmRemove = true }
            }
        }
        .formStyle(.grouped)
        .onAppear(perform: load)
        .onChange(of: account) { old, new in
            if old.identityKey != new.identityKey {
                commitText()
                load()
            } else {
                syncUnfocused()
            }
        }
        .onChange(of: focus) { old, _ in if old != nil { commitText() } }
        .confirmationDialog("Remove \(account.displayName)?", isPresented: $confirmRemove) {
            Button("Remove", role: .destructive) { Task { await store.remove(account); DashboardRouter.shared.selectedSlot = nil } }
        } message: {
            Text("Deletes its stored credentials and config backup. The Claude account itself is untouched.")
        }
        .confirmationDialog("Switch to \(account.displayName) without saving the current login?", isPresented: $confirmForce) {
            Button("Switch Anyway", role: .destructive) { Task { await store.switchTo(account, force: true) } }
        } message: {
            Text("Activates the stored credentials without first backing up the current login (--force). Use it when the current login is broken.")
        }
    }

    private var switchingSection: some View {
        Section {
            Toggle("Included in rotation", isOn: Binding(get: { !account.isDisabled },
                                                          set: { on in Task { await store.setDisabled(account, !on) } }))
            GroupField(account: account, group: $group, focus: $focus, onCommit: commitText)
            Toggle("Custom switch-away threshold", isOn: Binding(get: { thresholdOn }, set: { on in
                thresholdOn = on
                Task { await store.setThreshold(account, on ? threshold : nil) }
            }))
            if thresholdOn {
                LabeledContent("Switch away at") {
                    HStack {
                        Slider(value: $threshold, in: 50...99.9, step: 0.5) { editing in
                            if !editing { Task { await store.setThreshold(account, threshold) } }
                        }
                        .accessibilityValue("\(threshold, specifier: "%.1f") percent")
                        Text("\(threshold, specifier: "%.1f")%").monospacedDigit().frame(width: 52, alignment: .trailing)
                    }
                }
            }
        } header: {
            Text("Switching")
        } footer: {
            Text("Automatic switching tries accounts that share a group with the active one first, and moves off this account once its usage passes the threshold.")
        }
    }

    private var advancedSection: some View {
        Section {
            DisclosureGroup("Advanced", isExpanded: $showAdvanced) {
                MascotControls(account: account)
                TextField("Alias", text: $alias, prompt: Text("none"))
                    .focused($focus, equals: .alias)
                    .help("A short name shown instead of the email: letters, digits, . - _")
                    .onSubmit(commitText)
                Stepper("Priority: \(priority == 0 ? "none" : "\(priority)")", value: $priority, in: 0...1000, step: 10) { editing in
                    if !editing { Task { await store.setPriority(account, priority == 0 ? nil : priority) } }
                }
                .help("Read only by the priority-consume-first auto-switch strategy.")
                HStack {
                    Stepper("Position in list: \(moveSlot)", value: $moveSlot, in: 1...99)
                    Button("Move") { Task { await store.move(account, to: moveSlot) } }.disabled(moveSlot == account.number)
                }
                HStack {
                    Picker("Swap places with", selection: $swapWith) {
                        Text("—").tag(Int?.none)
                        ForEach(store.accounts.filter { $0.number != account.number }) { Text("\($0.number): \($0.displayName)").tag(Int?.some($0.number)) }
                    }
                    Button("Swap") {
                        if let other = swapWith.flatMap(store.account(slot:)) { Task { await store.swap(account, other) } }
                    }
                    .disabled(swapWith == nil)
                }
                Button("Switch Without Saving Current Login…") { confirmForce = true }
                    .disabled(account.active)
                    .help("Activate stored credentials without backing up the current login (--force)")
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    /// Saves a changed alias, and adds a typed group, to the account the fields were filled from.
    private func commitText() {
        guard let target = loaded else { return }
        let newAlias = alias.trimmingCharacters(in: .whitespaces)
        if newAlias != savedAlias {
            savedAlias = newAlias
            Task { await store.setAlias(target, newAlias.isEmpty ? nil : newAlias) }
        }
        // The group field adds one group (an account can be in several) and then empties.
        let newGroup = group.trimmingCharacters(in: .whitespaces)
        group = ""
        if !newGroup.isEmpty, !target.isInGroup(newGroup) {
            Task { await store.addToGroup(target, newGroup) }
        }
    }

    private func load() {
        loaded = account
        alias = account.alias ?? ""
        group = ""
        savedAlias = alias
        thresholdOn = account.threshold != nil
        threshold = account.threshold ?? store.configValue("autoswitch.threshold").flatMap { if case .number(let n) = $0 { n } else { nil } } ?? 90
        priority = store.priorities[account.number] ?? 0
        moveSlot = account.number
        swapWith = nil
    }

    /// A refresh of the same account: take its new values, but leave a field being typed in alone.
    private func syncUnfocused() {
        loaded = account
        if focus != .alias { alias = account.alias ?? ""; savedAlias = alias }
        thresholdOn = account.threshold != nil
        if let t = account.threshold { threshold = t }
        moveSlot = account.number
    }
}

fileprivate enum InspectorField { case alias, group }

/// Mascot, name, state, and the actions used most: Switch, Sign In Again, Run Claude.
private struct InspectorHeader: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        Section {
            HStack(spacing: 12) {
                MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 56, phase: store.phase(for: account))
                    .help(moodCaption)
                    .accessibilityLabel(moodCaption)
                VStack(alignment: .leading, spacing: 2) {
                    Text(account.displayName).font(.title3.weight(.semibold)).lineLimit(1).truncationMode(.middle)
                    if let detail { Text(detail).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle) }
                    Text(stateText).font(.callout.weight(.medium)).foregroundStyle(stateColor)
                }
                Spacer(minLength: 0)
            }
            if let note = account.statusNote {
                Label(note, systemImage: "exclamationmark.triangle.fill").foregroundStyle(Brand.warn)
            }
            HStack {
                if !account.active {
                    Button("Switch") { Task { await store.switchTo(account) } }
                        .buttonStyle(.glassProminent)
                        .keyboardShortcut(.return, modifiers: .command)
                        .help("Switch to \(account.displayName) (⌘↩)")
                }
                if account.offersSignIn { SignInAgainButton(account: account, prominent: account.needsRelogin) }
                Menu("Run Claude") {
                    TerminalMenuItems { store.runClaude(as: account, app: $0) }
                    Divider()
                    Button("In Folder…") {
                        if let dir = AppStore.chooseFolder(prompt: "Run Claude Here") { store.runClaude(as: account, folder: dir) }
                    }
                }
                .fixedSize()
                .help("Start Claude Code as this account in a new terminal")
                Spacer(minLength: 0)
            }
        }
    }

    /// The email and organisation when the title is an alias; else just the organisation.
    private var detail: String? {
        let parts = (account.alias == nil ? [] : [account.email]) + [account.organizationName].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    private var sessionCount: Int { store.sessions[account.number]?.count ?? 0 }

    private var stateText: String {
        let running = sessionCount == 0 ? nil : "\(sessionCount) session\(sessionCount == 1 ? "" : "s") running"
        if account.active { return ["In use", running].compactMap { $0 }.joined(separator: " · ") }
        if let running { return running }
        if account.needsRelogin { return "Sign-in needed" }
        if account.isDisabled { return "Out of rotation" }
        return "Ready"
    }

    private var stateColor: Color {
        if account.active || sessionCount > 0 { return Brand.cyanText }
        return account.needsRelogin ? Brand.warn : .secondary
    }

    private var moodCaption: String {
        switch store.mood(for: account) {
        case .running: "Working: a Claude session is using this account."
        case .idle: "Idle."
        case .angry: "Angry: 75% or more of a quota used."
        case .tired: "Tired: 90% or more of a quota used."
        case .sleeping: account.isDisabled ? "Sleeping: out of rotation." : "Sleeping: at its limit until the quota resets."
        case .error: "Needs attention: \(account.statusNote ?? "check login")."
        }
    }
}

/// The account's groups, each with a remove button, and a field that adds one: type a name and
/// press Return, or pick one already in use. An account can be in several groups.
private struct GroupField: View {
    @Environment(AppStore.self) private var store
    let account: Account
    @Binding var group: String
    var focus: FocusState<InspectorField?>.Binding
    let onCommit: () -> Void

    var body: some View {
        LabeledContent("Groups") {
            VStack(alignment: .trailing, spacing: 6) {
                ForEach(account.groupNames, id: \.self) { name in
                    HStack(spacing: 6) {
                        Text(name)
                        Button { Task { await store.removeFromGroup(account, name) } } label: {
                            Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                        .disabled(store.busy != nil)
                        .help("Take \(account.displayName) out of \(name)")
                        .accessibilityLabel("Remove from \(name)")
                    }
                }
                HStack(spacing: 4) {
                    TextField("Add to group", text: $group, prompt: Text(account.groupNames.isEmpty ? "none" : "add another"))
                        .labelsHidden()
                        .multilineTextAlignment(.trailing)
                        .focused(focus, equals: .group)
                        .onSubmit(onCommit)
                        .onChange(of: group) { _, new in
                            if new.count > AccountGroups.maxNameLength { group = String(new.prefix(AccountGroups.maxNameLength)) }
                        }
                    let others = AccountGroups.names(store.accounts).filter { !account.isInGroup($0) }
                    if !others.isEmpty {
                        Menu {
                            ForEach(others, id: \.self) { name in Button(name) { group = name; onCommit() } }
                        } label: { Image(systemName: "chevron.down") }
                        .menuIndicator(.hidden)
                        .fixedSize()
                        .help("Add to an existing group")
                        .accessibilityLabel("Existing groups")
                    }
                }
            }
        }
    }
}

/// Species and colour for the account's mascot; the mascot itself shows in the header.
private struct MascotControls: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        let style = store.style(for: account)
        Picker("Mascot", selection: Binding(get: { style.species }, set: { store.styles.set(MascotStyle(species: $0, hue: style.hue), for: account.identityKey) })) {
            ForEach(Species.allCases) { Text($0.title).tag($0) }
        }
        LabeledContent("Mascot colour") {
            HStack {
                Slider(value: Binding(get: { style.hue ?? style.species.baseHSB.h },
                                      set: { store.styles.set(MascotStyle(species: style.species, hue: $0), for: account.identityKey) }),
                       in: 0...1)
                    .tint(Color(hue: style.hue ?? style.species.baseHSB.h, saturation: 0.7, brightness: 0.9))
                    .accessibilityValue(style.hue == nil ? "default" : "hue \(Int((style.hue ?? 0) * 360)) degrees")
                Button("Reset") { store.styles.set(nil, for: account.identityKey) }.controlSize(.small)
            }
        }
    }
}

private struct UsageSection: View {
    let account: Account

    /// The cards' pace wording, with the same threshold, so the panel never contradicts the bar's colour.
    static func paceText(_ delta: Int) -> String {
        if delta >= UsageWindow.paceThreshold { return "\(delta) pts ahead" }
        if delta <= -UsageWindow.paceThreshold { return "\(-delta) pts under" }
        return "On pace"
    }

    var body: some View {
        Section("Usage") {
            if let err = account.usageError {
                Label(err, systemImage: "exclamationmark.circle").font(.callout).foregroundStyle(Brand.warn)
            }
            if let u = account.usage {
                if u.fiveHour != nil { UsageBar(label: "5-hour", window: u.fiveHour?.withPace(windowSeconds: UsageWindow.fiveHourSeconds)) }
                UsageBar(label: "Week", window: u.sevenDay)
                ForEach(u.scoped ?? [], id: \.name) { s in
                    UsageBar(label: s.name, window: UsageWindow(pct: s.pct, resetsAt: s.resetsAt))
                }
                if let w = u.sevenDay, let d = w.paceDelta, let expected = w.expectedPct {
                    LabeledContent("Weekly pace") {
                        Text("\(Self.paceText(d)) (expected \(Int(expected.rounded()))%)")
                            .foregroundStyle(w.isAheadOfPace ? Brand.warn : .secondary)
                    }
                    if let end = w.projectedExhaustionAt, w.willLastToReset == false {
                        LabeledContent("Projected run-out", value: end.formatted(date: .abbreviated, time: .shortened))
                    }
                }
            }
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
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(s.cwd.map { ($0 as NSString).abbreviatingWithTildeInPath } ?? "pid \(s.pid)").lineLimit(1)
                            Text("pid \(s.pid) · started \(s.startedAt.formatted(.relative(presentation: .named)))\(s.configDir == nil ? "" : " · session mode")")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        ContinueMenu(process: s, from: account.number)
                    }
                }
            }
        }
    }
}

/// "Continue on…": end this session and resume the conversation on another account.
struct ContinueMenu: View {
    @Environment(AppStore.self) private var store
    let process: ClaudeProcess
    /// The account it runs on, left out of the choices; nil when it runs on none.
    let from: Int?

    var body: some View {
        Menu("Continue on…") {
            if let best = store.bestTarget(excluding: from) {
                Button("Best: \(best.displayName) (\(Int(best.peakPct ?? 0))% used)") {
                    store.continueSession(process, on: best)
                }
                Divider()
            }
            ForEach(store.accounts.filter { $0.number != from && !$0.isDisabled }) { a in
                Button("\(a.number). \(a.displayName)\(a.peakPct.map { " — \(Int($0))% used" } ?? "")") {
                    store.continueSession(process, on: a)
                }
                .disabled(a.needsRelogin)
            }
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .help("Ends this Claude session and resumes the same conversation on another account, in a new terminal.")
    }
}

struct AddAccountSheet: View {
    enum Mode: String, Identifiable { case login, current, token; var id: String { rawValue } }

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
            Text(title).font(.title3.bold())
            Text(explanation)
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
                    if mode != .login { Toggle("Overwrite if the slot is taken", isOn: $overwrite) }
                }
            }
            .formStyle(.grouped)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button(mode == .login ? "Sign In" : "Add") {
                    Task {
                        if mode == .login {
                            store.signInNew(slot: useSlot ? slot : nil, alias: alias)
                        } else if mode == .current {
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

    private var title: String {
        switch mode {
        case .login: "Sign In New Account"
        case .current: "Add Current Claude Login"
        case .token: "Add Setup Token or API Key"
        }
    }

    private var explanation: String {
        switch mode {
        case .login:
            "Opens Claude Code's sign-in in a terminal and your browser. Sign in with your own account; it is added without changing the account in use. If the slot is taken, the terminal asks before replacing it."
        case .current:
            "Captures the account Claude Code is logged into right now. Log in with `claude` first, and don't /logout the previous account or its token may be revoked. Re-adding an existing account refreshes its credentials."
        case .token:
            "Registers an `sk-ant-oat01-…` setup token or an `sk-ant-api03-…` API key without a browser login. The token is passed over stdin and never appears in process arguments."
        }
    }
}
