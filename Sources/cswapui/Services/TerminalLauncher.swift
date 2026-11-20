import AppKit
import CSwapCore

enum TerminalApp: String, CaseIterable, Identifiable {
    case terminal, iterm, warp, ghostty, kitty, wezterm, alacritty

    var id: String { rawValue }

    var title: String {
        switch self {
        case .terminal: "Terminal"
        case .iterm: "iTerm2"
        case .warp: "Warp"
        case .ghostty: "Ghostty"
        case .kitty: "kitty"
        case .wezterm: "WezTerm"
        case .alacritty: "Alacritty"
        }
    }

    var bundleID: String {
        switch self {
        case .terminal: "com.apple.Terminal"
        case .iterm: "com.googlecode.iterm2"
        case .warp: "dev.warp.Warp-Stable"
        case .ghostty: "com.mitchellh.ghostty"
        case .kitty: "net.kovidgoyal.kitty"
        case .wezterm: "com.github.wez.wezterm"
        case .alacritty: "org.alacritty"
        }
    }

    var appURL: URL? { NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) }
    var icon: NSImage? { appURL.map { NSWorkspace.shared.icon(forFile: $0.path) } }

    static var installed: [TerminalApp] { allCases.filter { $0.appURL != nil } }

    static var preferred: TerminalApp {
        let saved = TerminalApp(rawValue: Prefs.d.string(forKey: Prefs.terminal) ?? "") ?? .terminal
        return saved.appURL != nil ? saved : .terminal
    }
}

enum ShellChoice: String, CaseIterable, Identifiable {
    case login = "", zsh = "/bin/zsh", bash = "/bin/bash", fish = "fish"
    var id: String { rawValue }
    var title: String { self == .login ? "Login shell ($SHELL)" : rawValue.split(separator: "/").last.map(String.init) ?? rawValue }
}

/// Opens a terminal window in a folder, optionally running a command, and leaves an
/// interactive shell behind when the command exits.
@MainActor
enum TerminalLauncher {
    enum LaunchError: LocalizedError {
        case notInstalled(String), failed(String)
        var errorDescription: String? {
            switch self {
            case .notInstalled(let t): "\(t) is not installed."
            case .failed(let m): m
            }
        }
    }

    /// Async launch failures (NSWorkspace completion handlers) are reported here.
    static var onError: ((String) -> Void)?

    static var scriptDir: URL { FileManager.default.temporaryDirectory.appendingPathComponent("cswapui", isDirectory: true) }

    static func open(_ app: TerminalApp = .preferred, command: String?, in folder: String) throws {
        guard let appURL = app.appURL else { throw LaunchError.notInstalled(app.title) }
        let script = try writeScript(command: command, folder: folder)
        let config = NSWorkspace.OpenConfiguration()
        config.activates = true

        switch app {
        case .terminal:
            // Terminal runs `.command` files in a new window; no Automation permission needed.
            NSWorkspace.shared.open([script], withApplicationAt: appURL, configuration: config, completionHandler: report(app))
        case .iterm:
            try runAppleScript("""
            tell application id "com.googlecode.iterm2"
              activate
              create window with default profile command \(appleScriptString(script.path))
            end tell
            """)
        case .warp:
            try openWarp(script: script, folder: folder)
        case .ghostty, .alacritty:
            config.createsNewApplicationInstance = true
            config.arguments = ["-e", script.path]
            NSWorkspace.shared.openApplication(at: appURL, configuration: config, completionHandler: report(app))
        case .kitty:
            config.createsNewApplicationInstance = true
            config.arguments = [script.path]
            NSWorkspace.shared.openApplication(at: appURL, configuration: config, completionHandler: report(app))
        case .wezterm:
            config.createsNewApplicationInstance = true
            config.arguments = ["start", "--cwd", folder, "--", script.path]
            NSWorkspace.shared.openApplication(at: appURL, configuration: config, completionHandler: report(app))
        }
    }

    /// The script deletes itself, cds into the folder, runs the command, then execs a shell.
    static func writeScript(command: String?, folder: String) throws -> URL {
        let fm = FileManager.default
        try fm.createDirectory(at: scriptDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        pruneOldScripts()
        let chosen = Prefs.d.string(forKey: Prefs.shell).flatMap { $0.isEmpty ? nil : $0 }
        let shell = chosen.map(shellQuote) ?? #""${SHELL:-/bin/zsh}""#
        let path = CSwapCLI.searchPaths.joined(separator: ":")
        let body = """
        #!/bin/zsh
        rm -f -- "$0"
        export PATH=\(shellQuote(path)):"$PATH"
        if ! cd \(shellQuote(folder)); then
          echo "cswapui: folder not found: "\(shellQuote(folder))
          exec \(shell) -l
        fi
        clear
        \(command ?? "")
        exec \(shell) -l
        """
        let url = scriptDir.appendingPathComponent("launch-\(UUID().uuidString.prefix(8)).command")
        try body.write(to: url, atomically: true, encoding: .utf8)
        try fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: url.path)
        return url
    }

    /// Warp runs commands only via launch configurations (YAML) opened by URL.
    static func openWarp(script: URL, folder: String) throws {
        let dir = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".warp/launch_configurations", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        // Unique per launch: two quick launches must not overwrite each other's config.
        for old in (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? [] where old.hasPrefix("cswapui-") {
            try? FileManager.default.removeItem(at: dir.appendingPathComponent(old))
        }
        let id = UUID().uuidString.prefix(8)
        let name = "cswapui-\(id).yaml"
        let yaml = """
        ---
        name: cswapui-\(id)
        windows:
          - tabs:
              - layout:
                  cwd: \(yamlString(folder))
                  commands:
                    - exec: \(yamlString(shellQuote(script.path)))
        """
        try yaml.write(to: dir.appendingPathComponent(name), atomically: true, encoding: .utf8)
        guard let url = URL(string: "warp://launch/\(name)") else { throw LaunchError.failed("Bad Warp URL") }
        NSWorkspace.shared.open(url)
    }

    static func pruneOldScripts() {
        let fm = FileManager.default
        let cutoff = Date().addingTimeInterval(-86_400)
        for url in (try? fm.contentsOfDirectory(at: scriptDir, includingPropertiesForKeys: [.contentModificationDateKey])) ?? [] {
            let modified = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate ?? .distantPast
            if modified < cutoff { try? fm.removeItem(at: url) }
        }
    }

    static func runAppleScript(_ source: String) throws {
        var error: NSDictionary?
        NSAppleScript(source: source)?.executeAndReturnError(&error)
        if let error { throw LaunchError.failed(error[NSAppleScript.errorMessage] as? String ?? "AppleScript failed") }
    }

    static func appleScriptString(_ s: String) -> String {
        "\"" + s.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
    }

    static func yamlString(_ s: String) -> String {
        let escaped = s.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\n", with: "\\n").replacingOccurrences(of: "\r", with: "\\r")
        return "\"" + escaped + "\""
    }

    static func report(_ app: TerminalApp) -> @Sendable (NSRunningApplication?, Error?) -> Void {
        { _, error in
            guard let error else { return }
            let message = "\(app.title): \(error.localizedDescription)"
            DispatchQueue.main.async { MainActor.assumeIsolated { onError?(message) } }
        }
    }
}
