import HaryXCore
import SwiftUI

/// The app-side AI settings shown above the CLI's own settings in AI Settings: the usage reading in
/// the menu bar, where terminals open, Claude Desktop following Use, and which notifications post.

struct UsageDisplaySection: View {
    @Environment(AppStore.self) private var store
    @AppStorage(Prefs.refreshSeconds) private var refreshSeconds = 60.0
    @AppStorage(Prefs.menuBarPercent) private var menuBarPercent = true

    var body: some View {
        Section("Usage & menu bar") {
            Picker("Refresh usage every", selection: $refreshSeconds) {
                Text("30 seconds").tag(30.0)
                Text("1 minute").tag(60.0)
                Text("2 minutes").tag(120.0)
                Text("5 minutes").tag(300.0)
            }
            Toggle("Show 5-hour % in the menu bar", isOn: $menuBarPercent)
            Toggle("Show animated mascot in the menu bar", isOn: Binding(
                get: { store.menuBarMascot }, set: { store.setMenuBarMascot($0) }))
                .help("Replaces the HaryX mark with the active account's mascot, which walks while Claude runs. Costs a little CPU while it walks.")
        }
    }
}

struct TerminalSettingsSection: View {
    @Environment(AppStore.self) private var store
    @AppStorage(Prefs.terminal) private var terminal = TerminalApp.terminal.rawValue
    @AppStorage(Prefs.shell) private var shell = ""
    @AppStorage(Prefs.claudeFolder) private var claudeFolder = CSwapCLI.home

    var body: some View {
        Section("Terminal") {
            Picker("Open terminals in", selection: $terminal) {
                ForEach(TerminalApp.installed) { Text($0.title).tag($0.rawValue) }
            }
            Picker("Shell", selection: $shell) {
                ForEach(ShellChoice.allCases) { Text($0.title).tag($0.rawValue) }
            }
            LabeledContent("Default folder") {
                HStack {
                    Text((claudeFolder as NSString).abbreviatingWithTildeInPath).lineLimit(1).truncationMode(.head)
                    Button("Choose…") { if let dir = AppStore.chooseFolder() { claudeFolder = dir } }
                }
            }
            Button("Test: Open a New Shell") { store.openTerminal() }
            if TerminalApp.preferred == .warp {
                Text("Warp opens through a launch configuration in ~/.warp/launch_configurations/ and uses its own shell setting (the Shell choice above doesn't apply).")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

struct DesktopSettingsSection: View {
    @Environment(AppStore.self) private var store
    @AppStorage(Prefs.desktopFollows) private var desktopFollows = false
    @AppStorage(Prefs.desktopSkipConfirm) private var desktopSkipConfirm = false

    var body: some View {
        Section {
            Toggle("Claude Desktop per account", isOn: $desktopFollows)
                .onChange(of: desktopFollows) { Task { await store.reloadDesktop() } }
            Toggle("Ask before quitting Claude Desktop", isOn: Binding(
                get: { !desktopSkipConfirm }, set: { desktopSkipConfirm = !$0 }))
                .disabled(!desktopFollows)
        } header: { Text("Claude Desktop") } footer: {
            Text("Each account gets its own Claude Desktop folder: sign in once with Set Up Desktop on its card, then Open Desktop reopens Claude Desktop as that account. Use, auto-switch and AI Wake never quit or open Desktop. Each folder takes about 750 MB.")
        }
    }
}

struct NotificationSettingsSection: View {
    @Environment(AppStore.self) private var store
    @State private var showNotifyDenied = false

    var body: some View {
        Section("Notifications") {
            ForEach(Notice.Category.allCases, id: \.self) { NotifyToggle(category: $0) }
            Button("Send Test Notification") { Task { await sendTestNotification() } }
        }
        .alert("Notifications are turned off", isPresented: $showNotifyDenied) {
            Button("Open System Settings") { Notifier.openSystemSettings() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("HaryX isn't allowed to post notifications. Allow them in System Settings › Notifications › HaryX.")
        }
    }

    private func sendTestNotification() async {
        switch await store.notifier.ensurePermission() {
        case .granted:
            store.notifier.post(Notice(key: "test:\(Date().timeIntervalSince1970)", category: .switched,
                                       title: "HaryX notifications work", body: "You'll hear from your mascots here."),
                                force: true)
        case .denied:
            showNotifyDenied = true
        case .unavailable:
            store.flash("Notifications need the bundled HaryX.app (not `swift run`).", error: true)
        }
    }
}

private struct NotifyToggle: View {
    let category: Notice.Category
    @AppStorage private var on: Bool

    init(category: Notice.Category) {
        self.category = category
        _on = AppStorage(wrappedValue: true, Prefs.notifyKey(category))
    }

    var body: some View {
        Toggle(title, isOn: $on)
    }

    private var title: String {
        switch category {
        case .switched: "Account switched"
        case .usage: "Usage reaches 80% / 95% or is on pace to run out"
        case .login: "Re-login needed, login expiring, plan expired"
        case .reset: "Exhausted account is ready again"
        case .session: "Claude session finished (ran 1 min or longer)"
        case .autoSwitch: "Auto-switch problems (quarantine, all exhausted, errors)"
        }
    }
}
