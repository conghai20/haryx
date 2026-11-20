import Foundation

/// Pixel legend: `.` clear · `B` body · `D` shade · `L` light · `E` eye · `W` white
/// `P` blush · `K` dark detail · `A` species accent. All rows are 16 wide.
public enum Species: String, CaseIterable, Sendable, Identifiable, Codable {
    case critter, cat, ghost, robot, frog, slime, chick, bunny

    public var id: String { rawValue }
    public var title: String { rawValue.capitalized }

    /// Default body colour as HSB (0...1). Account colour overrides only the hue.
    public var baseHSB: (h: Double, s: Double, b: Double) {
        switch self {
        case .critter: (0.045, 0.62, 0.85)
        case .cat: (0.08, 0.55, 0.95)
        case .ghost: (0.72, 0.18, 0.97)
        case .robot: (0.58, 0.30, 0.78)
        case .frog: (0.33, 0.62, 0.75)
        case .slime: (0.48, 0.60, 0.80)
        case .chick: (0.14, 0.75, 0.98)
        case .bunny: (0.95, 0.22, 0.98)
        }
    }

    public var accent: RGB {
        switch self {
        case .robot: RGB(0.35, 0.95, 1.0)
        case .chick: RGB(1.0, 0.55, 0.15)
        case .cat, .bunny: RGB(1.0, 0.62, 0.72)
        default: RGB(1, 1, 1)
        }
    }

    /// Robot eyes glow on a dark visor; everyone else has dark eyes.
    public var eye: RGB { self == .robot ? accent : RGB(0.09, 0.07, 0.08) }

    public var body: [String] {
        switch self {
        case .critter: [
            "...BBBBBBBBBB...",
            "...BBBBBBBBBB...",
            "...BBEBBBBEBB...",
            "...BBEBBBBEBB...",
            ".BBBBBBBBBBBBBB.",
            ".BBBBBBBBBBBBBB.",
            "...BBBBBBBBBB...",
            "...BBBBBBBBBB...",
        ]
        case .cat: [
            "..B..........B..",
            "..BB........BB..",
            "..BAB......BAB..",
            "..BBBBBBBBBBBB..",
            ".BBBBBBBBBBBBBB.",
            ".BBBEEBBBBEEBBB.",
            ".BBBEEBBBBEEBBB.",
            ".BPPBBBKKBBBPPB.",
            ".BBBBBBBBBBBBBB.",
            "..BBBBBBBBBBBB..",
            "..BBLLLLLLLLBB..",
            "..BBLLLLLLLLBB..",
        ]
        case .ghost: [
            ".....BBBBBB.....",
            "...BBBBBBBBBB...",
            "..BBBBBBBBBBBB..",
            "..BLBBBBBBBBBB..",
            ".BBBEEBBBBEEBBB.",
            ".BBBEEBBBBEEBBB.",
            ".BBBBBBBBBBBBBB.",
            ".BBPPBBKKBBPPBB.",
            ".BBBBBBBBBBBBBB.",
            ".BBBBBBBBBBBBBB.",
            ".DBBBBBBBBBBBBD.",
        ]
        case .robot: [
            ".......AA.......",
            ".......BB.......",
            "..BBBBBBBBBBBB..",
            "..BKKKKKKKKKKB..",
            "..BKEEKKKKEEKB..",
            "..BKEEKKKKEEKB..",
            "..BKKKKKKKKKKB..",
            "..BBBBBBBBBBBB..",
            "....DDDDDDDD....",
            "..BBBLLLLLLBBB..",
            "..B.BLLAALLB.B..",
            "..B.BBBBBBBB.B..",
        ]
        case .frog: [
            "..BBBB....BBBB..",
            ".BBWWBB..BBWWBB.",
            ".BBWEBB..BBWEBB.",
            ".BBBBBBBBBBBBBB.",
            "BBBBBBBBBBBBBBBB",
            "BBPPBBBBBBBBPPBB",
            "BBBBKKKKKKKKBBBB",
            ".BBBBBBBBBBBBBB.",
            "..BBLLLLLLLLBB..",
            "..BBLLLLLLLLBB..",
        ]
        case .slime: [
            "......BBBB......",
            "....BBLLBBBB....",
            "...BBLBBBBBBB...",
            "..BBBBBBBBBBBB..",
            "..BBBEEBBEEBBB..",
            ".BBBBEEBBEEBBBB.",
            ".BBPPBBBBBBPPBB.",
            "BBBBBBBKKBBBBBBB",
            "BBBBBBBBBBBBBBBB",
            "BDBBBBBBBBBBBBDB",
        ]
        case .chick: [
            ".......BB.......",
            "......BB........",
            "....BBBBBBBB....",
            "...BBBBBBBBBB...",
            "...BBEBBBBEBB...",
            "..BBBEBAABEBBB..",
            "..BBBBBAABBBBB..",
            "..LBBBBBBBBBBL..",
            "..LLBBBBBBBBLL..",
            "...BBBBBBBBBB...",
            "....BBBBBBBB....",
        ]
        case .bunny: [
            "....BB....BB....",
            "....BA....AB....",
            "....BA....AB....",
            "....BB....BB....",
            "...BBBBBBBBBB...",
            "..BBBBBBBBBBBB..",
            "..BBEBBBBBBEBB..",
            "..BBEBBBBBBEBB..",
            "..BPPBBKKBBPPB..",
            "..BBBBBBBBBBBB..",
            "...BBLLLLLLBB...",
            "...BBLLLLLLBB...",
        ]
        }
    }

    /// Two leg frames; running alternates them.
    public var legs: (a: [String], b: [String]) {
        switch self {
        case .critter: (["....B.B..B.B....", "....B.B..B.B...."],
                        ["...B.B....B.B...", "....B.B..B.B...."])
        case .cat: (["..BB.BB..BB.BB.."], [".BB..BB..BB..BB."])
        case .ghost: ([".BB.BBB..BBB.BB."], [".B.BBB.BB.BBB.B."])
        case .robot: ([".....DD..DD....."], ["....DD....DD...."])
        case .frog: ([".BBB........BBB."], ["BBB..........BBB"])
        case .slime: ([".DDDDDDDDDDDDDD."], ["DDDDDDDDDDDDDDDD"])
        case .chick: ([".....A....A.....", "....AA....AA...."],
                      ["....A......A....", "...AA......AA..."])
        case .bunny: (["...BB......BB..."], ["....BB....BB...."])
        }
    }

    /// Deterministic default so each account keeps the same mascot without configuration.
    public static func defaultFor(key: String) -> Species {
        allCases[Int(stableHash(key) % UInt64(allCases.count))]
    }
}

/// FNV-1a: `String.hashValue` is randomised per launch, which would reshuffle mascots.
public func stableHash(_ s: String) -> UInt64 {
    s.utf8.reduce(14_695_981_039_346_656_037) { ($0 ^ UInt64($1)) &* 1_099_511_628_211 }
}

public struct RGB: Sendable, Hashable {
    public let r: Double, g: Double, b: Double
    public init(_ r: Double, _ g: Double, _ b: Double) { self.r = r; self.g = g; self.b = b }

    public init(h: Double, s: Double, b v: Double) {
        let h6 = (h - floor(h)) * 6
        let c = v * s, x = c * (1 - abs(h6.truncatingRemainder(dividingBy: 2) - 1)), m = v - c
        let (r, g, b): (Double, Double, Double) = switch Int(h6) {
        case 0: (c, x, 0)
        case 1: (x, c, 0)
        case 2: (0, c, x)
        case 3: (0, x, c)
        case 4: (x, 0, c)
        default: (c, 0, x)
        }
        self.init(r + m, g + m, b + m)
    }
}
