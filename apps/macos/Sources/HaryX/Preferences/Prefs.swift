import HaryXCore
import Foundation

/// UserDefaults keys and defaults, shared by `@AppStorage` views and services.
enum Prefs {
    static let cswapPath = "cswapPath"
    static let refreshSeconds = "refreshSeconds"
    static let terminal = "terminal"
    static let shell = "shell"
    static let claudeFolder = "claudeFolder"
    static let autoOnLaunch = "autoOnLaunch"
    static let autoDryRun = "autoDryRun"
    static let menuBarPercent = "menuBarPercent"
    static let menuBarMascot = "menuBarMascot"
    static let mascotStyles = "mascotStyles"
    static let cliInstallOffered = "cliInstallOffered"
    /// "Claude Desktop follows Use" (off by default) and its confirm's "Don't ask again".
    static let desktopFollows = "desktopFollows"
    static let desktopSkipConfirm = "desktopSkipConfirm"
    static let sessionMinSeconds = 60.0

    static func notifyKey(_ c: Notice.Category) -> String { "notify.\(c.rawValue)" }

    static func register() {
        var defaults: [String: Any] = [
            refreshSeconds: 60.0, terminal: TerminalApp.terminal.rawValue, shell: "",
            claudeFolder: CSwapCLI.home, autoOnLaunch: false, autoDryRun: false, desktopFollows: false, desktopSkipConfirm: false, menuBarPercent: true, menuBarMascot: false,
        ]
        for c in Notice.Category.allCases { defaults[notifyKey(c)] = true }
        UserDefaults.standard.register(defaults: defaults)
    }

    static var d: UserDefaults { .standard }
}
