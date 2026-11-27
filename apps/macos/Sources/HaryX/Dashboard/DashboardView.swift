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

    /// HaryX Account is reached from the sidebar footer, not listed.
    var sections: [DashboardSection] { DashboardSection.allCases.filter { $0.group == self && $0 != .haryxAccount } }

    /// The sidebar headings this group's sections sit under, in order.
    var headings: [DashboardHeading] { DashboardHeading.allCases.filter { $0.group == self } }
}

/// The sidebar's sub-groups. AI splits by how often you visit: what you watch daily, what runs on its
/// own, and what you set up once. The Windows app uses the same headings, labels and order.
enum DashboardHeading: String, CaseIterable, Identifiable {
    case monitor, automate, setUp, app
    var id: String { rawValue }

    var group: DashboardGroup { self == .app ? .app : .ai }

    var title: String {
        switch self {
        case .monitor: "Monitor"
        case .automate: "Automate"
        case .setUp: "Set up"
        case .app: "HaryX"
        }
    }

    /// HaryX Account is reached from the sidebar footer, not listed.
    var sections: [DashboardSection] { DashboardSection.allCases.filter { $0.heading == self && $0 != .haryxAccount } }
}

enum DashboardSection: String, CaseIterable, Identifiable {
    case accounts, usage, sessions, autoSwitch, wake, mappings, profiles, config, maintenance, haryxAccount, preferences, about
    var id: String { rawValue }

    var heading: DashboardHeading {
        switch self {
        case .accounts, .usage, .sessions: .monitor
        case .autoSwitch, .wake, .mappings: .automate
        case .profiles, .config, .maintenance: .setUp
        case .haryxAccount, .preferences, .about: .app
        }
    }

    var group: DashboardGroup {
        switch self {
        case .accounts, .usage, .sessions, .autoSwitch, .wake, .mappings, .profiles, .config, .maintenance: .ai
        case .haryxAccount, .preferences, .about: .app
        }
    }

    var title: String {
        switch self {
        case .accounts: "Accounts"
        case .usage: "Usage"
        case .sessions: "Sessions"
        case .autoSwitch: "Auto-switch"
        case .wake: "Wake"
        case .mappings: "Folder mappings"
        case .profiles: "Setup profiles"
        case .config: "AI settings"
        case .maintenance: "Backup & maintenance"
        case .haryxAccount: "HaryX account"
        case .preferences: "App preferences"
        case .about: "About & updates"
        }
    }

    var icon: String {
        switch self {
        case .accounts: "person.2.fill"
        case .usage: "chart.bar.xaxis"
        case .sessions: "terminal"
        case .autoSwitch: "arrow.triangle.2.circlepath"
        case .wake: "alarm"
        case .mappings: "folder.badge.person.crop"
        case .profiles: "shippingbox"
        case .config: "slider.horizontal.3"
        case .maintenance: "externaldrive.badge.timemachine"
        case .haryxAccount: "person.crop.circle"
        case .preferences: "gearshape"
        case .about: "info.circle"
        }
    }
}

extension View {
    /// The frame every form-style pane uses: the form at most `maxWidth` wide and centred, inside a
    /// scroll area that fills the window. A grouped Form caps its own content at
    /// about 720 pt and centres it in anything wider, so the Form stops scrolling itself and sits in a
    /// full-width ScrollView instead: scrolling and the scroll bar then work across the whole pane.
    func paneFrame(maxWidth: CGFloat = 720) -> some View {
        ScrollView {
            scrollDisabled(true)
                .scrollContentBackground(.hidden)
                .frame(maxWidth: maxWidth)
                .frame(maxWidth: .infinity)
        }
    }
}

/// Lets the menu bar deep-link into the dashboard.
@MainActor @Observable
final class DashboardRouter {
    static let shared = DashboardRouter()
    var section: DashboardSection? = .accounts
    var selectedSlot: Int?
    /// Accounts the next export starts with ticked, set by a card's "Export…".
    var exportSelection: Set<Int>?

    /// How long the details panel takes to open or close, with the cards moving and resizing alongside.
    static let detailsAnimation = Animation.smooth(duration: 0.32)

    /// Opens the details panel on `slot`, or closes it with nil. Animated, so the cards move and
    /// resize with the panel instead of jumping to their new places; instant under Reduce Motion.
    func showDetails(_ slot: Int?) {
        guard slot != selectedSlot else { return }
        if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            selectedSlot = slot
        } else {
            withAnimation(Self.detailsAnimation) { selectedSlot = slot }
        }
    }

    func select(_ slot: Int) {
        section = .accounts
        selectedSlot = slot
    }

    /// Opens the dashboard on a section.
    func open(_ section: DashboardSection) {
        self.section = section
        DashboardWindow.shared.show()
    }

    /// Opens Maintenance with these accounts ticked for export.
    func export(_ slots: Set<Int>) {
        exportSelection = slots
        open(.maintenance)
    }

    /// The group being renamed (the Rename sheet is up) or about to be deleted (its confirmation is up).
    var renamingGroup: String?
    var deletingGroup: String?

    /// Accounts the New Group sheet starts with ticked; non-nil while the sheet is up.
    var newGroupSelection: Set<Int>?

    /// Opens the Accounts pane with the New Group sheet, these accounts ticked.
    func newGroup(_ slots: Set<Int> = []) {
        section = .accounts
        newGroupSelection = slots
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
                ForEach(DashboardHeading.allCases) { heading in
                    Section(heading.title) {
                        ForEach(heading.sections) { section in
                            Label(section.title, systemImage: section.icon).tag(section)
                        }
                    }
                }
            }
            .safeAreaInset(edge: .top) {
                BrandLockup(size: 20).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 16).padding(.vertical, 6)
            }
            .safeAreaInset(edge: .bottom) { AccountFooter() }
            .navigationSplitViewColumnWidth(min: 190, ideal: 210)
        } detail: {
            Group {
                switch router.section ?? .accounts {
                case .accounts: AccountsPane()
                case .usage: UsagePane()
                case .sessions: SessionsPane()
                case .autoSwitch: AutoSwitchPane()
                case .wake: WakePane()
                case .mappings: MappingsPane()
                case .profiles: ProfilesPane()
                case .config: ConfigPane()
                case .maintenance: MaintenancePane()
                case .haryxAccount: HaryXAccountPane()
                case .preferences: PreferencesPane()
                case .about: AboutPane()
                }
            }
            .navigationTitle((router.section ?? .accounts).title)
            // Keep the window toolbar in every section. SwiftUI drops it when the visible section has
            // no toolbar items, shrinking the title bar and moving the window buttons on switch, so
            // one invisible item is always present.
            .toolbar {
                ToolbarItem(placement: .navigation) {
                    Color.clear.frame(width: 1, height: 1).accessibilityHidden(true)
                }
                .sharedBackgroundVisibility(.hidden)
            }
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

private struct AccountGrid: View {
    @Bindable private var router = DashboardRouter.shared
    let accounts: [Account]

    var body: some View {
        CardFlow {
            ForEach(accounts) { a in
                AccountCard(account: a, selected: router.selectedSlot == a.number)
                    .onTapGesture { router.showDetails(a.number) }
            }
        }
    }
}

/// One group's cards under a header that collapses them. Collapsed groups are remembered on this
/// machine only.
private struct GroupSection: View {
    @AppStorage(Prefs.collapsedGroups) private var collapsedKeys = ""
    let section: AccountGroups.Section

    private var collapsed: Bool { collapsedKeys.split(separator: "\n").contains(Substring(section.id)) }
    private var title: String { section.name ?? "Ungrouped" }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button {
                var keys = Set(collapsedKeys.split(separator: "\n").map(String.init))
                if collapsed { keys.remove(section.id) } else { keys.insert(section.id) }
                withAnimation(.snappy) { collapsedKeys = keys.sorted().joined(separator: "\n") }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .rotationEffect(.degrees(collapsed ? 0 : 90))
                    Text(title).font(.headline)
                    Text("\(section.accounts.count)").font(.subheadline).foregroundStyle(.secondary).monospacedDigit()
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(title), \(section.accounts.count) accounts")
            .accessibilityValue(collapsed ? "Collapsed" : "Expanded")
            .accessibilityHint(collapsed ? "Shows this group's accounts" : "Hides this group's accounts")
            .contextMenu { if let name = section.name { GroupActions(name: name) } }
            .overlay(alignment: .trailing) {
                if let name = section.name {
                    Menu { GroupActions(name: name) } label: { Image(systemName: "ellipsis.circle") }
                        .menuStyle(.button).buttonStyle(.borderless).menuIndicator(.hidden).fixedSize()
                        .help("Rename or delete \(name)")
                        .accessibilityLabel("\(name) actions")
                        .offset(x: 28)
                }
            }
            if !collapsed { AccountGrid(accounts: section.accounts) }
        }
    }
}

struct AccountsPane: View {
    @Environment(AppStore.self) private var store
    @Bindable private var router = DashboardRouter.shared
    @State private var addSheet: AddAccountSheet.Mode?
    @State private var filter = AccountFilter()

    var body: some View {
        ScrollView {
            if store.claudeSetup?.ready == false { ClaudeSetupBanner().padding([.horizontal, .top], 20) }
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
            let all = store.active.map { [$0] + store.nextInLine } ?? store.nextInLine
            let shown = filter.pruned(for: all).apply(all)
            if filter.isActive, !all.isEmpty {
                FilterBar(filter: $filter, shown: shown.count, total: all.count).padding([.horizontal, .top], 20)
            }
            if !all.isEmpty, shown.isEmpty {
                ContentUnavailableView {
                    Label("No accounts match", systemImage: "line.3.horizontal.decrease.circle")
                } actions: {
                    Button("Clear Filters") { filter = AccountFilter() }
                }
                .padding(.top, 40)
            } else {
                if let active = shown.first(where: \.active) {
                    ActiveAccountCard(account: active, selected: router.selectedSlot == active.number)
                        .onTapGesture { router.showDetails(active.number) }
                        .padding([.horizontal, .top], 20)
                }
                if AccountGroups.inUse(all) {
                    VStack(alignment: .leading, spacing: 20) {
                        ForEach(AccountGroups.sections(shown, excludingActive: true)) { GroupSection(section: $0) }
                    }
                    .padding(20)
                } else {
                    AccountGrid(accounts: shown.filter { !$0.active }).padding(20)
                }
            }
            OtherMachineAccountsSection()
        }
        // A click on empty space (not on a card, whose own tap wins) closes the details panel.
        .contentShape(.rect)
        .onTapGesture { router.showDetails(nil) }
        .inspector(isPresented: Binding(get: { router.selectedSlot != nil }, set: { if !$0 { router.showDetails(nil) } })) {
            if let slot = router.selectedSlot, let a = store.account(slot: slot) {
                AccountInspector(account: a).inspectorColumnWidth(min: 300, ideal: 340, max: 420)
            } else {
                ContentUnavailableView("Select an account", systemImage: "person.crop.square")
            }
        }
        .toolbar {
            ToolbarItemGroup {
                Menu {
                    Section("Add") {
                        Button("Sign In New Account…") { addSheet = .login }
                        Button("Add Current Claude Login…") { addSheet = .current }
                        Button("Add Setup Token / API Key…") { addSheet = .token }
                    }
                    SignInMenuSection(showsNewAccount: false)
                } label: { Label("Add or Sign In", systemImage: "plus") }
                FilterMenu(filter: $filter)
                Button { router.newGroup() } label: { Label("New Group…", systemImage: "folder.badge.plus") }
                    .help("Make a group and choose its accounts")
                    .disabled(store.accounts.isEmpty)
                Menu {
                    ForEach(AppStore.SwitchMode.allCases) { mode in
                        Button(mode.title) { Task { await store.switchNext(mode) } }
                    }
                } label: { Label("Switch", systemImage: "arrow.left.arrow.right") }
                Button { Task { await store.refresh(); await store.loadExtras() } } label: {
                    Label("Refresh", systemImage: "arrow.clockwise").spinning(store.isRefreshing)
                }
                .disabled(store.isRefreshing)
            }
        }
        .sheet(item: $addSheet) { AddAccountSheet(mode: $0) }
        .sheet(isPresented: Binding(get: { router.renamingGroup != nil },
                                    set: { if !$0 { router.renamingGroup = nil } })) {
            if let name = router.renamingGroup { RenameGroupSheet(name: name) }
        }
        .confirmationDialog("Delete group \(router.deletingGroup ?? "")?",
                            isPresented: Binding(get: { router.deletingGroup != nil },
                                                 set: { if !$0 { router.deletingGroup = nil } })) {
            Button("Delete Group", role: .destructive) {
                if let name = router.deletingGroup { Task { await store.deleteGroup(name) } }
            }
        } message: {
            let n = store.accounts.filter { $0.isInGroup(router.deletingGroup ?? "") }.count
            Text("Removes the group from \(n) account\(n == 1 ? "" : "s"). The accounts stay, in their other groups or Ungrouped.")
        }
        .sheet(isPresented: Binding(get: { router.newGroupSelection != nil },
                                    set: { if !$0 { router.newGroupSelection = nil } })) {
            NewGroupSheet(initial: router.newGroupSelection ?? [])
        }
    }
}

