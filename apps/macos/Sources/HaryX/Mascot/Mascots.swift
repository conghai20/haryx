import AppKit
import HaryXCore
import SwiftUI

struct MascotStyle: Codable, Hashable, Sendable {
    var species: Species
    /// nil = the species' own colour.
    var hue: Double?

    var palette: Palette { Palette(species: species, hue: hue) }
}

/// Per-account mascot choices, keyed by cswap identity (email|org) so they survive slot moves.
@MainActor @Observable
final class MascotStyles {
    private var overrides: [String: MascotStyle]

    init() {
        let data = Prefs.d.data(forKey: Prefs.mascotStyles) ?? Data()
        overrides = (try? JSONDecoder().decode([String: MascotStyle].self, from: data)) ?? [:]
    }

    /// Unset accounts get a hash-chosen species, probing past ones already in use so a
    /// handful of accounts all look different.
    func resolve(_ accounts: [Account]) -> [String: MascotStyle] {
        var result: [String: MascotStyle] = [:]
        var used = Set(overrides.values.map(\.species))
        for a in accounts {
            if let o = overrides[a.identityKey] { result[a.identityKey] = o }
        }
        for a in accounts.sorted(by: { $0.identityKey < $1.identityKey }) where result[a.identityKey] == nil {
            let all = Species.allCases
            let start = all.firstIndex(of: Species.defaultFor(key: a.identityKey)) ?? 0
            let species = (0..<all.count).map { all[(start + $0) % all.count] }.first { !used.contains($0) }
                ?? all[start]
            used.insert(species)
            result[a.identityKey] = MascotStyle(species: species, hue: nil)
        }
        return result
    }

    func set(_ style: MascotStyle?, for key: String) {
        var next = overrides
        next[key] = style
        overrides = next
        if let data = try? JSONEncoder().encode(next) { Prefs.d.set(data, forKey: Prefs.mascotStyles) }
    }
}

/// Frames are tiny and few (species × colour × pose); cache them all.
@MainActor
enum MascotCache {
    private static let cache: NSCache<NSString, CGImage> = {
        let c = NSCache<NSString, CGImage>()
        c.countLimit = 2_000
        return c
    }()

    static func image(_ style: MascotStyle, pose: Pose, scale: Int = 1) -> CGImage? {
        let key = "\(style.species.rawValue)|\(style.hue ?? -1)|\(pose.hashValue)|\(scale)" as NSString
        if let hit = cache.object(forKey: key) { return hit }
        guard let img = PixelArt.image(style.species, palette: style.palette, pose: pose, scale: scale) else { return nil }
        cache.setObject(img, forKey: key)
        return img
    }

    /// Menu bar glyph: 18 pt drawn at 2× so pixels stay crisp.
    /// The same NSImage instance comes back per frame, so AppKit can skip redrawing an unchanged glyph.
    static func menuBarImage(_ style: MascotStyle, pose: Pose) -> NSImage {
        let key = "\(style.species.rawValue)|\(style.hue ?? -1)|\(pose.hashValue)" as NSString
        if let hit = menuBarImages.object(forKey: key) { return hit }
        guard let cg = image(style, pose: pose, scale: 2) else { return NSImage() }
        let img = NSImage(cgImage: cg, size: NSSize(width: 18, height: 18))
        menuBarImages.setObject(img, forKey: key)
        return img
    }
    private static let menuBarImages = NSCache<NSString, NSImage>()
}

struct MascotsPausedKey: EnvironmentKey { static let defaultValue = false }
extension EnvironmentValues {
    var mascotsPaused: Bool {
        get { self[MascotsPausedKey.self] }
        set { self[MascotsPausedKey.self] = newValue }
    }
}

struct MascotView: View {
    let style: MascotStyle
    let mood: Mood
    var size: CGFloat = 40
    var phase = 0

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.mascotsPaused) private var paused

    var body: some View {
        TimelineView(.animation(minimumInterval: 1 / Pose.fps(for: mood), paused: paused || reduceMotion)) { ctx in
            let pose = reduceMotion ? Pose(mood: mood)
                : Pose.at(ctx.date.timeIntervalSinceReferenceDate, mood: mood, phase: phase)
            if let cg = MascotCache.image(style, pose: pose) {
                Image(decorative: cg, scale: 1)
                    .resizable()
                    .interpolation(.none)
                    .frame(width: size, height: size)
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel("\(style.species.title) mascot, \(mood.rawValue)")
    }
}
