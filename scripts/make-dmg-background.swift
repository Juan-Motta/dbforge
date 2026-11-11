// Renders the DMG window background into backend/dmg/background.tiff.
// Usage: swift scripts/make-dmg-background.swift
//
// The window is 660 x 440 pt. Finder draws the two icons and their labels itself (centres at x 180 and
// 480, y 150, see scripts/dmg-settings.py); this image holds the backdrop, the arrow between them and,
// as the help line, a SQL statement that "installs" the app, in a small editor like the app's console.
// The TIFF carries a 1x and a 2x image so the background stays sharp on Retina screens.
import AppKit
import Foundation

let size = CGSize(width: 660, height: 440)
let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let output = root.appendingPathComponent("backend/dmg/background.tiff")
let scratch = FileManager.default.temporaryDirectory.appendingPathComponent("dbforge-dmg-background")

func color(_ hex: UInt32, alpha: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
}

// The window stays light: Finder draws the icon labels dark in light mode. The editor card takes the
// dark theme's tokens (frontend/styles/theme.css), as DBForge opens by default.
let top = color(0xFAFAF8)
let bottom = color(0xECECE8)
let accent = color(0xF07A3A)
let editor = color(0x111215)
let editorBar = color(0x16171A)
let editorBorder = color(0x30323A)
let text = color(0xE6E7EA)
let text2 = color(0xA6A9B0)
let text3 = color(0x868A93)
let keyword = color(0x8FB4FF)
let string = color(0xA5D6A0)

/// The icons' centres (scripts/dmg-settings.py).
let appCentre = CGPoint(x: 180, y: 150)
let applicationsCentre = CGPoint(x: 480, y: 150)

/// The statement, a run of (text, colour) per piece, a line each.
let statement: [[(String, NSColor)]] = [
    [("-- Drag DBForge onto Applications to install", text3)],
    [("INSERT INTO ", keyword), ("Applications ", text), ("(", text2), ("app", text), (")", text2)],
    [("VALUES ", keyword), ("(", text2), ("'DBForge'", string), (");", text2)],
]

func draw(_ string: String, _ font: NSFont, _ colour: NSColor, at point: CGPoint) {
    NSAttributedString(string: string, attributes: [.font: font, .foregroundColor: colour]).draw(at: point)
}

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

    NSGradient(starting: top, ending: bottom)!.draw(in: CGRect(origin: .zero, size: size), angle: -90)
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

    // The statement, in a small editor window like the app's query console.
    let mono = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
    let lines = statement.map { pieces in
        let line = NSMutableAttributedString()
        for (piece, colour) in pieces {
            line.append(NSAttributedString(string: piece, attributes: [.font: mono, .foregroundColor: colour]))
        }
        return line
    }
    let lineHeight: CGFloat = 22
    let bar: CGFloat = 28
    let gutter: CGFloat = 40
    let width = lines.map { $0.size().width }.max()! + gutter + 26
    let card = CGRect(
        x: (size.width - width) / 2, y: 266,
        width: width, height: bar + lineHeight * CGFloat(lines.count) + 20)
    let shape = CGPath(roundedRect: card, cornerWidth: 10, cornerHeight: 10, transform: nil)
    context.saveGState()
    context.setShadow(offset: CGSize(width: 0, height: 8), blur: 24, color: color(0x000000, alpha: 0.22).cgColor)
    context.addPath(shape)
    context.setFillColor(editor.cgColor)
    context.fillPath()
    context.restoreGState()
    // The title bar: the three dots and the file's tab.
    context.saveGState()
    context.addPath(shape)
    context.clip()
    context.setFillColor(editorBar.cgColor)
    context.fill(CGRect(x: card.minX, y: card.minY, width: card.width, height: bar))
    context.setFillColor(editorBorder.cgColor)
    context.fill(CGRect(x: card.minX, y: card.minY + bar - 1, width: card.width, height: 1))
    context.restoreGState()
    for (index, dot) in [0xFF5F57, 0xFEBC2E, 0x28C840].enumerated() {
        context.setFillColor(color(UInt32(dot)).cgColor)
        context.fillEllipse(in: CGRect(x: card.minX + 14 + CGFloat(index) * 16, y: card.minY + 9, width: 10, height: 10))
    }
    context.addPath(shape)
    context.setStrokeColor(editorBorder.cgColor)
    context.setLineWidth(1)
    context.strokePath()
    // The line being run, marked like the console marks it.
    let firstLine = card.minY + bar + 10
    context.setFillColor(color(0xF07A3A, alpha: 0.10).cgColor)
    context.fill(CGRect(x: card.minX + 1, y: firstLine + lineHeight, width: card.width - 2, height: lineHeight * 2))
    context.setFillColor(accent.cgColor)
    context.fill(CGRect(x: card.minX + 1, y: firstLine + lineHeight, width: 2, height: lineHeight * 2))

    NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
    draw("install.sql", NSFont.systemFont(ofSize: 11.5, weight: .medium), text2, at: CGPoint(x: card.minX + 66, y: card.minY + 6))
    let numbers = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    for (index, line) in lines.enumerated() {
        let y = firstLine + lineHeight * CGFloat(index) + 3
        let number = "\(index + 1)"
        let numberWidth = NSAttributedString(string: number, attributes: [.font: numbers]).size().width
        draw(number, numbers, text3, at: CGPoint(x: card.minX + gutter - 14 - numberWidth, y: y + 1))
        line.draw(at: CGPoint(x: card.minX + gutter, y: y))
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
