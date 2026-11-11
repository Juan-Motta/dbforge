// Renders the DMG window background into backend/dmg/background.tiff.
// Usage: swift scripts/make-dmg-background.swift
//
// The window is 660 x 400 pt. Finder draws the two icons and their labels itself (centres at x 180 and
// 480, y 170, see scripts/dmg-settings.py); this image holds the backdrop, the arrow between them and,
// as the help line, a SQL statement that "installs" the app, in the app's own syntax colours.
// The TIFF carries a 1x and a 2x image so the background stays sharp on Retina screens.
import AppKit
import Foundation

let size = CGSize(width: 660, height: 400)
let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let output = root.appendingPathComponent("backend/dmg/background.tiff")
let scratch = FileManager.default.temporaryDirectory.appendingPathComponent("dbforge-dmg-background")

func color(_ hex: UInt32) -> NSColor {
    NSColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
}

// The light theme's tokens (frontend/styles/theme.css).
let panel = color(0xF8F8F7)
let backdrop = color(0xEDEDEA)
let surface = color(0xFFFFFF)
let border = color(0xE2E2DF)
let text = color(0x18191B)
let text2 = color(0x4F525A)
let text3 = color(0x676B73)
let accent = color(0xB9521A)
let keyword = color(0x2A5BC9)
let string = color(0x2D7A33)

/// The statement, a run of (text, colour) per piece, a line each.
let statement: [[(String, NSColor)]] = [
    [("-- Drag DBForge onto Applications to install", text3)],
    [("INSERT INTO ", keyword), ("Applications ", text), ("(", text2), ("app", text), (")", text2)],
    [("VALUES ", keyword), ("(", text2), ("'DBForge'", string), (");", text2)],
]

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

    NSGradient(starting: panel, ending: backdrop)!.draw(in: CGRect(origin: .zero, size: size), angle: 90)

    // The arrow from the app to Applications.
    let arrow = CGMutablePath()
    arrow.move(to: CGPoint(x: 282, y: 170))
    arrow.addLine(to: CGPoint(x: 374, y: 170))
    arrow.move(to: CGPoint(x: 360, y: 156))
    arrow.addLine(to: CGPoint(x: 376, y: 170))
    arrow.addLine(to: CGPoint(x: 360, y: 184))
    context.addPath(arrow)
    context.setStrokeColor(accent.cgColor)
    context.setLineWidth(4)
    context.setLineCap(.round)
    context.setLineJoin(.round)
    context.strokePath()

    // The statement, in a card like the app's query editor.
    let font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
    let lines = statement.map { pieces in
        let line = NSMutableAttributedString()
        for (piece, colour) in pieces {
            line.append(NSAttributedString(string: piece, attributes: [.font: font, .foregroundColor: colour]))
        }
        return line
    }
    let lineHeight: CGFloat = 20
    let padding = CGSize(width: 18, height: 14)
    let width = lines.map { $0.size().width }.max()! + padding.width * 2
    let card = CGRect(
        x: (size.width - width) / 2, y: 274,
        width: width, height: lineHeight * CGFloat(lines.count) + padding.height * 2)
    let shape = CGPath(roundedRect: card, cornerWidth: 10, cornerHeight: 10, transform: nil)
    context.addPath(shape)
    context.setFillColor(surface.cgColor)
    context.fillPath()
    context.addPath(shape)
    context.setStrokeColor(border.cgColor)
    context.setLineWidth(1)
    context.strokePath()
    // The accent bar at its left, as the editor marks the line being run.
    context.setFillColor(accent.cgColor)
    context.fill(CGRect(x: card.minX + 8, y: card.minY + padding.height + lineHeight + 3, width: 3, height: lineHeight * 2 - 6))

    NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
    for (index, line) in lines.enumerated() {
        line.draw(at: CGPoint(x: card.minX + padding.width, y: card.minY + padding.height + lineHeight * CGFloat(index) + 2))
    }
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
