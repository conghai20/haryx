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
    /// Close triggers that `.transient` misses for an accessory app (installed while the panel shows).
    private var dismissMonitors: [Any] = []
    private var dismissObservers: [NSObjectProtocol] = []
    private let log = Logger(subsystem: "dev.haryx.app", category: "statusbar")

    init(store: AppStore) {
        self.store = store
        super.init()
        popover.behavior = CommandLine.arguments.contains("--open-panel") ? .applicationDefined : .transient
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

    func open() {
        popover.appearance = NSApp.appearance
        if !popover.isShown { toggle() }
    }

    func popoverDidShow(_ notification: Notification) {
        log.notice("popover shown")
        store.panelVisible = true
        installDismissTriggers()
    }
    func popoverDidClose(_ notification: Notification) {
        store.panelVisible = false
        removeDismissTriggers()
    }

    /// Close like a system menu bar panel: on a click in another app, on switching apps (⌘-Tab, Dock),
    /// and when another HaryX window (the dashboard) becomes key.
    private func installDismissTriggers() {
        removeDismissTriggers()
        guard !CommandLine.arguments.contains("--open-panel") else { return } // design review keeps it open
        let mask: NSEvent.EventTypeMask = [.leftMouseDown, .rightMouseDown, .otherMouseDown]
        if let global = NSEvent.addGlobalMonitorForEvents(matching: mask, handler: { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        }) {
            dismissMonitors.append(global)
        }
        let center = NotificationCenter.default
        dismissObservers.append(center.addObserver(forName: NSApplication.didResignActiveNotification,
                                                   object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        })
        dismissObservers.append(center.addObserver(forName: NSWindow.didBecomeKeyNotification,
                                                   object: nil, queue: .main) { [weak self] note in
            // Delivered on the main queue; the window is only touched there.
            let keyWindow = note.object as? NSWindow
            MainActor.assumeIsolated {
                guard let self, let window = keyWindow,
                      window != self.popover.contentViewController?.view.window,
                      !window.className.contains("StatusBar") else { return }
                // Sheets and alerts the panel itself opens keep it open.
                if window.sheetParent == nil, !(window is NSPanel) { self.close() }
            }
        })
    }

    private func removeDismissTriggers() {
        dismissMonitors.forEach(NSEvent.removeMonitor)
        dismissMonitors.removeAll()
        dismissObservers.forEach(NotificationCenter.default.removeObserver)
        dismissObservers.removeAll()
    }

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
            let tip = store.active.map { "HaryX · \($0.displayName)" } ?? "HaryX"
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
            w.title = "HaryX"
            w.setContentSize(NSSize(width: 1080, height: 700))
            w.isReleasedWhenClosed = false
            w.delegate = self
            w.setFrameAutosaveName("haryx.dashboard")
            if w.frame.origin == .zero { w.center() }
            window = w
        }
        NSApp.setActivationPolicy(.regular)
        if window?.isMiniaturized == true { window?.deminiaturize(nil) }
        window?.makeKeyAndOrderFront(nil)
        NSApp.activate()
    }

    func windowWillClose(_ notification: Notification) {
        store?.dashboardVisible = false
        NSApp.setActivationPolicy(.accessory)
    }

    /// Mascots and other motion stop while the window is covered or minimized.
    func windowDidChangeOcclusionState(_ notification: Notification) {
        guard let window = notification.object as? NSWindow else { return }
        store?.dashboardVisible = window.occlusionState.contains(.visible)
    }
}
