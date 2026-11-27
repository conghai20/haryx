import Foundation

// MARK: - Rotation verdict (`list --json` → `rotation`)

/// Will the week last? The CLI's verdict over the accounts auto-switch may land on: the room left
/// under their weekly lines, drained at the whole fleet's rate and refilled as weeks reset.
public struct Rotation: Decodable, Equatable, Sendable {
    /// Counted accounts: eligible for auto-switch and read.
    public let accounts: [Int]
    /// Eligible accounts with no weekly reading yet, left out of the verdict.
    public let unknown: [Int]
    public let headroomPct: Double
    public let burnPctPerHour: Double
    public let nextResetAt: Date?
    /// When the pool empties within the coming week; nil when it lasts.
    public let runsDryAt: Date?
    public let lasts: Bool

    public init(accounts: [Int], unknown: [Int] = [], headroomPct: Double, burnPctPerHour: Double,
                nextResetAt: Date?, runsDryAt: Date?, lasts: Bool) {
        self.accounts = accounts
        self.unknown = unknown
        self.headroomPct = headroomPct
        self.burnPctPerHour = burnPctPerHour
        self.nextResetAt = nextResetAt
        self.runsDryAt = runsDryAt
        self.lasts = lasts
    }

    /// The headline: "The week lasts" or "Runs dry Sat 10:28".
    public func headline(clock: (Date) -> String) -> String {
        guard !lasts, let runsDryAt else { return "The week lasts" }
        return "Runs dry \(clock(runsDryAt))"
    }

    /// The line under it, naming the accounts that count and the next reset.
    public func detail(clock: (Date) -> String) -> String {
        let count = accounts.count == 1 ? "1 account" : "\(accounts.count) accounts"
        let next = nextResetAt.map { "the next reset, \(clock($0))" } ?? "the next reset"
        guard !lasts, let runsDryAt else {
            return "\(count) auto-switch can use carry you past \(next)."
        }
        if let reset = nextResetAt, runsDryAt < reset {
            let hours = Int((reset.timeIntervalSince(runsDryAt) / 3600).rounded())
            return "\(hours) h before \(next), across \(count) auto-switch can use."
        }
        return "Across \(count) auto-switch can use, at the pace of the last few days."
    }
}

// MARK: - Week Tide geometry

/// The Week Tide: a weekly window drawn as the week it has been. x runs from the window's start
/// (its reset minus seven days) to its reset; y from 0 to 100%. The fill is observed use; a 1 pt
/// diagonal from (0, 0) to (1, 1) is the linear pace. Pure geometry, shared with the Windows app's
/// `weekTide` in `tide.ts`.
public enum WeekTide {
    public static let period: TimeInterval = 7 * 24 * 3600

    /// One stretch of the curve, in unit coordinates (x: 0…1 through the week, y: 0…1 of 100%).
    public struct Segment: Equatable, Sendable {
        public let points: [Point]
        /// Not read but assumed: the straight line an average draws across a stretch with no readings.
        public let estimated: Bool

        public init(points: [Point], estimated: Bool) {
            self.points = points
            self.estimated = estimated
        }
    }

    /// A point that needs no CoreGraphics in tests.
    public struct Point: Equatable, Sendable {
        public let x: Double
        public let y: Double
        public init(x: Double, y: Double) { self.x = x; self.y = y }
    }

    /// The curve for one account's week. `readings` are (time, weekly %) in any order; only those
    /// inside the current week count. The week starts at 0%; the last point is `pct` now. A stretch
    /// with no readings (before the first one kept, or a gap) is an estimated straight line.
    public static func segments(readings: [(at: Date, pct: Double)], resetsAt: Date, pct: Double,
                                now: Date) -> [Segment] {
        let start = resetsAt.addingTimeInterval(-period)
        let x = { (d: Date) in min(1, max(0, d.timeIntervalSince(start) / period)) }
        let y = { (p: Double) in min(1, max(0, p / 100)) }
        let inWeek = readings
            .filter { $0.at >= start && $0.at <= now }
            .sorted { $0.at < $1.at }
        let read = inWeek.map { (t: $0.at.timeIntervalSince1970, p: $0.pct) } + [(t: now.timeIntervalSince1970, p: pct)]
        let groups = UsageChartSupport.segments(read.map(\.t))
        let origin = Point(x: 0, y: 0)
        let toPoint = { (r: (t: TimeInterval, p: Double)) in Point(x: x(Date(timeIntervalSince1970: r.t)), y: y(r.p)) }

        var out: [Segment] = []
        var current: [Point] = []
        var group = groups.first ?? 0
        for (reading, g) in zip(read, groups) {
            if g != group, let last = current.last {
                out.append(Segment(points: current, estimated: false))
                out.append(Segment(points: [last, toPoint(reading)], estimated: true))
                current = []
                group = g
            }
            current.append(toPoint(reading))
        }
        out.append(Segment(points: current, estimated: false))

        // The stretch from the reset to the first reading: read when the first reading is close to
        // the start (a fresh week), else assumed.
        if let first = read.first {
            let lead = first.t - start.timeIntervalSince1970
            let isRead = !inWeek.isEmpty && lead <= UsageChartSupport.maxJoin
            if isRead {
                out[0] = Segment(points: [origin] + out[0].points, estimated: false)
            } else {
                out.insert(Segment(points: [origin, toPoint(first)], estimated: true), at: 0)
            }
        }
        return out.filter { $0.points.count > 1 }
    }
}
