import AppKit
import HaryXCore

/// UserDefaults keys and defaults, shared by `@AppStorage` views and services.
enum Prefs {
    static let cswapPath = "cswapPath"
    static let refreshSeconds = "refreshSeconds"
    static let terminal = "terminal"
    static let shell = "shell"
    static let claudeFolder = "claudeFolder"
    static let autoOnLaunch = "autoOnLaunch"
    static let autoDryRun = "autoDryRun"
    /// Whether the user last left auto-switch on; quitting keeps it, so the next launch resumes.
    static let autoWasOn = "autoWasOn"
    static let menuBarPercent = "menuBarPercent"
    /// `AppTheme` raw value: follow the system, or always light or dark.
    static let theme = "theme"
    static let menuBarMascot = "menuBarMascot"
    static let mascotStyles = "mascotStyles"
    static let cliInstallOffered = "cliInstallOffered"
    /// "Claude Desktop follows Use" (off by default) and its confirm's "Don't ask again".
    static let desktopFollows = "desktopFollows"
    static let desktopSkipConfirm = "desktopSkipConfirm"
    /// In-app updates: `UpdateMode` raw value, the last lookup, and a release the user skipped.
    static let updateMode = "updateMode"
    static let lastUpdateCheck = "lastUpdateCheck"
    static let skippedVersion = "skippedVersion"
    /// Usage history: a weekly summary notification (off by default) and when it last went out.
    static let weeklySummary = "weeklySummary"
    static let lastWeeklySummary = "lastWeeklySummary"
    static let sessionMinSeconds = 60.0

    static func notifyKey(_ c: Notice.Category) -> String { "notify.\(c.rawValue)" }

    static func register() {
        var defaults: [String: Any] = [
            refreshSeconds: 60.0, terminal: TerminalApp.terminal.rawValue, shell: "",
            claudeFolder: CSwapCLI.home, autoOnLaunch: false, autoWasOn: false, autoDryRun: false, desktopFollows: false, desktopSkipConfirm: false, menuBarPercent: true, menuBarMascot: false,
            updateMode: UpdateMode.notify.rawValue, weeklySummary: false, theme: AppTheme.system.rawValue,
        ]
        for c in Notice.Category.allCases { defaults[notifyKey(c)] = true }
        UserDefaults.standard.register(defaults: defaults)
    }

    static var d: UserDefaults { .standard }
}

/// The app's light or dark look. System leaves `NSApp.appearance` nil so macOS decides.
enum AppTheme: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }

    var title: String {
        switch self {
        case .system: "System"
        case .light: "Light"
        case .dark: "Dark"
        }
    }

    @MainActor static func applySaved() {
        let theme = AppTheme(rawValue: Prefs.d.string(forKey: Prefs.theme) ?? "") ?? .system
        NSApp.appearance = switch theme {
        case .system: nil
        case .light: NSAppearance(named: .aqua)
        case .dark: NSAppearance(named: .darkAqua)
        }
    }
}
