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
                if store.claudeSetup?.ready == false { ClaudeSetupBanner(compact: true).padding(.bottom, 8) }
                if let active = store.active {
                    ActiveTide(account: active)
                } else if let error = store.refreshError {
                    ErrorCard(message: error)
                }
                if let line = store.desktopLine { DesktopStatusLine(line: line).padding(.top, 8) }
                StuckSessionsLine()
                UpdateLine(updates: store.updates)
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
            guard shown else { return }
            Task { await store.reloadWake() }
            Task { await store.reloadDesktop() }
            if (store.lastRefresh?.timeIntervalSinceNow ?? -999) < -15 { Task { await store.refresh() } }
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
                store.toggleAuto()
            } label: {
                Text(store.auto.isRunning ? "Auto-switch on" : "Auto-switch off")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(store.auto.isRunning ? Brand.cyanText : .secondary)
            }
            .help(store.auto.isRunning ? "Stop auto-switch" : "Start auto-switch")
            Group {
                Button { Task { await store.refresh() } } label: {
                    Image(systemName: "arrow.clockwise").spinning(store.isRefreshing)
                }
                    .disabled(store.isRefreshing)
                    .help(store.isRefreshing ? "Refreshing…" : "Refresh")
                    .keyboardShortcut("r")
                    .accessibilityLabel("Refresh")
                Button { store.signInNew() } label: { Image(systemName: "plus") }
                    .disabled(store.addLoginPending)
                    .help(store.addLoginPending ? "A new-account sign-in is already open" : "Sign in a new account")
                    .accessibilityLabel("Sign in a new account")
                Menu {
                    Button("Open Dashboard") { DashboardWindow.shared.show() }.keyboardShortcut("d")
                    Section("Switch") {
                        ForEach(AppStore.SwitchMode.allCases) { mode in
                            Button(mode.title) { Task { await store.switchNext(mode) } }
                        }
                    }
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

    var body: some View {
        let five = account.usage?.fiveHour?.withPace(windowSeconds: UsageWindow.fiveHourSeconds)
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 36,
                           phase: store.phase(for: account))
                VStack(alignment: .leading, spacing: 2) {
                    Text(account.displayName).font(.headline).lineLimit(1)
                    HStack(spacing: 6) {
                        if let plan = account.plan?.label { Text(plan) }
                        if let n = store.sessions[account.number]?.count, n > 0 {
                            HStack(spacing: 4) {
                                Circle().fill(Brand.cyan).frame(width: 6, height: 6)
                                Text(n == 1 ? "1 running" : "\(n) running")
                            }
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
                paceLine(five).font(.caption)
                WeekStaff(window: account.usage?.sevenDay).padding(.top, 2)
            } else if let note = account.statusNote {
                Text(note).font(.callout).foregroundStyle(Brand.warn)
            }
            if account.offersSignIn { SignInAgainButton(account: account, prominent: account.needsRelogin).controlSize(.small) }
        }
        .contentShape(.rect)
        .onTapGesture { AccountActionMenu.show(for: account, store: store) }
        .help("\(account.fullIdentity)\nClick for account actions")
        .contextMenu { AccountContextMenu(account: account) }
    }

    @ViewBuilder private func paceLine(_ w: UsageWindow?) -> some View {
        if let d = w?.paceDelta, abs(d) >= UsageWindow.paceThreshold {
            Text(d > 0 ? "\(d) pts ahead of pace" : "\(-d) pts under pace")
                .foregroundStyle(d > 0 ? Brand.warn : .secondary)
        } else if let w = account.usage?.sevenDay, w.aheadOfPace == true, w.willLastToReset == false,
                  let end = w.projectedExhaustionAt {
            Text("Weekly runs out \(end.formatted(.relative(presentation: .named)))").foregroundStyle(Brand.warn)
        } else if w?.paceDelta != nil {
            Text("on pace").foregroundStyle(.secondary)
        }
    }
}

/// The other accounts as an almanac: in switch order, columns that never move.
private struct NextInLine: View {
    @Environment(AppStore.self) private var store
    let accounts: [Account]
    private static let rowHeight: CGFloat = 30
    /// About two and a half rows: the cut-off row shows there is more to scroll to.
    private static let maxListHeight: CGFloat = 140
    static let space = "nextInLine"
    /// Rows sit inside the hover fill, not flush with its edges.
    private static let rowInset: CGFloat = 8
    /// Modifiers on a GridRow apply to each cell, so a per-row hover fill leaves the column gaps
    /// bare. The grid draws one full-width fill instead, from each row's measured band.
    @State private var rowBands: [Int: CGRect] = [:]
    @State private var pointer: CGPoint?
    /// The grid's height, so a hovered row near the bottom shows its email above instead.
    @State private var gridHeight: CGFloat = 0
    /// The row mascot (18 pt) plus its spacing: where a row's name text starts.
    static let nameOffset: CGFloat = 24
    /// The email label's height (caption text plus padding) and its gap from the row.
    private static let labelHeight: CGFloat = 22
    private static let labelGap: CGFloat = 2

    private var hoveredSlot: Int? {
        guard let pointer else { return nil }
        return rowBands.first { $0.value.minY <= pointer.y && pointer.y < $0.value.maxY }?.key
    }

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
                ForEach(accounts) { a in
                    AccountRow(account: a) { band in rowBands[a.number] = band }
                }
            }
            .padding(.horizontal, Self.rowInset)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { gridHeight = $0 }
            .coordinateSpace(.named(Self.space))
            .background(alignment: .topLeading) { hoverFill }
            .overlay(alignment: .topLeading) { hoveredEmail }
            .contentShape(.rect)
            .onContinuousHover(coordinateSpace: .named(Self.space)) { phase in
                if case .active(let p) = phase { pointer = p } else { pointer = nil }
            }
            .onTapGesture {
                if let slot = hoveredSlot, let a = accounts.first(where: { $0.number == slot }) {
                    AccountActionMenu.show(for: a, store: store)
                }
            }
        }
        .scrollIndicators(.automatic)
        .scrollBounceBehavior(.basedOnSize)
        // Rows are not all one line tall, so size to the measured grid; estimate until measured.
        .frame(height: min(gridHeight > 0 ? gridHeight : CGFloat(accounts.count + 1) * Self.rowHeight,
                           Self.maxListHeight))
    }

    @ViewBuilder private var hoverFill: some View {
        if let slot = hoveredSlot, let band = rowBands[slot] {
            RoundedRectangle(cornerRadius: 6)
                .fill(.quaternary.opacity(0.5))
                .frame(maxWidth: .infinity)
                .frame(height: band.height)
                .offset(y: band.minY)
                .allowsHitTesting(false)
        }
    }

    /// The hovered row's full email in a small label just below the row (above it near the
    /// bottom of the list), shown at once instead of after the system tooltip delay.
    @ViewBuilder private var hoveredEmail: some View {
        if let slot = hoveredSlot, let band = rowBands[slot],
           let account = accounts.first(where: { $0.number == slot }), !account.showsSignInRow {
            let fitsBelow = band.maxY + Self.labelGap + Self.labelHeight <= gridHeight
            Text(account.email)
                .font(.caption)
                .lineLimit(1)
                .padding(.horizontal, 8)
                .frame(height: Self.labelHeight)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
                .shadow(color: .black.opacity(0.2), radius: 4, y: 1)
                .fixedSize()
                .offset(x: band.minX + Self.nameOffset,
                        y: fitsBelow ? band.maxY + Self.labelGap : band.minY - Self.labelGap - Self.labelHeight)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }
}

struct AccountRow: View {
    @Environment(AppStore.self) private var store
    let account: Account
    /// Reports the row's vertical band (from its tallest cell) in the grid's coordinate space.
    let onBand: (CGRect) -> Void

    var body: some View {
        Group {
            if account.showsSignInRow { signInRow } else { usageRow }
        }
        .font(.callout.monospacedDigit())
        .opacity(account.isDisabled ? 0.5 : 1)
        .contextMenu { AccountContextMenu(account: account) }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }

    private var usageRow: some View {
        GridRow {
            identity
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 5)
                .onGeometryChange(for: CGRect.self) { $0.frame(in: .named(NextInLine.space)) } action: { onBand($0) }
            usageCells
            trailing
        }
    }

    /// One cell across every column: name over email, then Sign In. It has no usage to show, and
    /// a single spanning cell leaves the column widths to the rows that do.
    private var signInRow: some View {
        GridRow {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 1) {
                    identity
                    // The name is the email unless the account has an alias; then name both.
                    if account.alias != nil {
                        Text(account.email)
                            .font(.caption).foregroundStyle(.secondary)
                            .lineLimit(1).truncationMode(.middle)
                            .padding(.leading, NextInLine.nameOffset)
                    }
                    Label(account.signInReason, systemImage: "exclamationmark.triangle.fill")
                        .labelStyle(.titleAndIcon)
                        .imageScale(.small)
                        .font(.caption).foregroundStyle(Brand.warn)
                        .lineLimit(1).truncationMode(.tail)
                        .padding(.leading, NextInLine.nameOffset)
                }
                Spacer(minLength: 0)
                CompactSignInButton(account: account)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 5)
            .onGeometryChange(for: CGRect.self) { $0.frame(in: .named(NextInLine.space)) } action: { onBand($0) }
            .gridCellColumns(5)
        }
    }

    private var identity: some View {
        HStack(spacing: 6) {
            MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 18,
                       phase: store.phase(for: account))
            Text(account.displayName).lineLimit(1).truncationMode(.middle)
            if let plan = account.plan?.label { Text(plan).font(.caption2).foregroundStyle(.tertiary) }
        }
    }

    /// The 5h, 7d and reset columns, or the account's status across them.
    @ViewBuilder private var usageCells: some View {
        if account.usage != nil {
            pct(account.usage?.fiveHour)
            pct(account.usage?.sevenDay)
            Text(resetText)
                .foregroundStyle(.secondary)
                .gridColumnAlignment(.trailing)
        } else {
            Text(account.isDisabled ? "disabled" : (account.statusNote ?? "—"))
                .foregroundStyle(account.needsRelogin ? Brand.warn : .secondary)
                .lineLimit(1)
                .gridCellColumns(3)
        }
    }

    /// The 5-hour reset clock; a window a wake started can read 0% with no reset, so the wake's
    /// own reset fills in before falling back to "not started".
    private var resetText: String {
        let five = account.usage?.fiveHour
        if let clock = five?.clock { return clock }
        if let reset = store.wake?.account(account.number)?.windowReset() {
            return reset.formatted(.dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits))
        }
        return five?.pct == 0 ? "not started" : "—"
    }

    private func pct(_ w: UsageWindow?) -> some View {
        Text(w?.pct.map { "\(Int($0))%" } ?? "—")
            .foregroundStyle(w?.pct.map { $0 >= 85 } == true ? Brand.usageFill(w) : .primary)
            .gridColumnAlignment(.trailing)
    }

    private var trailing: some View {
        // Not a glass style: glass draws outside the scroll view's clip, over the panel's buttons.
        Button("Use") { Task { await store.switchTo(account) } }
            .buttonStyle(.borderedProminent)
            .controlSize(.mini)
            .font(.caption.weight(.semibold))
            .disabled(store.busy != nil || account.isDisabled)
            .help("Switch to \(account.displayName)")
    }
}

/// Which account Claude Desktop is on, with one button when it is not on the active account.
private struct DesktopStatusLine: View {
    @Environment(AppStore.self) private var store
    let line: DesktopLine

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "macwindow").accessibilityHidden(true)
            Text(line.text).lineLimit(1).truncationMode(.middle)
            Spacer(minLength: 4)
            if let action = line.action {
                Button(action.title) { Task { await store.openDesktopAsActive() } }
                    .buttonStyle(.glass)
                    .controlSize(.mini)
                    .disabled(store.busy != nil)
                    .help(action == .setUp
                        ? "Open Claude Desktop on a new folder for the active account, to sign in once"
                        : "Open Claude Desktop as the active account")
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
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
            Button {
                if let next { Task { await store.switchTo(next) } } else { Task { await store.switchNext(.best) } }
            } label: {
                Text(next.map { "Switch to \($0.shortName)" } ?? "Switch")
                    .lineLimit(1).truncationMode(.tail)
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.glassProminent)
            .disabled(store.busy != nil)
            .help(next.map { "Switch to \($0.email)" } ?? "Switch to the account with most headroom")

            Menu {
                ForEach(DashboardGroup.allCases) { group in
                    Section(group.title) {
                        ForEach(group.sections) { section in
                            Button(section.title, systemImage: section.icon) { DashboardRouter.shared.open(section) }
                        }
                    }
                }
            } label: {
                Label("Dashboard", systemImage: "gauge.with.dots.needle.50percent")
            } primaryAction: {
                DashboardWindow.shared.show()
            }
            .buttonStyle(.glass)
            .labelStyle(.iconOnly)
            .fixedSize()
            .help("Click: open the dashboard (⌘D). Arrow: jump to a section")
            .accessibilityLabel("Open the dashboard")

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
    /// Off where the menu already offers its own "Sign In New Account…" (the dashboard's add menu).
    var showsNewAccount = true

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
            if showsNewAccount {
                Button("Sign In New Account…") { store.signInNew() }
                    .disabled(store.addLoginPending)
            }
        }
    }
}

private extension Account {
    /// The alias and email together (or just the email), for hover help where names are cut short.
    var fullIdentity: String { alias.map { "\($0) — \(email)" } ?? email }
}

/// "Sign In" sized like the row's "Use" button, so it does not widen the last column; a bare
/// spinner while that account's sign-in is open.
private struct CompactSignInButton: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        if store.loginPending(for: account) {
            ProgressView()
                .controlSize(.mini)
                .help("Waiting for sign-in: finish it in its window and your browser")
                .accessibilityLabel("Waiting for sign-in")
        } else {
            Button("Sign In") { store.signInAgain(account) }
                .buttonStyle(.borderedProminent).tint(.orange)
                .controlSize(.mini)
                .font(.caption.weight(.semibold))
                .help("Sign in again as \(account.email)")
        }
    }
}

extension Account {
    /// The menu bar row shows email and Sign In in place of usage.
    fileprivate var showsSignInRow: Bool { needsRelogin && canRelogin }

    /// Why the row asks for a sign-in, short enough for one line under the name.
    fileprivate var signInReason: String {
        switch usageStatus {
        case "no_credentials": "No saved login: sign in again"
        case "foreign_credential": "Saved login is another account's: sign in again"
        default: "Login expired: sign in again"
        }
    }
}
