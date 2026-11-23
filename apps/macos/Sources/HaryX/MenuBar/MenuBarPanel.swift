import HaryXCore
import SwiftUI

struct MenuBarPanel: View {
    @Environment(AppStore.self) private var store
    static let width: CGFloat = 380

    var body: some View {
        GlassEffectContainer(spacing: 12) {
            VStack(alignment: .leading, spacing: 0) {
                header
                Hairline().padding(.vertical, 10)
                if let active = store.active {
                    ActiveTide(account: active)
                } else if let error = store.refreshError {
                    ErrorCard(message: error)
                }
                let queue = store.nextInLine
                if !queue.isEmpty {
                    Hairline().padding(.vertical, 10)
                    NextInLine(accounts: queue)
                }
                if let event = store.auto.events.last(where: { $0.severity != .info }) {
                    Text("\(event.date.formatted(date: .omitted, time: .shortened))  \(event.summary)")
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        .padding(.top, 8)
                }
                if let banner = store.banner { BannerView(banner: banner).padding(.top, 8) }
                QuickActions().padding(.top, 12)
            }
            .padding(14)
        }
        .frame(width: Self.width)
        .environment(\.mascotsPaused, !store.panelVisible)
        .animation(.snappy, value: store.banner)
        .onChange(of: store.panelVisible) { _, shown in
            if shown, (store.lastRefresh?.timeIntervalSinceNow ?? -999) < -15 { Task { await store.refresh() } }
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            BrandLockup(size: 18)
            if let busy = store.busy {
                ProgressView().controlSize(.mini)
                Text(busy).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 4)
            Button {
                store.auto.isRunning ? store.auto.stop() : store.startAuto()
            } label: {
                Text(store.auto.isRunning ? "Auto-switch on" : "Auto-switch off")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(store.auto.isRunning ? Brand.cyanText : .secondary)
            }
            .help(store.auto.isRunning ? "Stop auto-switch" : "Start auto-switch")
            Group {
                Button { Task { await store.refresh() } } label: { Image(systemName: "arrow.clockwise") }
                    .help("Refresh").keyboardShortcut("r").accessibilityLabel("Refresh")
                Button { store.signInNew() } label: { Image(systemName: "plus") }
                    .disabled(store.addLoginPending)
                    .help(store.addLoginPending ? "A new-account sign-in is already open" : "Sign in a new account")
                    .accessibilityLabel("Sign in a new account")
                Menu {
                    Button("Open Dashboard") { DashboardWindow.shared.show() }.keyboardShortcut("d")
                    Divider()
                    Button("Quit HaryX") { NSApp.terminate(nil) }.keyboardShortcut("q")
                } label: { Image(systemName: "ellipsis") }
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .accessibilityLabel("More")
            }
            .foregroundStyle(.secondary)
        }
        .buttonStyle(.borderless)
    }
}

/// A 1 pt rule.
private struct Hairline: View {
    var body: some View { Rectangle().fill(.separator).frame(height: 1) }
}

/// The active account read like a tide table: observed use against the predicted pace, the
/// next reset as the next low water.
private struct ActiveTide: View {
    @Environment(AppStore.self) private var store
    let account: Account
    private static let fiveHours: TimeInterval = 5 * 3600

    var body: some View {
        let five = account.usage?.fiveHour
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 36,
                           phase: store.phase(for: account))
                VStack(alignment: .leading, spacing: 2) {
                    Text(account.displayName).font(.headline).lineLimit(1)
                    HStack(spacing: 6) {
                        if let plan = account.plan?.label { Text(plan) }
                        if let n = store.sessions[account.number]?.count, n > 0 {
                            Text(n == 1 ? "1 running" : "\(n) running").foregroundStyle(Brand.cyanText)
                        }
                        if let wake = store.wake?.account(account.number)?.summary() { Text(wake) }
                    }
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            if account.usage != nil {
                HStack(alignment: .lastTextBaseline) {
                    Text(five?.pct.map { "\(Int($0))%" } ?? "—")
                        .font(Brand.display(44, relativeTo: .largeTitle).monospacedDigit())
                        .foregroundStyle(five?.pct == nil ? .tertiary : .primary)
                    Text("of 5-hour window").font(.callout).foregroundStyle(.secondary)
                    Spacer(minLength: 8)
                    VStack(alignment: .trailing, spacing: 0) {
                        Text(five?.clock ?? "—").font(Brand.display(18, weight: .medium, relativeTo: .title3).monospacedDigit())
                        if let c = five?.countdown { Text("reset in \(c)").font(.caption).foregroundStyle(.secondary) }
                    }
                }
                .accessibilityElement(children: .combine)
                TideStaff(window: five)
                HStack {
                    paceLine(five)
                    Spacer()
                    Text("7d \(account.usage?.sevenDay?.pct.map { "\(Int($0))%" } ?? "—")")
                        .monospacedDigit().foregroundStyle(.secondary)
                }
                .font(.caption)
                TimeLeftRule(resetsAt: five?.resetsAt, windowSeconds: Self.fiveHours)
            } else if let note = account.statusNote {
                Text(note).font(.callout).foregroundStyle(Brand.warn)
            }
            if account.offersSignIn { SignInAgainButton(account: account, prominent: account.needsRelogin).controlSize(.small) }
        }
        .contentShape(.rect)
        .onTapGesture { AccountActionMenu.show(for: account, store: store) }
        .help("Click for account actions")
        .contextMenu { AccountContextMenu(account: account) }
    }

    @ViewBuilder private func paceLine(_ w: UsageWindow?) -> some View {
        if let d = w?.paceDelta, d != 0 {
            Text(d > 0 ? "\(d) pts ahead of pace" : "\(-d) pts under pace")
                .foregroundStyle(d > 0 ? Brand.warn : .secondary)
        } else if let w = account.usage?.sevenDay, w.aheadOfPace == true, w.willLastToReset == false,
                  let end = w.projectedExhaustionAt {
            Text("Weekly runs out \(end.formatted(.relative(presentation: .named)))").foregroundStyle(Brand.warn)
        } else {
            Text("on pace").foregroundStyle(.secondary)
        }
    }
}

/// The other accounts as an almanac: in switch order, columns that never move.
private struct NextInLine: View {
    let accounts: [Account]
    private static let rowHeight: CGFloat = 30

    var body: some View {
        ScrollView {
            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 0) {
                GridRow {
                    Text("Next in line")
                    Text("5h").gridColumnAlignment(.trailing)
                    Text("7d").gridColumnAlignment(.trailing)
                    Text("Resets").gridColumnAlignment(.trailing)
                    Color.clear.frame(width: 1, height: 1)
                }
                .font(.caption2.weight(.medium)).foregroundStyle(.tertiary)
                .padding(.bottom, 2)
                .accessibilityHidden(true)
                ForEach(accounts) { AccountRow(account: $0) }
            }
        }
        .scrollIndicators(.never)
        .scrollBounceBehavior(.basedOnSize)
        .frame(maxHeight: min(CGFloat(accounts.count + 1) * Self.rowHeight, 300))
    }
}

struct AccountRow: View {
    @Environment(AppStore.self) private var store
    let account: Account
    @State private var hovering = false

    var body: some View {
        GridRow {
            HStack(spacing: 6) {
                MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 18,
                           phase: store.phase(for: account))
                Text(account.displayName).lineLimit(1).truncationMode(.middle)
                if let plan = account.plan?.label { Text(plan).font(.caption2).foregroundStyle(.tertiary) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if account.usage != nil {
                pct(account.usage?.fiveHour)
                pct(account.usage?.sevenDay)
                Text(account.usage?.fiveHour?.clock ?? (account.usage?.fiveHour?.pct == 0 ? "not started" : "—"))
                    .foregroundStyle(.secondary)
                    .gridColumnAlignment(.trailing)
            } else {
                Text(account.isDisabled ? "disabled" : (account.statusNote ?? "—"))
                    .foregroundStyle(account.needsRelogin ? Brand.warn : .secondary)
                    .lineLimit(1)
                    .gridCellColumns(3)
            }
            trailing
        }
        .font(.callout.monospacedDigit())
        .opacity(account.isDisabled ? 0.5 : 1)
        .padding(.vertical, 5)
        .background(hovering ? AnyShapeStyle(.quaternary.opacity(0.5)) : AnyShapeStyle(.clear))
        .contentShape(.rect)
        .onTapGesture { AccountActionMenu.show(for: account, store: store) }
        .onHover { hovering = $0 }
        .contextMenu { AccountContextMenu(account: account) }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }

    private func pct(_ w: UsageWindow?) -> some View {
        Text(w?.pct.map { "\(Int($0))%" } ?? "—")
            .foregroundStyle(w?.pct.map { $0 >= 85 } == true ? Brand.usageFill(w) : .primary)
            .gridColumnAlignment(.trailing)
    }

    @ViewBuilder private var trailing: some View {
        if account.needsRelogin && account.canRelogin {
            SignInAgainButton(account: account, prominent: false).controlSize(.mini)
        } else {
            Button("Use") { Task { await store.switchTo(account) } }
                .buttonStyle(.borderless)
                .font(.caption.weight(.medium))
                .opacity(hovering ? 1 : 0)
                .disabled(store.busy != nil || account.isDisabled)
                .help("Switch to \(account.displayName)")
        }
    }
}

/// A section heading in the menu bar panel: a small label with its actions on the right.
private struct PanelSectionHeader<Accessory: View>: View {
    let title: String
    let systemImage: String
    @ViewBuilder let accessory: Accessory

    var body: some View {
        HStack(spacing: 6) {
            Label(title, systemImage: systemImage)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .accessibilityAddTraits(.isHeader)
            Spacer()
            accessory
        }
        .padding(.horizontal, 4)
    }
}

private struct ErrorCard: View {
    let message: String
    var body: some View {
        Label(message, systemImage: "exclamationmark.triangle")
            .font(.callout)
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassEffect(.regular.tint(.red.opacity(0.1)), in: .rect(cornerRadius: 16))
    }
}

/// Switch / auto-switch / terminal shortcuts.
private struct QuickActions: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        HStack(spacing: 8) {
            let next = store.nextInLine.first(where: { !$0.isDisabled && !$0.needsRelogin })
            HStack(spacing: 4) {
                Button {
                    if let next { Task { await store.switchTo(next) } } else { Task { await store.switchNext(.best) } }
                } label: {
                    Text(next.map { "Switch to \($0.displayName)" } ?? "Switch")
                        .lineLimit(1).truncationMode(.middle)
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.glassProminent)
                .help("Switch to the next account in line")
                Menu {
                    ForEach(AppStore.SwitchMode.allCases) { mode in
                        Button(mode.title) { Task { await store.switchNext(mode) } }
                    }
                } label: { Image(systemName: "chevron.down") }
                    .menuIndicator(.hidden)
                    .buttonStyle(.glass)
                    .fixedSize()
                    .help("Other ways to switch")
                    .accessibilityLabel("Other ways to switch")
            }
            .disabled(store.busy != nil)

            Menu {
                Section("New shell") {
                    TerminalMenuItems { store.openTerminal($0) }
                }
                Section("Claude (active account)") {
                    TerminalMenuItems { store.runClaude(as: nil, app: $0) }
                }
                Menu("Claude as…") {
                    ForEach(store.accounts) { a in
                        Button("\(a.number): \(a.displayName)") { store.runClaude(as: a) }
                    }
                }
                Button("Claude in Folder…") {
                    if let dir = AppStore.chooseFolder(prompt: "Run Claude Here") { store.runClaude(as: nil, folder: dir) }
                }
                if !store.mappings.isEmpty {
                    Section("Mapped folders") {
                        ForEach(store.mappings) { m in
                            Button((m.path as NSString).abbreviatingWithTildeInPath) { store.runMapped(m) }
                        }
                    }
                }
                SignInMenuSection()
            } label: {
                Label("Terminal", systemImage: "terminal")
            } primaryAction: {
                store.openTerminal()
            }
            .buttonStyle(.glass)
            .help("Click: new \(TerminalApp.preferred.title) window")
        }
        .menuStyle(.button)
        .labelStyle(.titleAndIcon)
        .controlSize(.large)
        .frame(maxWidth: .infinity)
    }
}

/// Sign-in entries for a dropdown: accounts that need a re-login first, then every other
/// account that has a login to renew, then a new account.
struct SignInMenuSection: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let renewable = store.accounts.filter(\.canRelogin)
        let needed = renewable.filter(\.offersSignIn)
        Section("Sign in") {
            ForEach(needed) { a in
                Button("Sign In Again: \(a.number): \(a.displayName)\(a.needsRelogin ? " — re-login needed" : " — login expiring")") {
                    store.signInAgain(a)
                }
                .disabled(store.loginPending(for: a))
            }
            Menu("Sign In Again…") {
                ForEach(renewable) { a in
                    Button("\(a.number): \(a.displayName)") { store.signInAgain(a) }
                        .disabled(store.loginPending(for: a))
                }
            }
            .disabled(renewable.isEmpty)
            Button("Sign In New Account…") { store.signInNew() }
                .disabled(store.addLoginPending)
        }
    }
}
