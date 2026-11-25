import HaryXCore
import ServiceManagement
import SwiftUI

struct PreferencesPane: View {
    @Environment(AppStore.self) private var store
    @AppStorage(Prefs.cswapPath) private var cswapPath = ""
    @AppStorage(Prefs.theme) private var theme = AppTheme.system.rawValue
    @State private var launchAtLogin = SMAppService.mainApp.status == .enabled

    var body: some View {
        Form {
            Section("General") {
                Toggle("Launch at login", isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _, on in setLaunchAtLogin(on) }
                Picker("Theme", selection: $theme) {
                    ForEach(AppTheme.allCases) { Text($0.title).tag($0.rawValue) }
                }
                .onChange(of: theme) { AppTheme.applySaved() }
            }

            CommandLineToolSection()

            Section {
                TextField("CLI path", text: $cswapPath, prompt: Text(CSwapCLI.locate(override: nil)?.path ?? "auto-detect"))
                    .onSubmit { store.relocateCLI(); Task { await store.refresh() } }
                LabeledContent("Using", value: store.cli?.executable.path ?? "not found")
            } header: { Text("Command-line backend") } footer: {
                Text("Leave blank to auto-detect `haryx` (then `cswap`) in ~/.local/bin, /opt/homebrew/bin, /usr/local/bin, ~/.cargo/bin. Install with `brew install haryx` or `cargo install haryx`.")
            }
        }
        .formStyle(.grouped)
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

/// The `haryx` command: whether the app's bundled CLI is linked onto the PATH, and buttons to change it.
private struct CommandLineToolSection: View {
    @Environment(AppStore.self) private var store
    @State private var state = CLIInstaller.current.state

    var body: some View {
        Section {
            switch state {
            case .linked(let path):
                LabeledContent("Installed", value: path)
                Button("Remove Command") { store.removeCLI(at: path); state = CLIInstaller.current.state }
            case .otherInstall(let path):
                LabeledContent("Using your own install", value: path)
            case .notInstalled:
                HStack {
                    Button("Install in /usr/local/bin…") { store.installCLI(systemWide: true); state = CLIInstaller.current.state }
                    Button("Install for Me Only") { store.installCLI(systemWide: false); state = CLIInstaller.current.state }
                }
            case .notBundled:
                Text("This build has no bundled haryx; install it with `cargo install haryx` (Homebrew, pipx and npm are coming).")
                    .foregroundStyle(.secondary)
            }
        } header: { Text("Terminal command") } footer: {
            Text("Links `haryx` from inside HaryX.app, so updating the app updates the command. Another haryx (Homebrew, cargo, pipx, npm) is never replaced.")
        }
        .onAppear { state = CLIInstaller.current.state }
    }
}
