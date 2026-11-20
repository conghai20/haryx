import AppKit

// Rasterize an SVG with AppKit's SVG support.
// usage: render <in.svg> <out.png> <size | WIDTHxHEIGHT> [background hex, e.g. 111827]
let args = CommandLine.arguments
guard args.count >= 4 else {
    FileHandle.standardError.write("usage: render in.svg out.png size|WxH [bghex]\n".data(using: .utf8)!)
    exit(2)
}
guard let image = NSImage(contentsOf: URL(fileURLWithPath: args[1])) else {
    FileHandle.standardError.write("cannot load \(args[1])\n".data(using: .utf8)!)
    exit(1)
}
let dims = args[3].split(separator: "x").compactMap { Int($0) }
let width = dims[0], height = dims.count > 1 ? dims[1] : dims[0]
guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8,
                                 samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                                 bytesPerRow: 0, bitsPerPixel: 0) else { exit(1) }
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let rect = NSRect(x: 0, y: 0, width: width, height: height)
if args.count > 4, let hex = Int(args[4], radix: 16) {
    NSColor(srgbRed: CGFloat(hex >> 16 & 255) / 255, green: CGFloat(hex >> 8 & 255) / 255,
            blue: CGFloat(hex & 255) / 255, alpha: 1).setFill()
    rect.fill()
}
image.draw(in: rect)
NSGraphicsContext.restoreGraphicsState()
guard let png = rep.representation(using: .png, properties: [:]) else { exit(1) }
try png.write(to: URL(fileURLWithPath: args[2]))
