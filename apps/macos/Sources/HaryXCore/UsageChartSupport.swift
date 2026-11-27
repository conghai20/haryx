import Foundation

/// Helpers that keep the Usage charts honest and readable: one colour per account in every chart,
/// lines that break where readings are missing, and compact axis figures. The Windows app uses the
/// same palette, gap rule and number format.
public enum UsageChartSupport {
    /// Account colours as hex RGB, one per account number, the same in every chart and table and on
    /// both apps. None is a reading colour: no cyan (live), amber or yellow (pace), red (limit), or a
    /// lime that reads as either. `palette` is for dark backgrounds, `lightPalette` (same hues,
    /// deeper) for light ones.
    public static let palette: [String] = [
        "#A78BFA", "#34D399", "#F472B6", "#60A5FA", "#D6B98C", "#6366F1", "#A8A29E", "#9CA36B",
    ]
    public static let lightPalette: [String] = [
        "#7C3AED", "#059669", "#DB2777", "#2563EB", "#8B6F47", "#4F46E5", "#78716C", "#65733A",
    ]

    /// The colour for an account number (1-based).
    public static func colorHex(account: Int, light: Bool = false) -> String {
        let list = light ? lightPalette : palette
        return list[((account - 1) % list.count + list.count) % list.count]
    }

    /// Never join two readings further apart than this, however sparse the readings are. An account
    /// that is not active is read every few minutes to an hour, so a shorter cap shreds its line.
    public static let maxJoin: TimeInterval = 2 * 60 * 60
    /// Never break between readings closer than this, however dense the readings are.
    public static let minBreak: TimeInterval = 15 * 60

    /// For readings of one series sorted by time, the segment each one belongs to. A new segment
    /// starts after a gap longer than three times the median spacing (at least `minBreak`), or
    /// longer than `maxJoin`, so a chart never draws a straight line across missing data.
    public static func segments(_ times: [TimeInterval]) -> [Int] {
        guard times.count > 1 else { return times.map { _ in 0 } }
        let gaps = zip(times.dropFirst(), times).map { $0 - $1 }
        let median = gaps.sorted()[gaps.count / 2]
        let limit = min(maxJoin, max(minBreak, median * 3))
        var segment = 0
        var result = [0]
        for gap in gaps {
            if gap > limit { segment += 1 }
            result.append(segment)
        }
        return result
    }

    /// An axis figure: 400M, 1.2B, 35K, 900. A trailing ".0" is dropped.
    public static func compact(_ value: Double) -> String {
        let magnitude = abs(value)
        let (scaled, suffix): (Double, String) = switch magnitude {
        case 1e9...: (value / 1e9, "B")
        case 1e6...: (value / 1e6, "M")
        case 1e3...: (value / 1e3, "K")
        default: (value, "")
        }
        let text = String(format: "%.1f", scaled)
        return (text.hasSuffix(".0") ? String(text.dropLast(2)) : text) + suffix
    }
}

/// Shortens a hook command for display: the home folder as ~, and a long command reduced to the
/// name of the script it runs plus its arguments. The full command goes in the tooltip.
public enum HookSummary {
    public static func tilde(_ command: String, home: String = NSHomeDirectory()) -> String {
        command.replacingOccurrences(of: home, with: "~")
    }

    public static func short(_ command: String, home: String = NSHomeDirectory(), limit: Int = 60) -> String {
        let text = tilde(command, home: home)
        guard text.count > limit else { return text }
        let tokens = text.split(separator: " ").map { $0.trimmingCharacters(in: CharacterSet(charactersIn: "'\"")) }
        guard let pathIndex = tokens.lastIndex(where: { $0.contains("/") }) else { return String(text.prefix(limit)) + "…" }
        let name = (tokens[pathIndex] as NSString).lastPathComponent
        let args = tokens[(pathIndex + 1)...].joined(separator: " ")
        return args.isEmpty ? name : "\(name) \(args)"
    }
}
