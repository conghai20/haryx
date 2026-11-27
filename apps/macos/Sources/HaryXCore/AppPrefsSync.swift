import Foundation

/// The app's synced preferences ↔ the flat object `haryx sync prefs get/set` carries
/// (`packages/contracts/haryx/app-prefs.json`). Shared keys have no prefix and mean the same on
/// macOS and Windows; `macos.*` keys only this app reads. Machine state and paths never sync.
public enum AppPrefsSync {
    public enum Kind: Sendable {
        case bool, number
        /// A string from a fixed set.
        case choice(Set<String>)
        /// The per-account mascot map, stored in UserDefaults as JSON data.
        case mascotStyles
    }

    public struct Entry: Sendable {
        /// The key in the synced object.
        public let syncKey: String
        /// The UserDefaults key (`Prefs` in the app). These names are on disk, so they never change.
        public let defaultsKey: String
        public let kind: Kind
    }

    public static let entries: [Entry] = [
        Entry(syncKey: "theme", defaultsKey: "theme", kind: .choice(["system", "light", "dark"])),
        Entry(syncKey: "weeklySummary", defaultsKey: "weeklySummary", kind: .bool),
        Entry(syncKey: "menuBarMascot", defaultsKey: "menuBarMascot", kind: .bool),
        Entry(syncKey: "mascotStyles", defaultsKey: "mascotStyles", kind: .mascotStyles),
        Entry(syncKey: "updateMode", defaultsKey: "updateMode", kind: .choice(Set(UpdateMode.allCases.map(\.rawValue)))),
        Entry(syncKey: "macos.menuBarPercent", defaultsKey: "menuBarPercent", kind: .bool),
        Entry(syncKey: "macos.desktopFollows", defaultsKey: "desktopFollows", kind: .bool),
        Entry(syncKey: "macos.refreshSeconds", defaultsKey: "refreshSeconds", kind: .number),
        Entry(syncKey: "macos.autoOnLaunch", defaultsKey: "autoOnLaunch", kind: .bool),
    ] + Notice.Category.allCases.map {
        Entry(syncKey: "notifications.\($0.rawValue)", defaultsKey: "notify.\($0.rawValue)", kind: .bool)
    }

    /// The synced object for the preferences this Mac has set. `stored` is the app's persistent
    /// UserDefaults domain, so registered defaults the user never chose are not pushed over
    /// another machine's choices.
    public static func export(_ stored: [String: Any]) -> [String: Any] {
        var out: [String: Any] = [:]
        for entry in entries {
            guard let raw = stored[entry.defaultsKey], let value = syncValue(raw, entry.kind) else { continue }
            out[entry.syncKey] = value
        }
        return out
    }

    /// The object to hand `sync prefs set`: what the CLI holds now with this Mac's values on top.
    /// Keys this app does not own (`windows.*`, keys from a newer app) are kept, since `set`
    /// replaces the whole object and a dropped key would sync as a deletion.
    public static func overlay(_ local: [String: Any], on current: [String: Any]) -> [String: Any] {
        current.merging(local) { _, mine in mine }
    }

    /// UserDefaults writes for a synced object: known keys with the right type only. Unknown
    /// keys, `windows.*` and wrong types are ignored.
    public static func importValues(_ synced: [String: Any]) -> [String: Any] {
        var out: [String: Any] = [:]
        for entry in entries {
            guard let raw = synced[entry.syncKey], let value = defaultsValue(raw, entry.kind) else { continue }
            out[entry.defaultsKey] = value
        }
        return out
    }

    /// Two synced objects hold the same values.
    public static func same(_ a: [String: Any], _ b: [String: Any]) -> Bool {
        NSDictionary(dictionary: a).isEqual(to: b)
    }

    // MARK: conversion

    private static func isBool(_ value: Any) -> Bool {
        guard let n = value as? NSNumber else { return false }
        return CFGetTypeID(n) == CFBooleanGetTypeID()
    }

    private static func syncValue(_ raw: Any, _ kind: Kind) -> Any? {
        switch kind {
        case .mascotStyles:
            guard let data = raw as? Data, let map = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
            return validStyles(map)
        default:
            return defaultsValue(raw, kind)
        }
    }

    private static func defaultsValue(_ raw: Any, _ kind: Kind) -> Any? {
        switch kind {
        case .bool:
            return isBool(raw) ? (raw as? Bool) : nil
        case .number:
            guard !isBool(raw), let n = raw as? NSNumber, n.doubleValue.isFinite else { return nil }
            return n.doubleValue
        case .choice(let allowed):
            guard let s = raw as? String, allowed.contains(s) else { return nil }
            return s
        case .mascotStyles:
            guard let map = raw as? [String: Any] else { return nil }
            return try? JSONSerialization.data(withJSONObject: validStyles(map), options: .sortedKeys)
        }
    }

    /// Keeps the entries this app can draw (`{"species": <Species>, "hue": number?}`); a species
    /// another platform added is dropped rather than resetting every style.
    private static func validStyles(_ map: [String: Any]) -> [String: Any] {
        map.filter { _, value in
            guard let style = value as? [String: Any],
                  let species = style["species"] as? String, Species(rawValue: species) != nil else { return false }
            guard let hue = style["hue"], !(hue is NSNull) else { return true }
            return hue is NSNumber && !isBool(hue)
        }
    }
}
