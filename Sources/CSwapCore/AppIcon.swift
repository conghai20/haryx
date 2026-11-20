import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// The cswapui icon: the pixel critter on a warm squircle, with pixel swap arrows.
/// Rendered from the same sprite data the app animates, so icon and mascot never drift.
public enum AppIcon {
    public static let side = 1024
    static let inset = 100
    static let pixel = 30

    static let arrows: [String] = [
        "...K..........",
        "..KK..........",
        ".KKKKKKKKKKK..",
        "..KK..........",
        "...K......K...",
        "..........KK..",
        "..KKKKKKKKKKK.",
        "..........KK..",
        "..........K...",
    ]

    public static func render() -> CGImage? {
        let palette = Palette(species: .critter, hue: nil)
        let grid = PixelArt.pixels(.critter, palette: palette, pose: Pose())
        // Pad the 18-px sprite into a 24-px board with the arrows above it.
        let board = 24
        var big = [[RGB?]](repeating: [RGB?](repeating: nil, count: board), count: board)
        let arrowColor = RGB(0.30, 0.20, 0.16)
        for (y, row) in arrows.enumerated() {
            for (x, ch) in row.enumerated() where ch == "K" { big[y + 1][x + 5] = arrowColor }
        }
        for y in 0..<PixelArt.canvas {
            for x in 0..<PixelArt.canvas { if let p = grid[y][x] { big[y + 5][x + 3] = p } }
        }
        let origin = ((side - board * pixel) / 2, (side - board * pixel) / 2 + 10)
        return PixelArt.draw(big, scale: pixel, background: drawBackground, origin: origin, size: side)
    }

    static func drawBackground(_ ctx: CGContext, _ side: Int) {
        let rect = CGRect(x: inset, y: inset, width: side - 2 * inset, height: side - 2 * inset)
        let path = CGPath(roundedRect: rect, cornerWidth: 185, cornerHeight: 185, transform: nil)
        ctx.saveGState()
        ctx.setShadow(offset: CGSize(width: 0, height: -10), blur: 28, color: CGColor(gray: 0, alpha: 0.35))
        ctx.addPath(path)
        ctx.setFillColor(CGColor(srgbRed: 0.96, green: 0.93, blue: 0.87, alpha: 1))
        ctx.fillPath()
        ctx.restoreGState()

        ctx.saveGState()
        ctx.addPath(path)
        ctx.clip()
        let colors = [CGColor(srgbRed: 0.99, green: 0.96, blue: 0.91, alpha: 1),
                      CGColor(srgbRed: 0.95, green: 0.84, blue: 0.74, alpha: 1)] as CFArray
        if let gradient = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: colors, locations: [0, 1]) {
            ctx.drawLinearGradient(gradient, start: CGPoint(x: 0, y: rect.maxY), end: CGPoint(x: 0, y: rect.minY), options: [])
        }
        // Glassy top sheen.
        ctx.setFillColor(CGColor(gray: 1, alpha: 0.28))
        ctx.fillEllipse(in: CGRect(x: rect.minX - 120, y: rect.midY + 120, width: rect.width + 240, height: rect.height))
        ctx.restoreGState()
    }

    public static func writePNG(_ image: CGImage, to url: URL) throws {
        guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
            throw CocoaError(.fileWriteUnknown)
        }
        CGImageDestinationAddImage(dest, image, nil)
        guard CGImageDestinationFinalize(dest) else { throw CocoaError(.fileWriteUnknown) }
    }
}
