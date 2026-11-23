import HaryXCore
import SwiftUI

/// Sidebar groups. AI holds everything about AI coding accounts (the `haryx ai` commands); later
/// sections (DevSecOps, SRE, Cloud…) become siblings of it. App holds the app's own settings.
enum DashboardGroup: String, CaseIterable, Identifiable {
    case ai, app
    var id: String { rawValue }

    var title: String {
        switch self {
        case .ai: "AI"
        case .app: "HaryX"
        }
    }

    var sections: [DashboardSection] { DashboardSection.allCases.filter { $0.group == self } }
}

enum DashboardSection: String, CaseIterable, Identifiable {
    case accounts, autoSwitch, wake, mappings, profiles, config, maintenance, preferences
    var id: String { rawValue }

    var group: DashboardGroup {
        switch self {
        case .accounts, .autoSwitch, .wake, .mappings, .profiles, .config, .maintenance: .ai
        case .preferences: .app
        }
    }

    var title: String {
        switch self {
        case .accounts: "Accounts"
        case .autoSwitch: "Auto-Switch"
        case .wake: "Wake"
        case .mappings: "Folder Mappings"
        case .profiles: "Setup Profiles"
        case .config: "CLI Settings"
        case .maintenance: "Backup & Maintenance"
        case .preferences: "App Preferences"
        }
    }

    var icon: String {
        switch self {
        case .accounts: "person.2.fill"
        case .autoSwitch: "arrow.triangle.2.circlepath"
        case .wake: "alarm"
        case .mappings: "folder.badge.person.crop"
        case .profiles: "shippingbox"
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

    /// Opens the dashboard on a section.
    func open(_ section: DashboardSection) {
        self.section = section
        DashboardWindow.shared.show()
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
            List(selection: $router.section) {
                ForEach(DashboardGroup.allCases) { group in
                    Section(group.title) {
                        ForEach(group.sections) { section in
                            Label(section.title, systemImage: section.icon).tag(section)
                        }
                    }
                }
            }
            .safeAreaInset(edge: .top) {
                BrandLockup(size: 20).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 16).padding(.vertical, 6)
            }
            .navigationSplitViewColumnWidth(min: 190, ideal: 210)
        } detail: {
            Group {
                switch router.section ?? .accounts {
                case .accounts: AccountsPane()
                case .autoSwitch: AutoSwitchPane()
                case .wake: WakePane()
                case .mappings: MappingsPane()
                case .profiles: ProfilesPane()
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
        .environment(\.mascotsPaused, !store.dashboardVisible)
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
                ContentUnavailableView {
                    Label("No accounts yet", systemImage: "person.crop.circle.badge.plus")
                } description: {
                    Text(store.refreshError ?? "Sign in with your Claude account to add it.")
                } actions: {
                    Button("Sign In…") { store.signInNew() }
                        .buttonStyle(.glassProminent)
                        .disabled(store.addLoginPending)
                    if store.canImportFromCswap {
                        Button("Import Accounts from cswap") { Task { await store.importFromCswap() } }
                            .buttonStyle(.glassProminent)
                    }
                }
                .padding(.top, 80)
            }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 250), spacing: 16)], spacing: 16) {
                    ForEach(store.active.map { [$0] + store.nextInLine } ?? store.nextInLine) { a in
                        AccountCard(account: a, selected: router.selectedSlot == a.number)
                            .onTapGesture { router.selectedSlot = a.number }
                    }
                }
                .padding(20)
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
                    Button("Sign In New Account…") { addSheet = .login }
                    Button("Add Current Claude Login…") { addSheet = .current }
                    Button("Add Setup Token / API Key…") { addSheet = .token }
                } label: { Label("Add", systemImage: "plus") }
                Menu {
                    ForEach(AppStore.SwitchMode.allCases) { mode in
                        Button(mode.title) { Task { await store.switchNext(mode) } }
                    }
                } label: { Label("Switch", systemImage: "arrow.left.arrow.right") }
                Menu { SignInMenuSection() } label: { Label("Sign In", systemImage: "person.badge.key") }
                Button { Task { await store.refresh(); await store.loadExtras() } } label: {
                    Label("Refresh", systemImage: "arrow.clockwise").spinning(store.isRefreshing)
                }
                .disabled(store.isRefreshing)
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
                    Text(account.displayName).font(.headline).lineLimit(1)
                    Text(([account.alias != nil ? account.email : nil, "Slot \(account.number)", account.organizationName]
                        .compactMap { $0 }).joined(separator: " · "))
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    AccountBadges(account: account)
                }
                Spacer(minLength: 0)
            }
            // Same layout for every card (usage unknown shows "—"), so cards needing a re-login are
            // as tall as the others.
            CardTide(account: account)
            Text(account.statusNote ?? store.wake?.account(account.number)?.summary() ?? " ")
                .font(.caption).foregroundStyle(account.statusNote != nil ? Brand.warn : .secondary)
                .lineLimit(1).truncationMode(.tail)
                .help(account.statusNote ?? "")
            HStack {
                if account.offersSignIn {
                    SignInAgainButton(account: account, prominent: account.needsRelogin)
                } else if account.active {
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
        .frame(maxHeight: .infinity, alignment: .top)
        .background(selected ? AnyShapeStyle(Color.accentColor.opacity(0.18)) : AnyShapeStyle(.fill.quaternary),
                    in: .rect(cornerRadius: 16))
        // The whole rounded card is clickable, padding and empty space included (not only its views).
        .contentShape(.rect(cornerRadius: 16))
        .contextMenu { AccountContextMenu(account: account) }
    }
}

/// The card's tide reading: the 5-hour % large, its reset, the staff, and the week.
private struct CardTide: View {
    let account: Account

    var body: some View {
        let five = account.usage?.fiveHour?.withPace(windowSeconds: UsageWindow.fiveHourSeconds)
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .lastTextBaseline) {
                Text(five?.pct.map { "\(Int($0))%" } ?? "—")
                    .font(Brand.display(28, relativeTo: .title).monospacedDigit())
                    .foregroundStyle(five?.pct == nil ? .tertiary : .primary)
                Text("5-hour").font(.caption).foregroundStyle(.secondary)
                Spacer()
                if let clock = five?.clock {
                    Text("resets \(clock)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
            }
            TideStaff(window: five, height: 5)
            TimeLeftRule(resetsAt: five?.resetsAt, windowSeconds: UsageWindow.fiveHourSeconds)
            HStack {
                if let d = five?.paceDelta, abs(d) >= UsageWindow.paceThreshold {
                    Text(d > 0 ? "\(d) pts ahead of pace" : "\(-d) pts under pace")
                        .foregroundStyle(d > 0 ? Brand.warn : .secondary)
                }
                Spacer()
            }
            .font(.caption)
            HStack {
                Text("Week \(account.usage?.sevenDay?.pct.map { "\(Int($0))%" } ?? "—")")
                Spacer()
                if let c = account.usage?.sevenDay?.countdown { Text("resets in \(c)") }
            }
            .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }
}
