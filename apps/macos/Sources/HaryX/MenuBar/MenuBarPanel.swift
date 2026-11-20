import HaryXCore
import SwiftUI

struct MenuBarPanel: View {
    @Environment(AppStore.self) private var store
    static let width: CGFloat = 372

    var body: some View {
        GlassEffectContainer(spacing: 12) {
            VStack(spacing: 10) {
                header
                if let active = store.active {
                    HeroCard(account: active)
                } else if let error = store.refreshError {
                    ErrorCard(message: error)
                }
                accountList
                QuickActions()
                if let banner = store.banner { BannerView(banner: banner) }
                footer
            }
            .padding(12)
        }
        .frame(width: Self.width)
        .environment(\.mascotsPaused, !store.panelVisible)
        .animation(.snappy, value: store.banner)
        .onChange(of: store.panelVisible) { _, shown in
            if shown, (store.lastRefresh?.timeIntervalSinceNow ?? -999) < -15 { Task { await store.refresh() } }
        }
    }

    private var header: some View {
        HStack {
            Text("HaryX").font(.headline)
            if let busy = store.busy {
                ProgressView().controlSize(.mini)
                Text(busy).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if store.auto.isRunning {
                Label("Auto", systemImage: "arrow.triangle.2.circlepath")
                    .font(.caption2.weight(.semibold)).foregroundStyle(.green)
            }
            Button { Task { await store.refresh() } } label: { Image(systemName: "arrow.clockwise") }
                .buttonStyle(.borderless).help("Refresh")
                .keyboardShortcut("r")
        }
    }

    private var accountList: some View {
        ScrollView {
            VStack(spacing: 4) {
                ForEach(store.accounts.filter { !$0.active }) { AccountRow(account: $0) }
            }
        }
        .scrollIndicators(.never)
        .frame(maxHeight: min(CGFloat(max(store.accounts.count - 1, 0)) * 64, 360))
    }

    private var footer: some View {
        HStack {
            Button("Dashboard") { DashboardWindow.shared.show() }
            .keyboardShortcut("d")
            Spacer()
            if let t = store.lastRefresh {
                Text(t, format: .relative(presentation: .numeric)).font(.caption2).foregroundStyle(.tertiary)
            }
            Spacer()
            Button("Quit") { NSApp.terminate(nil) }.keyboardShortcut("q")
        }
        .buttonStyle(.borderless)
        .font(.caption)
    }
}

private struct HeroCard: View {
    @Environment(AppStore.self) private var store
    let account: Account

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 64, phase: store.phase(for: account))
            VStack(alignment: .leading, spacing: 6) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(account.displayName).font(.headline).lineLimit(1)
                    if account.alias != nil { Text(account.email).font(.caption2).foregroundStyle(.secondary).lineLimit(1) }
                    AccountBadges(account: account)
                }
                if account.usage != nil {
                    if account.usage?.fiveHour != nil { UsageBar(label: "5-HOUR", window: account.usage?.fiveHour) }
                    UsageBar(label: "WEEKLY", window: account.usage?.sevenDay)
                    if let w = account.usage?.sevenDay, w.aheadOfPace == true, w.willLastToReset == false, let end = w.projectedExhaustionAt {
                        Label("On pace to run out \(end.formatted(.relative(presentation: .named)))", systemImage: "gauge.with.dots.needle.67percent")
                            .font(.caption2).foregroundStyle(.orange)
                    }
                } else if let note = account.statusNote {
                    Text(note).font(.caption).foregroundStyle(.orange)
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassEffect(.regular.tint(.accentColor.opacity(0.08)), in: .rect(cornerRadius: 18))
        .contentShape(.rect(cornerRadius: 18))
        .onTapGesture { AccountActionMenu.show(for: account, store: store) }
        .help("Click for account actions")
        .contextMenu { AccountContextMenu(account: account) }
    }
}

struct AccountRow: View {
    @Environment(AppStore.self) private var store
    let account: Account
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 10) {
            MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 34, phase: store.phase(for: account))
                .opacity(account.isDisabled ? 0.55 : 1)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 4) {
                    Text("\(account.number)").font(.caption2.monospacedDigit()).foregroundStyle(.tertiary)
                    Text(account.displayName).font(.callout.weight(.medium)).lineLimit(1)
                    AccountBadges(account: account)
                }
                if account.usage != nil {
                    HStack(spacing: 8) {
                        UsageBar(label: "5h", window: account.usage?.fiveHour, compact: true)
                        UsageBar(label: "7d", window: account.usage?.sevenDay, compact: true)
                    }
                } else if let note = account.statusNote {
                    Text(note).font(.caption2).foregroundStyle(.orange).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            Button("Use") { Task { await store.switchTo(account) } }
                .buttonStyle(.glass)
                .controlSize(.small)
                .opacity(hovering ? 1 : 0.55)
                .disabled(store.busy != nil)
                .help("Switch to \(account.displayName)")
        }
        .padding(.horizontal, 8).padding(.vertical, 6)
        .background(hovering ? AnyShapeStyle(.quaternary.opacity(0.5)) : AnyShapeStyle(.clear), in: .rect(cornerRadius: 10))
        .contentShape(.rect)
        .onTapGesture { AccountActionMenu.show(for: account, store: store) }
        .onHover { hovering = $0 }
        .contextMenu { AccountContextMenu(account: account) }
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
            Menu {
                ForEach(AppStore.SwitchMode.allCases) { mode in
                    Button(mode.title) { Task { await store.switchNext(mode) } }
                }
            } label: {
                Label("Switch", systemImage: "arrow.left.arrow.right")
            } primaryAction: {
                Task { await store.switchNext(.best) }
            }
            .help("Click: switch to the account with most headroom")

            Button {
                store.auto.isRunning ? store.auto.stop() : store.startAuto()
            } label: {
                Label("Auto", systemImage: store.auto.isRunning ? "pause.fill" : "play.fill")
            }
            .tint(store.auto.isRunning ? .green : nil)
            .help(store.auto.isRunning ? "Stop auto-switch" : "Start auto-switch")

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
            } label: {
                Label("Terminal", systemImage: "terminal")
            } primaryAction: {
                store.openTerminal()
            }
            .help("Click: new \(TerminalApp.preferred.title) window")
        }
        .buttonStyle(.glass)
        .menuStyle(.button)
        .labelStyle(.titleAndIcon)
        .controlSize(.regular)
        .frame(maxWidth: .infinity)
    }
}
