import HaryXCore
import ServiceManagement
import SwiftUI

struct PreferencesPane: View {
    @Environment(AppStore.self) private var store
    @AppStorage(Prefs.cswapPath) private var cswapPath = ""
    @AppStorage(Prefs.refreshSeconds) private var refreshSeconds = 60.0
    @AppStorage(Prefs.terminal) private var terminal = TerminalApp.terminal.rawValue
    @AppStorage(Prefs.shell) private var shell = ""
    @AppStorage(Prefs.claudeFolder) private var claudeFolder = CSwapCLI.home
    @AppStorage(Prefs.menuBarPercent) private var menuBarPercent = true
    @AppStorage(Prefs.menuBarAnimate) private var menuBarAnimate = true
    @State private var launchAtLogin = SMAppService.mainApp.status == .enabled
    @State private var showNotifyDenied = false

    var body: some View {
        Form {
            Section("General") {
                Toggle("Launch at login", isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _, on in setLaunchAtLogin(on) }
                Toggle("Show 5-hour % in the menu bar", isOn: $menuBarPercent)
                Toggle("Animate menu bar mascot while Claude runs", isOn: $menuBarAnimate)
                    .onChange(of: menuBarAnimate) { store.updateMenuBarAnimation() }
                Picker("Refresh usage every", selection: $refreshSeconds) {
                    Text("30 seconds").tag(30.0)
                    Text("1 minute").tag(60.0)
                    Text("2 minutes").tag(120.0)
                    Text("5 minutes").tag(300.0)
                }
            }

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

            Section("Notifications") {
                ForEach(Notice.Category.allCases, id: \.self) { c in
                    NotifyToggle(category: c)
                }
                Button("Send Test Notification") { Task { await sendTestNotification() } }
            }
            .alert("Notifications are turned off", isPresented: $showNotifyDenied) {
                Button("Open System Settings") { Notifier.openSystemSettings() }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("HaryX isn't allowed to post notifications. Allow them in System Settings › Notifications › HaryX.")
            }

            Section {
                TextField("cswap path", text: $cswapPath, prompt: Text(CSwapCLI.locate(override: nil)?.path ?? "auto-detect"))
                    .onSubmit { store.relocateCLI(); Task { await store.refresh() } }
                LabeledContent("Using", value: store.cli?.executable.path ?? "not found")
            } header: { Text("cswap") } footer: {
                Text("Leave blank to auto-detect in ~/.local/bin, /opt/homebrew/bin, /usr/local/bin. Install with `uv tool install claude-swap`.")
            }
        }
        .formStyle(.grouped)
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

    private func setLaunchAtLogin(_ on: Bool) {
        do {
            if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
        } catch {
            store.flash("Launch at login: \(error.localizedDescription)", error: true)
            launchAtLogin = SMAppService.mainApp.status == .enabled
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
