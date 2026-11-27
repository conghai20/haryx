import Foundation

extension UsageWindow {
    /// Points from the pace that count as off it: smaller drifts read "on pace".
    public static let paceThreshold = 5

    /// Points ahead (+) or behind (−) the expected pace, when both are known.
    public var paceDelta: Int? {
        guard let pct, let expectedPct else { return nil }
        return Int((pct - expectedPct).rounded())
    }
}

/// A card's weekly pace line: how far the week is from its expected pace, and, when it is ahead and
/// the CLI projects it will not last to the reset, when it runs out. Read only from the CLI's
/// expected pace, which it leaves out for the first day after a reset (any use then reads far ahead).
public struct WeeklyPace: Equatable, Sendable {
    public enum Drift: Equatable, Sendable {
        case ahead(Int)
        case under(Int)
        case onPace
    }

    public let drift: Drift
    public let runsOutAt: Date?

    public init?(_ window: UsageWindow?) {
        guard let window, window.expectedPct != nil, let delta = window.paceDelta else { return nil }
        if delta >= UsageWindow.paceThreshold {
            drift = .ahead(delta)
            runsOutAt = window.willLastToReset == false ? window.projectedExhaustionAt : nil
        } else if delta <= -UsageWindow.paceThreshold {
            drift = .under(-delta)
            runsOutAt = nil
        } else {
            drift = .onPace
            runsOutAt = nil
        }
    }

    public var isAhead: Bool {
        if case .ahead = drift { return true }
        return false
    }

    /// "14 pts ahead of pace · runs out Sat 10:15", "9 pts under pace" or "on pace".
    public func text(runOut format: (Date) -> String) -> String {
        let head = switch drift {
        case .ahead(let points): "\(points) pts ahead of pace"
        case .under(let points): "\(points) pts under pace"
        case .onPace: "on pace"
        }
        guard let runsOutAt else { return head }
        return "\(head) · runs out \(format(runsOutAt))"
    }
}
