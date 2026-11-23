import AppKit
import HaryXCore

/// Left-click menu for an account in the menu bar panel: Info, Sign In Again, Switch, Force Switch, Remove.
/// Built with AppKit so it pops at the cursor without wrapping the row in a SwiftUI `Menu`
/// (which would swallow clicks on the row's own "Use" button).
@MainActor
enum AccountActionMenu {
    static func show(for account: Account, store: AppStore) {
        let menu = NSMenu()
        menu.autoenablesItems = false
        menu.addItem(ActionItem("Info", symbol: "info.circle") {
            DashboardRouter.shared.reveal(account.number)
        })
        menu.addItem(.separator())
        if account.canRelogin {
            let signIn = ActionItem("Sign In Again…", symbol: "person.badge.key") {
                store.signInAgain(account)
            }
            signIn.isEnabled = !store.loginPending(for: account)
            menu.addItem(signIn)
            menu.addItem(.separator())
        }
        let switchItem = ActionItem("Switch", symbol: "arrow.left.arrow.right") {
            Task { await store.switchTo(account) }
        }
        switchItem.isEnabled = !account.active && store.busy == nil
        menu.addItem(switchItem)
        let forceItem = ActionItem("Force Switch", symbol: "bolt.fill") {
            Task { await store.switchTo(account, force: true) }
        }
        forceItem.isEnabled = store.busy == nil
        menu.addItem(forceItem)
        menu.addItem(.separator())
        let removeItem = ActionItem("Remove…", symbol: "trash") {
            guard confirmRemove(account) else { return }
            Task { await store.remove(account) }
        }
        removeItem.isEnabled = store.busy == nil
        menu.addItem(removeItem)
        menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
    }

    private static func confirmRemove(_ account: Account) -> Bool {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Remove \(account.displayName)?"
        alert.informativeText = "Deletes its stored credentials and config backup. The Claude account itself is untouched."
        alert.addButton(withTitle: "Remove").hasDestructiveAction = true
        alert.addButton(withTitle: "Cancel")
        NSApp.activate()
        return alert.runModal() == .alertFirstButtonReturn
    }
}

/// NSMenuItem that runs a closure instead of a target/selector pair.
private final class ActionItem: NSMenuItem {
    private let handler: () -> Void

    init(_ title: String, symbol: String, handler: @escaping () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(run), keyEquivalent: "")
        target = self
        image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    @objc private func run() { handler() }
}
