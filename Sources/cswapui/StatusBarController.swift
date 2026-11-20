import AppKit
import SwiftUI
import os

/// Owns the status item directly: swapping `button.image` per animation frame is nearly free,
/// whereas a SwiftUI `MenuBarExtra` label re-runs Auto Layout on every frame (~7% CPU at 3 fps).
@MainActor
final class StatusBarController: NSObject, NSPopoverDelegate {
    private let store: AppStore
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let popover = NSPopover()
    private var lastTitle = ""
    private let log = Logger(subsystem: "dev.cswapui", category: "statusbar")

    init(store: AppStore) {
        self.store = store
        super.init()
        popover.behavior = .transient
        popover.delegate = self
        let host = NSHostingController(rootView: MenuBarPanel().environment(store))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host

        if let button = item.button {
            button.imagePosition = .imageLeading
            button.target = self
            button.action = #selector(toggle)
            button.font = .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        }
        render()
    }

    @objc private func toggle() {
        guard let button = item.button else { return }
        log.notice("toggle (shown=\(self.popover.isShown))")
        if popover.isShown {
            popover.performClose(nil)
        } else {
            NSApp.activate() // an inactive accessory app's transient popover closes immediately
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
            popover.contentViewController?.view.window?.makeKey()
        }
    }

    func close() { popover.performClose(nil) }

    func popoverDidShow(_ notification: Notification) {
        log.notice("popover shown")
        store.panelVisible = true
    }
    func popoverDidClose(_ notification: Notification) { store.panelVisible = false }

    /// Re-renders whenever an observed store property it reads changes.
    private func render() {
        withObservationTracking {
            guard let button = item.button else { return }
            button.image = store.menuBarImage
            let pct = store.active?.usage?.fiveHour?.pct ?? store.active?.usage?.sevenDay?.pct
            let title = Prefs.d.bool(forKey: Prefs.menuBarPercent) ? pct.map { " \(Int($0))%" } ?? "" : ""
            if title != lastTitle { // avoid re-layout when only the image changed
                button.title = title
                lastTitle = title
            }
            let tip = store.active.map { "cswap · \($0.displayName)" } ?? "cswap"
            if button.toolTip != tip { button.toolTip = tip }
        } onChange: { [weak self] in
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.render() } }
        }
    }
}

/// The dashboard lives in an AppKit-owned window so the menu bar can open it without a
/// SwiftUI scene context. While it's open the app shows in the Dock and ⌘-Tab.
@MainActor
final class DashboardWindow: NSObject, NSWindowDelegate {
    static let shared = DashboardWindow()
    var store: AppStore?
    private var window: NSWindow?

    func show() {
        guard let store else { return }
        if window == nil {
            let host = NSHostingController(rootView: DashboardView().environment(store))
            host.sceneBridgingOptions = [.toolbars, .title]
            let w = NSWindow(contentViewController: host)
            w.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
            w.title = "cswapui"
            w.setContentSize(NSSize(width: 1080, height: 700))
            w.isReleasedWhenClosed = false
            w.delegate = self
            w.setFrameAutosaveName("cswapui.dashboard")
            if w.frame.origin == .zero { w.center() }
            window = w
        }
        NSApp.setActivationPolicy(.regular)
        window?.makeKeyAndOrderFront(nil)
        NSApp.activate()
    }

    func windowWillClose(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
    }
}
