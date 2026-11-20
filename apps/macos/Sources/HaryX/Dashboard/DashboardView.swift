import HaryXCore
import SwiftUI

enum DashboardSection: String, CaseIterable, Identifiable {
    case accounts, autoSwitch, mappings, config, maintenance, preferences
    var id: String { rawValue }

    var title: String {
        switch self {
        case .accounts: "Accounts"
        case .autoSwitch: "Auto-Switch"
        case .mappings: "Folder Mappings"
        case .config: "CLI Settings"
        case .maintenance: "Backup & Maintenance"
        case .preferences: "App Preferences"
        }
    }

    var icon: String {
        switch self {
        case .accounts: "person.2.fill"
        case .autoSwitch: "arrow.triangle.2.circlepath"
        case .mappings: "folder.badge.person.crop"
        case .config: "slider.horizontal.3"
        case .maintenance: "externaldrive.badge.timemachine"
        case .preferences: "gearshape"
        }
    }
}

/// Lets the menu bar deep-link into the dashboard.
@MainActor @Observable
final class DashboardRouter {
    static let shared = DashboardRouter()
    var section: DashboardSection? = .accounts
    var selectedSlot: Int?

    func select(_ slot: Int) {
        section = .accounts
        selectedSlot = slot
    }

    /// Opens the dashboard on this account's detail inspector.
    func reveal(_ slot: Int) {
        select(slot)
        DashboardWindow.shared.show()
    }
}

struct DashboardView: View {
    @Environment(AppStore.self) private var store
    @Bindable private var router = DashboardRouter.shared

    var body: some View {
        NavigationSplitView {
            List(DashboardSection.allCases, selection: $router.section) { section in
                Label(section.title, systemImage: section.icon).tag(section)
            }
            .navigationSplitViewColumnWidth(min: 190, ideal: 210)
        } detail: {
            Group {
                switch router.section ?? .accounts {
                case .accounts: AccountsPane()
                case .autoSwitch: AutoSwitchPane()
                case .mappings: MappingsPane()
                case .config: ConfigPane()
                case .maintenance: MaintenancePane()
                case .preferences: PreferencesPane()
                }
            }
            .navigationTitle((router.section ?? .accounts).title)
        }
        .overlay(alignment: .bottom) {
            if let banner = store.banner {
                BannerView(banner: banner).frame(maxWidth: 520).padding()
            }
        }
        .animation(.snappy, value: store.banner)
        .frame(minWidth: 860, minHeight: 560)
        .task { await store.loadExtras() }
    }
}

struct AccountsPane: View {
    @Environment(AppStore.self) private var store
    @Bindable private var router = DashboardRouter.shared
    @State private var addSheet: AddAccountSheet.Mode?

    var body: some View {
        ScrollView {
            if store.accounts.isEmpty {
                ContentUnavailableView("No accounts yet", systemImage: "person.crop.circle.badge.plus",
                                       description: Text(store.refreshError ?? "Log in with Claude Code, then add the current login."))
                    .padding(.top, 80)
            }
            GlassEffectContainer(spacing: 16) {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 250), spacing: 16)], spacing: 16) {
                    ForEach(store.accounts) { a in
                        AccountCard(account: a, selected: router.selectedSlot == a.number)
                            .onTapGesture { router.selectedSlot = a.number }
                    }
                }
                .padding(20)
            }
        }
        .inspector(isPresented: Binding(get: { router.selectedSlot != nil }, set: { if !$0 { router.selectedSlot = nil } })) {
            if let slot = router.selectedSlot, let a = store.account(slot: slot) {
                AccountInspector(account: a).inspectorColumnWidth(min: 300, ideal: 340, max: 420)
            } else {
                ContentUnavailableView("Select an account", systemImage: "person.crop.square")
            }
        }
        .toolbar {
            ToolbarItemGroup {
                Menu {
                    Button("Add Current Claude Login…") { addSheet = .current }
                    Button("Add Setup Token / API Key…") { addSheet = .token }
                } label: { Label("Add", systemImage: "plus") }
                Menu {
                    ForEach(AppStore.SwitchMode.allCases) { mode in
                        Button(mode.title) { Task { await store.switchNext(mode) } }
                    }
                } label: { Label("Switch", systemImage: "arrow.left.arrow.right") }
                Button { Task { await store.refresh(); await store.loadExtras() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }
            }
        }
        .sheet(item: $addSheet) { AddAccountSheet(mode: $0) }
    }
}

struct AccountCard: View {
    @Environment(AppStore.self) private var store
    let account: Account
    let selected: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                MascotView(style: store.style(for: account), mood: store.mood(for: account), size: 72, phase: store.phase(for: account))
                    .opacity(account.isDisabled ? 0.55 : 1)
                VStack(alignment: .leading, spacing: 3) {
                    Text("Slot \(account.number)").font(.caption2.monospacedDigit()).foregroundStyle(.tertiary)
                    Text(account.displayName).font(.headline).lineLimit(1)
                    if account.alias != nil { Text(account.email).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                    if let org = account.organizationName { Text(org).font(.caption2).foregroundStyle(.secondary).lineLimit(1) }
                    AccountBadges(account: account)
                }
                Spacer(minLength: 0)
            }
            if account.usage != nil {
                if account.usage?.fiveHour != nil { UsageBar(label: "5-HOUR", window: account.usage?.fiveHour) }
                UsageBar(label: "WEEKLY", window: account.usage?.sevenDay)
            }
            if let note = account.statusNote { Text(note).font(.caption).foregroundStyle(.orange) }
            HStack {
                if account.active {
                    Label("In use", systemImage: "checkmark.circle.fill").font(.caption).foregroundStyle(.tint)
                } else {
                    Button("Switch") { Task { await store.switchTo(account) } }.buttonStyle(.glassProminent)
                }
                Spacer()
                Menu {
                    TerminalMenuItems { store.runClaude(as: account, app: $0) }
                } label: { Image(systemName: "terminal") } primaryAction: {
                    store.runClaude(as: account)
                }
                .menuStyle(.button).buttonStyle(.glass).fixedSize()
                .help("Run Claude as this account in a new terminal")
            }
            .controlSize(.small)
        }
        .padding(14)
        .glassEffect(.regular.tint(selected ? Color.accentColor.opacity(0.18) : .clear).interactive(), in: .rect(cornerRadius: 20))
        .overlay { if selected { RoundedRectangle(cornerRadius: 20).strokeBorder(.tint, lineWidth: 1.5) } }
        .contextMenu { AccountContextMenu(account: account) }
    }
}
