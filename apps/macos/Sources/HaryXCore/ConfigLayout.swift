import Foundation

/// Where each `haryx config` key appears in AI Settings: which section, in what order, which keys wait
/// on another, and how choice values read. The Windows app uses the same rules (`lib/settings.ts`).
public enum ConfigLayout {
    public struct Group: Identifiable, Equatable, Sendable {
        public let title: String
        public let entries: [ConfigEntry]
        public var id: String { title }
    }

    /// Auto-switch rules (when, how often, which account), then the keys whose order carries meaning:
    /// a caption that says "the setting above" must sit below it. Other keys follow in key order.
    public static let keyOrder = [
        "autoswitch.threshold", "autoswitch.weeklyThreshold", "autoswitch.strategy", "autoswitch.hysteresisPct",
        "autoswitch.intervalSeconds", "autoswitch.cooldownSeconds", "autoswitch.unhealthyTicks",
        "autoswitch.includeApiKeyAccounts", "autoswitch.model",
        "usage.headerProbe", "usage.fableProbe", "usage.shareBetweenMachines",
        "history.enabled", "history.import", "history.keepDays",
    ]

    /// Sections the app places itself, in AI Settings order. Any other CLI group follows them.
    public static let knownTitles = ["Usage & menu bar", "Usage history", "Run", "Command line"]

    /// A key whose control is off while the boolean key it names is false.
    static let requires = ["usage.fableProbe": "usage.headerProbe"]

    static let choiceLabels: [String: [String: String]] = [
        "ui.theme": ["auto": "Auto", "dark": "Dark", "light": "Light"],
        "ui.orgTag": ["auto": "Auto", "on": "On", "off": "Off"],
        "history.import": ["ask": "Ask", "all": "Import all", "none": "Skip them"],
    ]

    public static func title(ofGroup group: String) -> String {
        switch group {
        case "usage": "Usage & menu bar"
        case "history": "Usage history"
        case "run": "Run"
        case "ui", "updates": "Command line"
        default: group.prefix(1).uppercased() + group.dropFirst()
        }
    }

    public static func ordered(_ entries: [ConfigEntry]) -> [ConfigEntry] {
        let rank = { (k: String) in keyOrder.firstIndex(of: k) ?? Int.max }
        return entries.sorted { (rank($0.key), $0.key) < (rank($1.key), $1.key) }
    }

    /// Every key except the auto-switch rules (they live in Auto-switch), grouped into sections.
    public static func groups(_ entries: [ConfigEntry]) -> [Group] {
        let byTitle = Dictionary(grouping: entries.filter { !$0.key.hasPrefix("autoswitch.") }) {
            title(ofGroup: String($0.key.split(separator: ".").first ?? ""))
        }
        let sectionRank = { (t: String) in knownTitles.firstIndex(of: t) ?? Int.max }
        return byTitle.keys.sorted { (sectionRank($0), $0) < (sectionRank($1), $1) }
            .map { Group(title: $0, entries: ordered(byTitle[$0] ?? [])) }
    }

    public static func isBlocked(_ key: String, in entries: [ConfigEntry]) -> Bool {
        guard let needed = requires[key] else { return false }
        return entries.first { $0.key == needed }?.value == .bool(false)
    }

    public static func choiceLabel(_ key: String, _ value: String) -> String {
        choiceLabels[key]?[value] ?? value
    }
}
