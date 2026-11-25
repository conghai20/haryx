import AppKit
import HaryXCore

/// The HaryX account's sync, wired into the rest of the app: preferences another machine changed
/// take effect here, and a sync that changed accounts refreshes them.
extension AppStore {
    func wireAccountSync() {
        accountSync.cliProvider = { [weak self] in self?.cli }
        accountSync.onPrefsApplied = { [weak self] keys in self?.applySyncedPrefs(keys) }
        accountSync.onLocalChanged = { [weak self] in
            Task { await self?.refresh(); await self?.loadExtras() }
        }
    }

    /// Side effects of preferences written by a pull; the rest are read where they are used.
    private func applySyncedPrefs(_ keys: Set<String>) {
        if keys.contains(Prefs.theme) { AppTheme.applySaved() }
        if keys.contains(Prefs.menuBarMascot) { setMenuBarMascot(Prefs.d.bool(forKey: Prefs.menuBarMascot)) }
        if keys.contains(Prefs.mascotStyles) { styles.reload() }
        // Re-assigning runs the controller's mode change (an automatic download, say).
        if keys.contains(Prefs.updateMode) { updates.mode = updates.mode }
        if keys.contains(Prefs.desktopFollows) { Task { await reloadDesktop() } }
    }
}
