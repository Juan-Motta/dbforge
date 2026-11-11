// Renders the DMG window background into backend/dmg/background.tiff.
// Usage: swift scripts/make-dmg-background.swift
//
// The window is 660 x 380 pt. Finder draws the two icons and their labels itself (centres at x 180 and
// 480, y 150, see scripts/dmg-settings.py); this image holds the backdrop, the arrow between them and,
// as the help line, the SQL statement that "installs" the app.
// The TIFF carries a 1x and a 2x image so the background stays sharp on Retina screens.
import AppKit
import Foundation

let size = CGSize(width: 660, height: 380)
let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let output = root.appendingPathComponent("backend/dmg/background.tiff")
let scratch = FileManager.default.temporaryDirectory.appendingPathComponent("dbforge-dmg-background")

func color(_ hex: UInt32, alpha: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
}

// The window stays light: Finder draws the icon labels dark in light mode. Colours are the light theme's
// tokens (frontend/styles/theme.css).
let top = color(0xFAFAF8)
let bottom = color(0xECECE8)
let accent = color(0xB9521A)
let text2 = color(0x4F525A)

/// The icons' centres (scripts/dmg-settings.py).
let appCentre = CGPoint(x: 180, y: 150)
let applicationsCentre = CGPoint(x: 480, y: 150)

/// What installing the app would be, said in SQL.
let statement = "INSERT INTO Applications VALUES ('DBForge');"

func render(scale: CGFloat, to url: URL) throws {
    guard let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(size.width * scale), pixelsHigh: Int(size.height * scale),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .calibratedRGB, bytesPerRow: 0, bitsPerPixel: 0
    ) else { throw CocoaError(.fileWriteUnknown) }
    bitmap.size = size

    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    let context = NSGraphicsContext.current!.cgContext
    // Top-left origin, like the window.
    context.translateBy(x: 0, y: size.height)
    context.scaleBy(x: 1, y: -1)

    NSGradient(starting: top, ending: bottom)!.draw(in: CGRect(origin: .zero, size: size), angle: 90)
    // An arc from the app to Applications, ending in a filled head.
    let start = CGPoint(x: appCentre.x + 92, y: appCentre.y)
    let end = CGPoint(x: applicationsCentre.x - 96, y: applicationsCentre.y)
    let control = CGPoint(x: (start.x + end.x) / 2, y: appCentre.y - 34)
    let arc = CGMutablePath()
    arc.move(to: start)
    arc.addQuadCurve(to: end, control: control)
    context.addPath(arc)
    context.setStrokeColor(accent.cgColor)
    context.setLineWidth(3)
    context.setLineCap(.round)
    context.setLineDash(phase: 0, lengths: [1, 8])
    context.strokePath()
    context.setLineDash(phase: 0, lengths: [])
    // The head points along the curve where it ends, which runs from the control point to the end.
    let angle = atan2(end.y - control.y, end.x - control.x)
    let head = CGMutablePath()
    head.move(to: CGPoint(x: 10, y: 0))
    head.addLine(to: CGPoint(x: -6, y: -8))
    head.addLine(to: CGPoint(x: -2, y: 0))
    head.addLine(to: CGPoint(x: -6, y: 8))
    head.closeSubpath()
    context.addPath(head.copy(using: [CGAffineTransform(translationX: end.x, y: end.y).rotated(by: angle)])!)
    context.setFillColor(accent.cgColor)
    context.fillPath()

    // The statement, plain and centred under the icons' labels.
    let line = NSAttributedString(string: statement, attributes: [
        .font: NSFont.monospacedSystemFont(ofSize: 14, weight: .regular),
        .foregroundColor: text2,
    ])
    NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
    line.draw(at: CGPoint(x: (size.width - line.size().width) / 2, y: 288))
    NSGraphicsContext.restoreGraphicsState()

    guard let png = bitmap.representation(using: .png, properties: [:]) else { throw CocoaError(.fileWriteUnknown) }
    try png.write(to: url)
}

try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
try FileManager.default.createDirectory(at: output.deletingLastPathComponent(), withIntermediateDirectories: true)
let normal = scratch.appendingPathComponent("background.png")
let retina = scratch.appendingPathComponent("background@2x.png")
try render(scale: 1, to: normal)
try render(scale: 2, to: retina)

let tiffutil = Process()
tiffutil.executableURL = URL(fileURLWithPath: "/usr/bin/tiffutil")
tiffutil.arguments = ["-cathidpicheck", normal.path, retina.path, "-out", output.path]
try tiffutil.run()
tiffutil.waitUntilExit()
try? FileManager.default.removeItem(at: scratch)
guard tiffutil.terminationStatus == 0 else { exit(tiffutil.terminationStatus) }
print("Wrote \(output.path)")
