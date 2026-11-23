import AppKit
import HaryXCore
import SwiftUI

// `HaryX --render-icon out.png`: draws the legacy sprite icon (kept for reference; build.sh uses the brand icon).
if let i = CommandLine.arguments.firstIndex(of: "--render-icon"), i + 1 < CommandLine.arguments.count {
    do {
        guard let image = AppIcon.render() else { throw CocoaError(.fileWriteUnknown) }
        try AppIcon.writePNG(image, to: URL(fileURLWithPath: CommandLine.arguments[i + 1]))
        exit(0)
    } catch {
        FileHandle.standardError.write(Data("icon render failed: \(error)\n".utf8))
        exit(1)
    }
}

signal(SIGPIPE, SIG_IGN) // writes to an exited child's stdin return EPIPE instead of killing the app
HaryXApp.main()

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let store = AppStore()
    private var statusBar: StatusBarController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusBar = StatusBarController(store: store)
        DashboardWindow.shared.store = store
        store.start()
        // Design review: `--open-dashboard` shows the dashboard, `--open-panel` opens the menu bar panel once the first refresh lands;
        // `--light` / `--dark` force an appearance.
        if CommandLine.arguments.contains("--light") { NSApp.appearance = NSAppearance(named: .aqua) }
        if CommandLine.arguments.contains("--dark") { NSApp.appearance = NSAppearance(named: .darkAqua) }
        if CommandLine.arguments.contains("--open-dashboard") { DashboardWindow.shared.show() }
        if CommandLine.arguments.contains("--open-panel") {
            DispatchQueue.main.asyncAfter(deadline: .now() + 4) { [weak self] in self?.statusBar?.open() }
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        store.shutdown()
    }

    /// Re-opening the app from Finder/Spotlight shows the dashboard.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        DashboardWindow.shared.show()
        return false
    }
}

struct HaryXApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        Settings {
            PreferencesPane().environment(delegate.store).frame(width: 520, height: 560)
        }
    }
}
