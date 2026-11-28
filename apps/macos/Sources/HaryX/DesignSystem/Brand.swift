import AppKit
import HaryXCore
import SwiftUI

/// The HaryX brand on native surfaces. The app accent (Resources/Assets.xcassets, `Color.accentColor`)
/// is a deep Electric Cyan that carries white text; Electric Cyan itself marks data and live states, Midnight Navy for the mark and
/// selected emphasis, Space Grotesk for display moments only. Everything else stays system.
enum Brand {
    static let navyHex = 0x101827
    static let cyanHex = 0x22D3EE
    static let softWhiteHex = 0xF1F5F9

    /// Electric Cyan in dark; a deeper cyan in light so it holds contrast on white.
    static let cyan = Color(nsColor: .dynamic(light: 0x0891B2, dark: cyanHex))
    /// Cyan for text: 4.5:1 or better on light glass down to #DCDCDC (5.3:1 there).
    static let cyanText = Color(nsColor: .dynamic(light: 0x155E75, dark: cyanHex))
    static let navy = Color(hex: navyHex)
    static let softWhite = Color(hex: softWhiteHex)
    /// Ahead of pace or near a limit.
    static let warn = Color.orange
    /// At a limit.
    static let limit = Color.red

    /// Space Grotesk, bundled in the app (Info.plist ATSApplicationFontsPath). Falls back to the
    /// system face when the font is missing (a `swift run` build).
    static func display(_ size: CGFloat, weight: Font.Weight = .semibold, relativeTo style: Font.TextStyle = .title) -> Font {
        Font.custom("Space Grotesk", size: size, relativeTo: style).weight(weight)
    }

    /// Fill for a usage reading: cyan while on pace, warning ahead of pace or past 85%, red at the limit.
    /// The Three-Step Fill Rule. `off` (an account held out of rotation) trades cyan for grey, since
    /// cyan means live; amber and red stay, because they are why an Off account is still tracked.
    static func usageFill(_ window: UsageWindow?, off: Bool = false) -> Color {
        guard let pct = window?.pct else { return .secondary }
        if pct >= 100 { return limit }
        if pct >= 85 || window?.isAheadOfPace == true { return warn }
        return off ? .secondary : cyan
    }
}

extension Color {
    init(hex: Int) {
        self.init(.sRGB,
                  red: Double((hex >> 16) & 0xFF) / 255,
                  green: Double((hex >> 8) & 0xFF) / 255,
                  blue: Double(hex & 0xFF) / 255)
    }
}

extension NSColor {
    static func dynamic(light: Int, dark: Int) -> NSColor {
        NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return NSColor(hex: isDark ? dark : light)
        }
    }

    convenience init(hex: Int) {
        self.init(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
                  green: CGFloat((hex >> 8) & 0xFF) / 255,
                  blue: CGFloat(hex & 0xFF) / 255,
                  alpha: 1)
    }
}

// MARK: - Wing H mark

/// The Wing H mark: two point-symmetric halves interlocking as an X (assets/brand/haryx-mark.svg).
enum WingH {
    /// One half in the SVG's local coordinates (before its `scale(1.05 1.2)`).
    private static func half(_ p: inout Path) {
        p.move(to: CGPoint(x: -16, y: -19))
        p.addLine(to: CGPoint(x: -9.1, y: -19))
        p.addLine(to: CGPoint(x: -9.1, y: -11.7))
        p.addLine(to: CGPoint(x: -3.8, y: -5.4))
        p.addCurve(to: CGPoint(x: 16, y: -19), control1: CGPoint(x: 3, y: -10.5), control2: CGPoint(x: 11.5, y: -12.8))
        p.addCurve(to: CGPoint(x: -4.5, y: 2.5), control1: CGPoint(x: 14, y: -12.5), control2: CGPoint(x: 7, y: -6.2))
        p.addLine(to: CGPoint(x: -16, y: -11.7))
        p.closeSubpath()
    }

    /// The upper half (Electric Cyan on the tile) and the lower half (Soft White), in a 64×64 box.
    static func halves() -> (upper: Path, lower: Path) {
        var local = Path()
        half(&local)
        let scale = CGAffineTransform(scaleX: 1.05, y: 1.2)
        let place = CGAffineTransform(scaleX: 1.228, y: 1.228).concatenating(CGAffineTransform(translationX: 32, y: 32))
        let upper = local.applying(scale.concatenating(place))
        let lower = local.applying(scale.concatenating(CGAffineTransform(rotationAngle: .pi)).concatenating(place))
        return (upper, lower)
    }

    /// A template image for the status item (16 pt), drawn once and cached by the caller.
    /// `running` adds a dot at the lower right.
    static func templateImage(size: CGFloat = 16, running: Bool = false) -> NSImage {
        let width = running ? size + 5 : size
        let image = NSImage(size: NSSize(width: width, height: size), flipped: true) { _ in
            let (upper, lower) = halves()
            let fit = CGAffineTransform(scaleX: size / 64, y: size / 64)
            NSColor.black.setFill()
            for half in [upper, lower] {
                NSBezierPath(cgPath: half.applying(fit).cgPath).fill()
            }
            if running {
                NSBezierPath(ovalIn: NSRect(x: size + 0.5, y: size - 5, width: 4, height: 4)).fill()
            }
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "HaryX"
        return image
    }
}

/// The mark on its Midnight Navy tile.
struct WingHTile: View {
    var size: CGFloat = 20

    var body: some View {
        let (upper, lower) = WingH.halves()
        Canvas { ctx, canvas in
            let fit = CGAffineTransform(scaleX: canvas.width / 64, y: canvas.height / 64)
            ctx.fill(upper.applying(fit), with: .color(Color(hex: Brand.cyanHex)))
            ctx.fill(lower.applying(fit), with: .color(Brand.softWhite))
        }
        .padding(size * 0.12)
        .frame(width: size, height: size)
        .background(Brand.navy, in: .rect(cornerRadius: size * 0.24))
        .accessibilityHidden(true)
    }
}

/// HARYX lockup: the tile and the wordmark set in Space Grotesk.
struct BrandLockup: View {
    var size: CGFloat = 18

    var body: some View {
        HStack(spacing: size * 0.4) {
            WingHTile(size: size)
            Text("HARYX")
                .font(Brand.display(size * 0.72, weight: .bold, relativeTo: .headline))
                .tracking(size * 0.08)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("HaryX")
    }
}

// MARK: - Tide staff

/// A 5-hour (or weekly) window read like a tide gauge: the flat fill is observed use, the 1 pt tick
/// is where the pace says use should be by now.
struct TideStaff: View {
    let window: UsageWindow?
    var height: CGFloat = 6
    /// An Off account's reading: grey where it would be cyan.
    var off = false
    /// How far the pace tick reaches above and below the bar.
    private let overhang: CGFloat = 3

    var body: some View {
        let pct = min(1, max(0, (window?.pct ?? 0) / 100))
        let expected = window?.expectedPct.map { min(1, max(0, $0 / 100)) }
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                ZStack(alignment: .leading) {
                    Rectangle().fill(.quaternary)
                    Rectangle().fill(Brand.usageFill(window, off: off)).frame(width: geo.size.width * pct)
                }
                .frame(height: height)
                .clipShape(.rect(cornerRadius: 1.5))
                if let expected {
                    Rectangle().fill(.primary).frame(width: 1, height: height + overhang * 2)
                        .offset(x: geo.size.width * expected - 0.5)
                }
            }
            .frame(height: geo.size.height)
        }
        .frame(height: height + overhang * 2)
        .accessibilityHidden(true)
    }
}

/// The weekly window under a 5-hour reading: "Week 79%", its reset, and under them either a thin
/// staff with the pace tick (the menu bar panel) or, given the week's readings, its Week Tide.
struct WeekStaff: View {
    let window: UsageWindow?
    var off = false
    /// The week is the tighter limit right now: its label turns amber below the 85% step too, so
    /// the 5-hour figure above it does not hide it (the menu bar panel).
    var tighter = false
    /// The week's readings: draws the Week Tide in place of the thin staff. Empty draws the average.
    var readings: [(at: Date, pct: Double)]?

    var body: some View {
        let week = window?.withPace(windowSeconds: UsageWindow.sevenDaySeconds)
        let high = (week?.pct ?? 0) >= 85
        VStack(alignment: .leading, spacing: 1) {
            HStack {
                Text("Week \(week?.pct.map { "\(Int($0))%" } ?? "—")")
                    .foregroundStyle(high ? AnyShapeStyle(Brand.usageFill(week, off: off))
                                     : tighter ? AnyShapeStyle(Brand.warn) : AnyShapeStyle(.secondary))
                Spacer()
                if let c = week?.countdown { Text("resets in \(c)").foregroundStyle(.secondary) }
            }
            .font(.caption.monospacedDigit())
            if let readings {
                WeekTideView(window: week, readings: readings, off: off).padding(.top, 2)
            } else {
                TideStaff(window: week, height: 3, off: off)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Weekly usage \(week?.pct.map { "\(Int($0)) percent" } ?? "unknown")\(week?.countdown.map { ", resets in \($0)" } ?? "")")
    }
}

/// The Week Tide (DESIGN.md): the week so far as a filled curve over a quaternary track, coloured by
/// the Three-Step Fill Rule, against a 1 pt diagonal that is the linear pace. Stretches with no
/// readings are an estimated straight line, dashed and lighter. Nothing animates.
struct WeekTideView: View {
    let window: UsageWindow?
    let readings: [(at: Date, pct: Double)]
    var off = false
    var height: CGFloat = 16

    var body: some View {
        let fill = Brand.usageFill(window, off: off)
        Canvas { ctx, size in
            let track = Path(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: 1.5)
            ctx.fill(track, with: .style(.quaternary))
            ctx.clip(to: track)
            var pace = Path()
            pace.move(to: CGPoint(x: 0, y: size.height))
            pace.addLine(to: CGPoint(x: size.width, y: 0))
            ctx.stroke(pace, with: .style(.secondary), lineWidth: 1)
            for segment in segments {
                let points = segment.points.map { CGPoint(x: $0.x * size.width, y: (1 - $0.y) * size.height) }
                guard let first = points.first, let last = points.last else { continue }
                var line = Path()
                line.addLines(points)
                var area = line
                area.addLine(to: CGPoint(x: last.x, y: size.height))
                area.addLine(to: CGPoint(x: first.x, y: size.height))
                area.closeSubpath()
                ctx.fill(area, with: .color(fill.opacity(segment.estimated ? 0.14 : 0.32)))
                ctx.stroke(line, with: .color(fill),
                           style: StrokeStyle(lineWidth: 1.25, lineCap: .round, lineJoin: .round,
                                              dash: segment.estimated ? [2, 2] : []))
            }
        }
        .frame(height: height)
        .accessibilityHidden(true)
    }

    private var segments: [WeekTide.Segment] {
        guard let pct = window?.pct, let resetsAt = window?.resetsAt else { return [] }
        return WeekTide.segments(readings: readings, resetsAt: resetsAt, pct: pct, now: Date())
    }
}

extension UsageWindow {
    /// This window with an expected pace: the CLI's when it sent one, else linear over the window
    /// (elapsed share of `windowSeconds`, from the reset time).
    func withPace(windowSeconds: TimeInterval, now: Date = Date()) -> UsageWindow {
        guard expectedPct == nil, let resetsAt else { return self }
        let elapsed = 1 - max(0, min(1, resetsAt.timeIntervalSince(now) / windowSeconds))
        return UsageWindow(pct: pct, resetsAt: resetsAt, countdown: countdown, clock: clock,
                           expectedPct: elapsed * 100, aheadOfPace: aheadOfPace,
                           projectedExhaustionAt: projectedExhaustionAt, willLastToReset: willLastToReset)
    }

    static let fiveHourSeconds: TimeInterval = 5 * 3600
    static let sevenDaySeconds: TimeInterval = 7 * 24 * 3600

    /// Ahead by at least ``UsageWindow/paceThreshold`` points, else the CLI's own verdict.
    var isAheadOfPace: Bool {
        if let d = paceDelta { return d >= Self.paceThreshold }
        return aheadOfPace == true
    }
}
