import CSwapCore
import Foundation
import UserNotifications
import os

/// Posts deduplicated user notifications. The first snapshot after launch only seeds the
/// dedupe set, so restarting the app doesn't replay every standing condition.
@MainActor
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    private var fired: Set<String> = []
    private var seeded = false
    private let log = Logger(subsystem: "dev.cswapui", category: "notify")

    /// UNUserNotificationCenter crashes outside an app bundle (e.g. `swift run`).
    private var center: UNUserNotificationCenter? {
        Bundle.main.bundleIdentifier == nil ? nil : UNUserNotificationCenter.current()
    }

    func requestAuthorization() {
        guard let center else { return }
        center.delegate = self
        center.requestAuthorization(options: [.alert, .sound]) { [log] granted, error in
            if let error { log.error("Notification auth failed: \(error.localizedDescription)") }
            else if !granted { log.notice("Notifications not granted") }
        }
    }

    func handle(_ notices: [Notice]) {
        guard seeded else {
            fired.formUnion(notices.map(\.key))
            seeded = true
            return
        }
        // Time-less keys (re-login, expired plan) re-arm once their condition clears.
        let current = Set(notices.map(\.key))
        fired = fired.filter { key in
            current.contains(key) || !NotificationRules.rearmingPrefixes.contains(where: key.hasPrefix)
        }
        for notice in notices where !fired.contains(notice.key) {
            fired.insert(notice.key)
            post(notice)
        }
    }

    func post(_ notice: Notice) {
        guard Prefs.d.bool(forKey: Prefs.notifyKey(notice.category)), let center else { return }
        let content = UNMutableNotificationContent()
        content.title = notice.title
        content.body = notice.body
        content.sound = notice.category == .usage || notice.category == .login ? .default : nil
        content.threadIdentifier = notice.category.rawValue
        let request = UNNotificationRequest(identifier: notice.key, content: content, trigger: nil)
        center.add(request) { [log] error in
            if let error { log.error("Notification failed: \(error.localizedDescription)") }
        }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async
        -> UNNotificationPresentationOptions { [.banner, .sound, .list] }
}
