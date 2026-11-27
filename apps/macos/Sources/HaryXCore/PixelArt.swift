import CoreGraphics
import Foundation

public enum Mood: String, Sendable, Hashable {
    case idle, running, angry, tired, sleeping, error

    static let angryPct = 75.0
    static let tiredPct = 90.0

    /// Held out of rotation → sleeping, signed out → error, at its limit → sleeping, a Claude session
    /// on it → running, 90% or more of a quota → tired, 75% or more → angry.
    public static func of(_ a: Account, running: Bool) -> Mood {
        if a.isDisabled { return .sleeping }
        if a.needsRelogin { return .error }
        if a.isExhausted { return .sleeping }
        if running { return .running }
        let peak = a.peakPct ?? 0
        if peak >= tiredPct { return .tired }
        if peak >= angryPct { return .angry }
        return .idle
    }
}

/// One animation frame. Derived purely from mood + clock so views need no timers of their own.
public struct Pose: Sendable, Hashable {
    public var altLegs = false
    public var squat = false
    public var blink = false
    public var overlayPhase = 0
    public var mood: Mood = .idle

    public init(altLegs: Bool = false, squat: Bool = false, blink: Bool = false, overlayPhase: Int = 0, mood: Mood = .idle) {
        self.altLegs = altLegs; self.squat = squat; self.blink = blink; self.overlayPhase = overlayPhase; self.mood = mood
    }

    public static let idleFPS = 4.0
    public static let runningFPS = 8.0
    static let blinkEvery = 14

    public static func fps(for mood: Mood) -> Double { mood == .running ? runningFPS : idleFPS }

    /// `phase` de-synchronises mascots so they don't blink in unison.
    public static func at(_ time: TimeInterval, mood: Mood, phase: Int) -> Pose {
        let tick = Int(time * fps(for: mood)) + phase
        switch mood {
        case .running:
            return Pose(altLegs: tick % 2 == 1, squat: tick % 2 == 0, overlayPhase: tick, mood: mood)
        case .sleeping:
            return Pose(squat: (tick / 4) % 2 == 1, blink: true, overlayPhase: tick / 3, mood: mood)
        case .angry: // stomping, the mark throbbing in time
            return Pose(squat: tick % 2 == 0, overlayPhase: tick, mood: mood)
        case .idle, .tired, .error:
            return Pose(squat: (tick / 2) % 2 == 1, blink: tick % blinkEvery == 0, overlayPhase: tick / 2, mood: mood)
        }
    }
}

public struct Palette: Sendable, Hashable {
    public let body: RGB, shade: RGB, light: RGB

    public init(species: Species, hue: Double?) {
        let base = species.baseHSB
        let h = hue ?? base.h
        body = RGB(h: h, s: base.s, b: base.b)
        shade = RGB(h: h, s: min(1, base.s + 0.15), b: base.b * 0.68)
        light = RGB(h: h, s: base.s * 0.35, b: min(1, base.b + 0.12))
    }
}

/// Renders sprites into CGImages. Callers cache by `(species, palette, pose, scale)`.
public enum PixelArt {
    public static let canvas = 18
    static let blush = RGB(1.0, 0.55, 0.62)
    static let dark = RGB(0.12, 0.10, 0.12)
    static let white = RGB(1, 1, 1)
    static let sweat = RGB(0.45, 0.78, 1.0)
    static let alert = RGB(1.0, 0.27, 0.23)

    /// Returns the pixel grid (row-major, `canvas`×`canvas`) as optional colours.
    public static func pixels(_ species: Species, palette: Palette, pose: Pose) -> [[RGB?]] {
        var grid = [[RGB?]](repeating: [RGB?](repeating: nil, count: canvas), count: canvas)
        let body = species.body
        let legs = pose.altLegs ? species.legs.b : species.legs.a
        let top = canvas - body.count - legs.count
        let bodyTop = top + (pose.squat ? 1 : 0)

        func color(_ ch: Character, row: [Character], below: [Character]?, col: Int) -> RGB? {
            switch ch {
            case "B": palette.body
            case "D": palette.shade
            case "L": palette.light
            case "W": white
            case "P": blush
            case "K": dark
            case "A": species.accent
            case "E":
                // Closed eye: body colour, with a lash line on the eye's lowest row.
                !pose.blink ? species.eye : (below.map { $0[col] == "E" } == true ? palette.body : palette.shade)
            default: nil
            }
        }

        func stamp(_ rows: [String], at y0: Int) {
            let chars = rows.map(Array.init)
            for (r, row) in chars.enumerated() {
                let y = y0 + r
                guard y >= 0, y < canvas else { continue }
                for (c, ch) in row.enumerated() {
                    guard let rgb = color(ch, row: row, below: r + 1 < chars.count ? chars[r + 1] : nil, col: c) else { continue }
                    grid[y][c + 1] = rgb
                }
            }
        }
        stamp(body, at: bodyTop)
        stamp(legs, at: canvas - legs.count)
        overlay(&grid, pose: pose, top: top)
        return grid
    }

    static func overlay(_ grid: inout [[RGB?]], pose: Pose, top: Int) {
        func put(_ x: Int, _ y: Int, _ c: RGB) { if y >= 0, y < canvas, x >= 0, x < canvas { grid[y][x] = c } }
        switch pose.mood {
        case .tired: // sweat drop sliding down beside the head
            let y = top + 1 + pose.overlayPhase % 3
            put(1, y, sweat); put(1, y + 1, sweat); put(0, y + 1, sweat)
        case .sleeping: // drifting "z"
            let y = max(0, top - 4) + (pose.overlayPhase % 2)
            let x = 13
            for dx in 0..<4 { put(x + dx, y, white); put(x + dx, y + 3, white) }
            put(x + 2, y + 1, white); put(x + 1, y + 2, white)
        case .angry: // a throbbing anger mark
            let y = max(0, top - 5)
            let marks = pose.overlayPhase % 2 == 0
                ? [(13, 0), (15, 0), (12, 1), (13, 1), (15, 1), (16, 1), (12, 3), (13, 3), (15, 3), (16, 3), (13, 4), (15, 4)]
                : [(13, 1), (15, 1), (13, 3), (15, 3)]
            for (x, dy) in marks { put(x, y + dy, alert) }
        case .error: // blinking "!"
            if pose.overlayPhase % 2 == 0 {
                let y = max(0, top - 4)
                put(16, y, alert); put(16, y + 1, alert); put(16, y + 3, alert)
            }
        case .running: // little speed dust behind the feet
            if pose.altLegs { put(0, canvas - 1, white.dimmed(0.6)) } else { put(1, canvas - 2, white.dimmed(0.6)) }
        case .idle:
            break
        }
    }

    public static func image(_ species: Species, palette: Palette, pose: Pose, scale: Int) -> CGImage? {
        draw(pixels(species, palette: palette, pose: pose), scale: scale, background: nil)
    }

    /// Draws a grid at an integer scale (crisp pixels), optionally onto a pre-drawn background.
    public static func draw(_ grid: [[RGB?]], scale: Int, background: ((CGContext, Int) -> Void)?, origin: (x: Int, y: Int) = (0, 0), size: Int? = nil) -> CGImage? {
        let side = size ?? canvas * scale
        guard let ctx = CGContext(data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        ctx.interpolationQuality = .none
        ctx.setShouldAntialias(false)
        background?(ctx, side)
        for (y, row) in grid.enumerated() {
            for (x, px) in row.enumerated() {
                guard let px else { continue }
                ctx.setFillColor(CGColor(srgbRed: px.r, green: px.g, blue: px.b, alpha: 1))
                // CG's origin is bottom-left; grid row 0 is the top.
                ctx.fill(CGRect(x: origin.x + x * scale, y: side - origin.y - (y + 1) * scale, width: scale, height: scale))
            }
        }
        return ctx.makeImage()
    }
}

extension RGB {
    func dimmed(_ f: Double) -> RGB { RGB(r * f, g * f, b * f) }
}
