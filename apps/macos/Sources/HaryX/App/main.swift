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
        // Design review: `--open-dashboard` shows the dashboard (`--section NAME`, `--select SLOT`), `--open-panel` opens the menu bar panel once the first refresh lands;
        // `--light` / `--dark` force an appearance.
        AppTheme.applySaved()
        if CommandLine.arguments.contains("--light") { NSApp.appearance = NSAppearance(named: .aqua) }
        if CommandLine.arguments.contains("--dark") { NSApp.appearance = NSAppearance(named: .darkAqua) }
        let args = CommandLine.arguments
        func value(after flag: String) -> String? {
            args.firstIndex(of: flag).flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil }
        }
        if let raw = value(after: "--section"), let section = DashboardSection(rawValue: raw) {
            DashboardRouter.shared.section = section
        }
        if let slot = value(after: "--select").flatMap(Int.init) {
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { DashboardRouter.shared.select(slot) }
        }
        if args.contains("--open-dashboard") { DashboardWindow.shared.show() }
        // `--cycle-panel N`: open and close the panel N times, 3 s apart (settled-memory checks).
        if let n = value(after: "--cycle-panel").flatMap(Int.init) {
            for i in 0..<(n * 2) {
                DispatchQueue.main.asyncAfter(deadline: .now() + 5 + Double(i) * 3) { [weak self] in
                    if i.isMultiple(of: 2) { self?.statusBar?.open() } else { self?.statusBar?.close() }
                }
            }
        }
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
