import AppKit
import HaryXCore

/// Left-click menu for an account in the menu bar panel: Info, Sign In Again, Set Up Claude Desktop,
/// Switch, Switch Without Saving Current Login, Groups, Remove.
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
        }
        if store.offersDesktopSetUp(account) {
            let desktop = ActionItem("Set Up Claude Desktop…", symbol: "macwindow") {
                Task { await store.openDesktop(as: account) }
            }
            desktop.isEnabled = store.busy == nil
            menu.addItem(desktop)
        }
        if account.canRelogin || store.offersDesktopSetUp(account) { menu.addItem(.separator()) }
        let switchItem = ActionItem("Switch", symbol: "arrow.left.arrow.right") {
            Task { await store.switchTo(account) }
        }
        switchItem.isEnabled = !account.active && store.busy == nil
        menu.addItem(switchItem)
        // Named and confirmed as in the account inspector: it skips backing up the current login.
        let forceItem = ActionItem("Switch Without Saving Current Login…", symbol: "bolt.fill") {
            guard confirmForce(account) else { return }
            Task { await store.switchTo(account, force: true) }
        }
        forceItem.isEnabled = store.busy == nil
        menu.addItem(forceItem)
        menu.addItem(.separator())
        menu.addItem(groupItem(for: account, store: store))
        menu.addItem(.separator())
        let removeItem = ActionItem("Remove…", symbol: "trash") {
            guard confirmRemove(account) else { return }
            Task { await store.remove(account) }
        }
        removeItem.isEnabled = store.busy == nil
        menu.addItem(removeItem)
        menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
    }

    /// "Groups" with a submenu: every group in use, checked where the account is in it (choosing
    /// one adds or removes it), New Group…, and Remove from All Groups. Matches the cards' Groups menu.
    private static func groupItem(for account: Account, store: AppStore) -> NSMenuItem {
        let title = account.groupNames.isEmpty ? "Groups" : "Groups: \(account.groupLabel)"
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.image = NSImage(systemSymbolName: "folder", accessibilityDescription: nil)
        item.isEnabled = store.busy == nil
        let sub = NSMenu()
        sub.autoenablesItems = false
        let names = AccountGroups.names(store.accounts)
        for name in names {
            let member = account.isInGroup(name)
            let pick = ActionItem(name, symbol: "folder") {
                Task {
                    if member { await store.removeFromGroup(account, name) } else { await store.addToGroup(account, name) }
                }
            }
            pick.image = nil
            pick.state = member ? .on : .off
            sub.addItem(pick)
        }
        if !names.isEmpty { sub.addItem(.separator()) }
        sub.addItem(ActionItem("New Group…", symbol: "folder.badge.plus") {
            DashboardRouter.shared.newGroup([account.number])
        })
        if account.groupNames.count > 1 {
            sub.addItem(ActionItem("Remove from All Groups", symbol: "folder.badge.minus") {
                Task { await store.clearGroups(account) }
            })
        }
        item.submenu = sub
        return item
    }

    private static func confirmForce(_ account: Account) -> Bool {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Switch to \(account.displayName) without saving the current login?"
        alert.informativeText = "Activates the stored credentials without first backing up the current login (--force). Use it when the current login is broken."
        alert.addButton(withTitle: "Switch Anyway").hasDestructiveAction = true
        alert.addButton(withTitle: "Cancel")
        NSApp.activate()
        return alert.runModal() == .alertFirstButtonReturn
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
