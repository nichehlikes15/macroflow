// MacroFlow area selector for macOS, modelled on the Windows Snipping Tool:
// the screen freezes and dims, a small toolbar sits at the top centre, you drag a
// rectangle (shown at full brightness with a thin border and its size), and letting
// go confirms straight away. Esc or the toolbar's close button cancels.
//
//   macroflow-select <screenshot.png> <x> <y> <w> <h> [hint]
//
// x/y/w/h are the display's position and size in global screen points (origin at the
// top-left of the main display, y down: the same coordinates the mouse uses).
// Prints "OK x0 y0 x1 y1": the selection as fractions (0-1) of the display.
// Exit codes: 0 = picked, 3 = cancelled, 2 = bad arguments. Compiled automatically by build.rs.

import Cocoa

struct Config {
    let image: NSImage
    let x: Double, y: Double, w: Double, h: Double
    let hint: String
}

func loadConfig() -> Config {
    let a = CommandLine.arguments
    guard a.count >= 6,
        let image = NSImage(contentsOfFile: a[1]),
        let x = Double(a[2]), let y = Double(a[3]),
        let w = Double(a[4]), let h = Double(a[5]),
        w > 0, h > 0
    else {
        FileHandle.standardError.write("usage: macroflow-select <image> <x> <y> <w> <h> [hint]\n".data(using: .utf8)!)
        exit(2)
    }
    return Config(image: image, x: x, y: y, w: w, h: h, hint: a.count >= 7 ? a[6] : "Drag to select an area")
}

let config = loadConfig()

func finish(_ result: NSRect?) -> Never {
    if let r = result {
        let fw = max(config.w, 1), fh = max(config.h, 1)
        // String(format:) isn't localised, so the decimal point is always ".".
        let line = String(
            format: "OK %.6f %.6f %.6f %.6f\n",
            Double(r.minX) / fw, Double(r.minY) / fh, Double(r.maxX) / fw, Double(r.maxY) / fh
        )
        FileHandle.standardOutput.write(line.data(using: .utf8)!)
        exit(0)
    }
    exit(3)
}

/// Borderless windows can't take keyboard focus by default; we need it for Esc.
final class OverlayWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

final class SnipView: NSView {
    var start: NSPoint?
    var current: NSPoint?
    var hoverClose = false

    override var isFlipped: Bool { true }  // top-left origin, like the screen
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    var selection: NSRect? {
        guard let s = start, let c = current else { return nil }
        let r = NSRect(x: min(s.x, c.x), y: min(s.y, c.y), width: abs(s.x - c.x), height: abs(s.y - c.y))
        return r.width >= 1 && r.height >= 1 ? r : nil
    }

    // Toolbar geometry (top centre).
    var toolbar: NSRect {
        let w: CGFloat = 132, h: CGFloat = 44
        return NSRect(x: (bounds.width - w) / 2, y: 18, width: w, height: h)
    }
    var rectButton: NSRect { NSRect(x: toolbar.minX + 6, y: toolbar.minY + 6, width: 32, height: 32) }
    var closeButton: NSRect { NSRect(x: toolbar.maxX - 38, y: toolbar.minY + 6, width: 32, height: 32) }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .crosshair)
        addCursorRect(toolbar, cursor: .arrow)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseMoved, .activeAlways, .inVisibleRect], owner: self))
    }

    override func draw(_ dirtyRect: NSRect) {
        // The frozen screen.
        config.image.draw(in: bounds, from: .zero, operation: .copy, fraction: 1, respectFlipped: true, hints: nil)

        // Dim everything except the selection.
        let dim = NSBezierPath(rect: bounds)
        if let sel = selection {
            dim.append(NSBezierPath(rect: sel))
            dim.windingRule = .evenOdd
        }
        NSColor(white: 0, alpha: 0.5).setFill()
        dim.fill()

        if let sel = selection {
            NSColor.white.setStroke()
            let border = NSBezierPath(rect: sel.insetBy(dx: 0.5, dy: 0.5))
            border.lineWidth = 1
            border.stroke()
            drawSizeLabel(for: sel)
        }

        drawToolbar()
        drawHint()
    }

    func drawSizeLabel(for sel: NSRect) {
        let text = "\(Int(sel.width.rounded())) × \(Int(sel.height.rounded()))" as NSString
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .medium),
            .foregroundColor: NSColor.white,
        ]
        let size = text.size(withAttributes: attrs)
        var origin = NSPoint(x: sel.minX, y: sel.maxY + 6)
        if origin.y + size.height + 8 > bounds.maxY { origin.y = sel.minY - size.height - 14 }
        if origin.y < 0 { origin.y = sel.minY + 6; origin.x = sel.minX + 6 }
        let box = NSRect(x: origin.x, y: origin.y, width: size.width + 12, height: size.height + 6)
        NSColor(white: 0.12, alpha: 0.9).setFill()
        NSBezierPath(roundedRect: box, xRadius: 4, yRadius: 4).fill()
        text.draw(at: NSPoint(x: box.minX + 6, y: box.minY + 3), withAttributes: attrs)
    }

    func drawToolbar() {
        let bar = NSBezierPath(roundedRect: toolbar, xRadius: 8, yRadius: 8)
        NSColor(calibratedRed: 0.13, green: 0.13, blue: 0.14, alpha: 0.97).setFill()
        bar.fill()
        NSColor(white: 1, alpha: 0.12).setStroke()
        bar.lineWidth = 1
        bar.stroke()

        // Rectangle mode (the only mode, shown as selected).
        NSColor(white: 1, alpha: 0.14).setFill()
        NSBezierPath(roundedRect: rectButton, xRadius: 5, yRadius: 5).fill()
        NSColor.white.setStroke()
        let icon = NSBezierPath(rect: rectButton.insetBy(dx: 9, dy: 11))
        icon.lineWidth = 1.5
        icon.stroke()

        // Divider.
        NSColor(white: 1, alpha: 0.15).setFill()
        NSRect(x: closeButton.minX - 8, y: toolbar.minY + 12, width: 1, height: 20).fill()

        // Close.
        if hoverClose {
            NSColor(white: 1, alpha: 0.14).setFill()
            NSBezierPath(roundedRect: closeButton, xRadius: 5, yRadius: 5).fill()
        }
        let c = closeButton.insetBy(dx: 11, dy: 11)
        let x = NSBezierPath()
        x.move(to: NSPoint(x: c.minX, y: c.minY)); x.line(to: NSPoint(x: c.maxX, y: c.maxY))
        x.move(to: NSPoint(x: c.maxX, y: c.minY)); x.line(to: NSPoint(x: c.minX, y: c.maxY))
        x.lineWidth = 1.5
        NSColor.white.setStroke()
        x.stroke()
    }

    func drawHint() {
        guard selection == nil else { return }
        let text = config.hint as NSString
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 13, weight: .regular),
            .foregroundColor: NSColor(white: 1, alpha: 0.9),
        ]
        let size = text.size(withAttributes: attrs)
        let box = NSRect(x: (bounds.width - size.width) / 2 - 10, y: toolbar.maxY + 10, width: size.width + 20, height: size.height + 10)
        NSColor(white: 0.1, alpha: 0.75).setFill()
        NSBezierPath(roundedRect: box, xRadius: 6, yRadius: 6).fill()
        text.draw(at: NSPoint(x: box.minX + 10, y: box.minY + 5), withAttributes: attrs)
    }

    override func mouseMoved(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        let over = closeButton.contains(p)
        if over != hoverClose { hoverClose = over; needsDisplay = true }
    }

    override func mouseDown(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        if closeButton.contains(p) { finish(nil) }
        if toolbar.contains(p) { return }
        start = p
        current = p
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        guard start != nil else { return }
        var p = convert(event.locationInWindow, from: nil)
        p.x = min(max(p.x, 0), bounds.width)
        p.y = min(max(p.y, 0), bounds.height)
        current = p
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        guard start != nil else { return }
        if let sel = selection, sel.width >= 4, sel.height >= 4 {
            finish(sel)
        }
        // Just a click: start again.
        start = nil
        current = nil
        needsDisplay = true
    }

    override func rightMouseDown(with event: NSEvent) {
        // A right-click throws away the box you're drawing, or cancels if there isn't one.
        if start == nil { finish(nil) }
        start = nil
        current = nil
        needsDisplay = true
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 { finish(nil) }  // Esc
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

// Convert from top-left global points to Cocoa's bottom-left coordinates.
let primaryHeight = NSScreen.screens.first?.frame.height ?? CGFloat(config.h)
let frame = NSRect(
    x: CGFloat(config.x),
    y: primaryHeight - CGFloat(config.y) - CGFloat(config.h),
    width: CGFloat(config.w),
    height: CGFloat(config.h)
)

let window = OverlayWindow(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
window.level = .screenSaver  // above the menu bar and Dock
window.isOpaque = true
window.backgroundColor = .black
window.hasShadow = false
window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
window.acceptsMouseMovedEvents = true
let view = SnipView(frame: NSRect(origin: .zero, size: frame.size))
window.contentView = view
window.setFrame(frame, display: true)
window.makeKeyAndOrderFront(nil)
window.makeFirstResponder(view)
app.activate(ignoringOtherApps: true)

// Never leave an invisible overlay behind.
Timer.scheduledTimer(withTimeInterval: 180, repeats: false) { _ in finish(nil) }

app.run()
