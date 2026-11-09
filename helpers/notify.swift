// MacroFlow pop-up notification for macOS.
// Shows a small card at the top of the screen without taking focus away from the
// app the macro is working in. Click it (or wait) to dismiss.
//
//   macroflow-notify <title> <message> <style> <position> <duration-ms> <wait 0|1> <sound 0|1>
//
// style: info | success | warning | error      position: top-center | top-right
// With wait = 1 it stays until clicked. Exits 0 when it closes, 2 on bad arguments.
// Compiled automatically by build.rs.

import Cocoa

let args = CommandLine.arguments
guard args.count >= 8 else {
    FileHandle.standardError.write(
        "usage: macroflow-notify <title> <message> <style> <position> <duration-ms> <wait 0|1> <sound 0|1>\n"
            .data(using: .utf8)!)
    exit(2)
}
let titleText = args[1].isEmpty ? "MacroFlow" : args[1]
let messageText = args[2]
let style = args[3]
let atRight = args[4] == "top-right"
let duration = max(0.5, (Double(args[5]) ?? 3000) / 1000)
let waitForClick = args[6] == "1"
let playSound = args[7] == "1"

let accent: NSColor
let symbol: String
switch style {
case "success":
    accent = NSColor(srgbRed: 0.204, green: 0.780, blue: 0.482, alpha: 1)
    symbol = "✓"
case "warning":
    accent = NSColor(srgbRed: 0.961, green: 0.722, blue: 0.290, alpha: 1)
    symbol = "!"
case "error":
    accent = NSColor(srgbRed: 0.937, green: 0.357, blue: 0.357, alpha: 1)
    symbol = "×"
default:
    accent = NSColor(srgbRed: 0.486, green: 0.361, blue: 1.0, alpha: 1)
    symbol = "i"
}

// Layout.
let cardWidth: CGFloat = 380
let pad: CGFloat = 16
let iconSize: CGFloat = 32
let textX = pad + iconSize + 12
let textW = cardWidth - textX - pad - 18

func textAttributes(_ font: NSFont, _ color: NSColor) -> [NSAttributedString.Key: Any] {
    let p = NSMutableParagraphStyle()
    p.lineBreakMode = .byWordWrapping
    return [.font: font, .foregroundColor: color, .paragraphStyle: p]
}
let titleAttrs = textAttributes(NSFont.systemFont(ofSize: 13, weight: .semibold), NSColor(white: 0.93, alpha: 1))
let bodyAttrs = textAttributes(NSFont.systemFont(ofSize: 12), NSColor(white: 0.70, alpha: 1))
let hintAttrs = textAttributes(NSFont.systemFont(ofSize: 11, weight: .medium), accent)
let hintText = "Click to continue"

func textHeight(_ s: String, _ a: [NSAttributedString.Key: Any]) -> CGFloat {
    if s.isEmpty { return 0 }
    let r = (s as NSString).boundingRect(
        with: NSSize(width: textW, height: 10000),
        options: [.usesLineFragmentOrigin, .usesFontLeading],
        attributes: a)
    return ceil(r.height)
}
let titleH = textHeight(titleText, titleAttrs)
let bodyH = textHeight(messageText, bodyAttrs)
let hintH = waitForClick ? textHeight(hintText, hintAttrs) : 0
let cardHeight = max(
    pad + titleH + (bodyH > 0 ? 4 + bodyH : 0) + (hintH > 0 ? 6 + hintH : 0) + pad,
    iconSize + pad * 2)

/// A panel that never takes keyboard focus, so the macro keeps typing into its app.
final class ToastPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

var dismissing = false
func dismiss(_ panel: NSWindow) {
    if dismissing { return }
    dismissing = true
    NSAnimationContext.runAnimationGroup(
        { ctx in
            ctx.duration = 0.18
            panel.animator().alphaValue = 0
        },
        completionHandler: { exit(0) })
}

final class ToastView: NSView {
    var hoverClose = false

    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    var closeRect: NSRect { NSRect(x: bounds.width - pad - 12, y: pad, width: 12, height: 12) }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(
            NSTrackingArea(
                rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                owner: self))
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }

    override func draw(_ dirtyRect: NSRect) {
        let card = NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5), xRadius: 12, yRadius: 12)
        NSColor(srgbRed: 0.094, green: 0.102, blue: 0.133, alpha: 0.97).setFill()
        card.fill()

        // Coloured stripe down the left edge.
        NSGraphicsContext.saveGraphicsState()
        card.addClip()
        accent.setFill()
        NSRect(x: 0, y: 0, width: 4, height: bounds.height).fill()
        NSGraphicsContext.restoreGraphicsState()

        NSColor(white: 1, alpha: 0.10).setStroke()
        card.lineWidth = 1
        card.stroke()

        // Icon.
        let iconRect = NSRect(x: pad, y: pad, width: iconSize, height: iconSize)
        accent.withAlphaComponent(0.18).setFill()
        NSBezierPath(ovalIn: iconRect).fill()
        let symAttrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 15, weight: .bold), .foregroundColor: accent,
        ]
        let symSize = (symbol as NSString).size(withAttributes: symAttrs)
        (symbol as NSString).draw(
            at: NSPoint(x: iconRect.midX - symSize.width / 2, y: iconRect.midY - symSize.height / 2),
            withAttributes: symAttrs)

        // Text.
        let opts: NSString.DrawingOptions = [.usesLineFragmentOrigin, .usesFontLeading]
        var y = pad
        (titleText as NSString).draw(
            with: NSRect(x: textX, y: y, width: textW, height: titleH), options: opts, attributes: titleAttrs)
        y += titleH
        if bodyH > 0 {
            y += 4
            (messageText as NSString).draw(
                with: NSRect(x: textX, y: y, width: textW, height: bodyH), options: opts, attributes: bodyAttrs)
            y += bodyH
        }
        if hintH > 0 {
            y += 6
            (hintText as NSString).draw(
                with: NSRect(x: textX, y: y, width: textW, height: hintH), options: opts, attributes: hintAttrs)
        }

        // Close cross.
        let c = closeRect.insetBy(dx: 2, dy: 2)
        let cross = NSBezierPath()
        cross.move(to: NSPoint(x: c.minX, y: c.minY))
        cross.line(to: NSPoint(x: c.maxX, y: c.maxY))
        cross.move(to: NSPoint(x: c.maxX, y: c.minY))
        cross.line(to: NSPoint(x: c.minX, y: c.maxY))
        cross.lineWidth = 1.5
        NSColor(white: 1, alpha: hoverClose ? 0.9 : 0.45).setStroke()
        cross.stroke()
    }

    override func mouseMoved(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        let over = closeRect.insetBy(dx: -6, dy: -6).contains(p)
        if over != hoverClose {
            hoverClose = over
            needsDisplay = true
        }
    }

    override func mouseExited(with event: NSEvent) {
        if hoverClose {
            hoverClose = false
            needsDisplay = true
        }
    }

    override func mouseDown(with event: NSEvent) {
        if let w = window { dismiss(w) }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)  // no Dock icon, never steals focus

// The screen the mouse is on, top centre (or top right), below the menu bar.
let mouse = NSEvent.mouseLocation
let screen =
    NSScreen.screens.first(where: { NSMouseInRect(mouse, $0.frame, false) }) ?? NSScreen.main
    ?? NSScreen.screens[0]
let visible = screen.visibleFrame
let originX = atRight ? visible.maxX - cardWidth - 16 : visible.midX - cardWidth / 2
let finalFrame = NSRect(x: originX, y: visible.maxY - cardHeight - 12, width: cardWidth, height: cardHeight)

let panel = ToastPanel(
    contentRect: finalFrame.offsetBy(dx: 0, dy: 10),
    styleMask: [.borderless, .nonactivatingPanel],
    backing: .buffered,
    defer: false)
panel.level = .statusBar
panel.isOpaque = false
panel.backgroundColor = .clear
panel.hasShadow = true
panel.hidesOnDeactivate = false
panel.isFloatingPanel = true
panel.becomesKeyOnlyIfNeeded = true
panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
panel.alphaValue = 0
panel.contentView = ToastView(frame: NSRect(origin: .zero, size: finalFrame.size))
panel.orderFrontRegardless()

// Slide down and fade in.
NSAnimationContext.runAnimationGroup { ctx in
    ctx.duration = 0.2
    panel.animator().alphaValue = 1
    panel.animator().setFrame(finalFrame, display: true)
}

if playSound {
    let name = style == "error" ? "Basso" : (style == "warning" ? "Funk" : "Glass")
    NSSound(named: NSSound.Name(name))?.play()
}

if !waitForClick {
    Timer.scheduledTimer(withTimeInterval: duration, repeats: false) { _ in dismiss(panel) }
}

app.run()
