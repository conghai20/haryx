import Foundation

/// `haryx ai profile` payloads (contracts in `packages/contracts/haryx/profile-*.json`) and the
/// argument lists the app sends. Profiles exist only in the `haryx` backend, not in cswap.
public enum ProfileJSON {
    public static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    /// The profile that is always the live ~/.claude.
    public static let defaultName = "default"
}

public struct ProfileSummary: Decodable, Sendable, Identifiable, Hashable {
    public let name: String
    public let createdAt: String
    public let categories: [String]
    public let totalBytes: UInt64
    public let secrets: String
    public let plugins: String
    public let hasCredentials: Bool
    public let sourceOs: String
    public let lastApplied: Bool
    public var id: String { name }
}

public struct ProfileList: Decodable, Sendable {
    public let schemaVersion: Int
    public let profiles: [ProfileSummary]
}

public struct ProfileItem: Decodable, Sendable, Identifiable, Hashable {
    public let category: String
    public let id: String
    public let kind: String
    public let size: UInt64
    public let files: Int
    public let status: String
    public let detail: [String]

    public var isProblem: Bool { status != "ok" }
}

public struct ProfileInventory: Decodable, Sendable {
    public let schemaVersion: Int
    public let profile: String
    public let items: [ProfileItem]

    /// Items grouped by category, in the CLI's category order.
    public var grouped: [(category: String, items: [ProfileItem])] {
        ProfileCategory.all.compactMap { cat in
            let inCat = items.filter { $0.category == cat.name }
            return inCat.isEmpty ? nil : (cat.name, inCat)
        }
    }
}

public struct ProfilePlanEntry: Decodable, Sendable, Identifiable, Hashable {
    public let category: String
    public let id: String
    public let action: String
    public let note: String?
    public let needsSecret: [String]

    public var changes: Bool { ["add", "update", "merge", "remove"].contains(action) }
}

public struct ProfilePlan: Decodable, Sendable {
    public let schemaVersion: Int
    public let profile: String
    public let prune: Bool
    public let entries: [ProfilePlanEntry]
    public let runsCode: [String]
    public let missingRequires: [String]
    public let warnings: [String]

    public var changes: [ProfilePlanEntry] { entries.filter(\.changes) }
    public var unchangedCount: Int { entries.count - changes.count }
}

public struct ProfileDiffItem: Decodable, Sendable, Identifiable, Hashable {
    public let category: String
    public let id: String
    /// `only-left`, `only-right`, `differs` or `same`.
    public let state: String

    /// The state in words, as the Windows app shows it ("left" is the saved profile, "right" live).
    public func stateText(left: String) -> String {
        switch state {
        case "only-left": "only in \(left)"
        case "only-right": "only live"
        default: state
        }
    }
}

/// `haryx ai profile diff NAME --json`: a saved profile (left) against the live ~/.claude (right).
public struct ProfileDiff: Decodable, Sendable {
    public let schemaVersion: Int
    public let left: String
    public let right: String
    public let items: [ProfileDiffItem]

    public var differences: [ProfileDiffItem] { items.filter { $0.state != "same" } }
    public var sameCount: Int { items.count - differences.count }
}

/// A part of the setup, mirroring the CLI's categories.
public struct ProfileCategory: Sendable, Hashable, Identifiable {
    public let name: String
    public let title: String
    public let defaultOn: Bool
    public var id: String { name }

    public static let all: [ProfileCategory] = [
        .init(name: "settings", title: "Settings", defaultOn: true),
        .init(name: "hooks", title: "Hooks", defaultOn: true),
        .init(name: "skills", title: "Skills", defaultOn: true),
        .init(name: "agents", title: "Agents", defaultOn: true),
        .init(name: "commands", title: "Commands", defaultOn: true),
        .init(name: "rules", title: "Rules", defaultOn: true),
        .init(name: "output-styles", title: "Output Styles", defaultOn: true),
        .init(name: "plugins", title: "Plugins", defaultOn: true),
        .init(name: "mcps", title: "MCP Servers", defaultOn: true),
        .init(name: "memory", title: "Memory (CLAUDE.md)", defaultOn: true),
        .init(name: "other", title: "Other Files", defaultOn: true),
        .init(name: "history", title: "History", defaultOn: true),
        .init(name: "credentials", title: "Credentials", defaultOn: false),
        .init(name: "signed", title: "Signed Files", defaultOn: false),
        .init(name: "claude-json", title: "Whole ~/.claude.json", defaultOn: false),
        .init(name: "cache", title: "Caches", defaultOn: false),
    ]

    public static func title(of name: String) -> String {
        all.first { $0.name == name }?.title ?? name
    }
}

/// What to put in an export or a save. Defaults match the CLI's.
public struct ProfileExportOptions: Sendable, Equatable {
    public var encrypt = false
    public var bundlePlugins = false
    public var includeHistory = true
    public var credentials = false
    public var signed = false
    public var claudeJSON = false
    public var cache = false
    public var includeDirs: [String] = []

    public init() {}

    var partArgs: [String] {
        var args: [String] = []
        if !includeHistory { args.append("--no-history") }
        if credentials { args.append("--credentials") }
        if signed { args.append("--signed") }
        if claudeJSON { args.append("--claude-json") }
        if cache { args.append("--cache") }
        for dir in includeDirs { args += ["--include-dir", dir] }
        return args
    }

    /// Credentials can only travel encrypted.
    public var isValid: Bool { !credentials || encrypt }
}

/// Argument lists for `haryx ai profile …`.
public enum ProfileArgs {
    public static func list() -> [String] { ["ai", "profile", "list", "--json"] }

    public static func show(_ name: String) -> [String] { ["ai", "profile", "show", name, "--json"] }

    public static func save(_ name: String, _ o: ProfileExportOptions, force: Bool) -> [String] {
        ["ai", "profile", "save", name] + o.partArgs
            + ["--plugins", o.bundlePlugins ? "bundle" : "ref"] + (force ? ["--force"] : [])
    }

    public static func export(_ name: String, to file: String, _ o: ProfileExportOptions) -> [String] {
        ["ai", "profile", "export", name, file] + o.partArgs
            + ["--secrets", o.encrypt ? "encrypt" : "redact", "--plugins", o.bundlePlugins ? "bundle" : "ref"]
    }

    public static func importFile(_ name: String, from file: String, force: Bool) -> [String] {
        ["ai", "profile", "import", name, file] + (force ? ["--force"] : [])
    }

    public static func plan(_ name: String, prune: Bool, includeHistory: Bool) -> [String] {
        ["ai", "profile", "apply", name, "--dry-run", "--json"]
            + (prune ? ["--prune"] : []) + (includeHistory ? [] : ["--no-history"])
    }

    /// The app always confirms in its own sheet first, so the CLI gets `--yes`.
    public static func apply(_ name: String, prune: Bool, includeHistory: Bool) -> [String] {
        ["ai", "profile", "apply", name, "--yes"]
            + (prune ? ["--prune"] : []) + (includeHistory ? [] : ["--no-history"])
    }

    public static func diff(_ name: String) -> [String] { ["ai", "profile", "diff", name, "--json"] }

    public static func delete(_ name: String) -> [String] { ["ai", "profile", "delete", name, "--yes"] }

    public static func rollback() -> [String] { ["ai", "profile", "rollback"] }

    /// The environment variable that carries a passphrase to the CLI (never an argument).
    public static let passphraseEnv = "HARYX_PROFILE_PASSPHRASE"

    /// A name the CLI accepts: letters, digits, `.`, `_`, `-`; not hidden; not `default`.
    public static func isValidName(_ name: String) -> Bool {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "._-"))
        return !name.isEmpty && name.count <= 64 && !name.hasPrefix(".") && name != ProfileJSON.defaultName
            && name.unicodeScalars.allSatisfy { allowed.contains($0) && $0.isASCII }
    }
}

/// `1.2 MB` style sizes, matching the CLI.
public func profileByteString(_ bytes: UInt64) -> String {
    ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .binary)
}

extension ProfileInventory {
    /// A category's collapsed line: how many items, their size on disk, and how many need attention.
    public static func summary(_ items: [ProfileItem]) -> String {
        let bytes = items.filter { $0.files > 0 }.reduce(UInt64(0)) { $0 + $1.size }
        let problems = items.filter(\.isProblem).count
        var parts = ["\(items.count) item\(items.count == 1 ? "" : "s")"]
        if bytes > 0 { parts.append(profileByteString(bytes)) }
        if problems > 0 { parts.append("\(problems) need\(problems == 1 ? "s" : "") attention") }
        return parts.joined(separator: " · ")
    }

    /// Lines in first-seen order, each once, with how many times it appeared.
    public static func counted(_ lines: [String]) -> [(line: String, count: Int)] {
        lines.reduce(into: [(line: String, count: Int)]()) { acc, line in
            if let i = acc.firstIndex(where: { $0.line == line }) {
                acc[i].count += 1
            } else {
                acc.append((line, 1))
            }
        }
    }
}
