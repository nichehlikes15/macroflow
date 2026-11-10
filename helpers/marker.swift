// MacroFlow click marker for macOS.
// A small green dot that shows where a running macro is about to click. Clicks pass
// straight through it, it never takes focus, and it's excluded from screen capture
// (sharingType = .none), so MacroFlow's own screenshots don't see it.
//
// Reads one command per line on stdin (coordinates are global, top-left origin,
// in points, the same as the mouse):
//   show <x> <y> click|move     pulse     hide
// Prints "ready" once it can take commands. Quits when stdin closes.
// Compiled automatically by build.rs.

import Cocoa

setvbuf(stdout, nil, _IOLBF, 0)

let green = NSColor(srgbRed: 0.204, green: 0.780, blue: 0.482, alpha: 1)
let boxSize: CGFloat = 60

final class DotView: NSView {
    /// 0...1 while the click ripple plays, -1 otherwise.
    var ripple: CGFloat = -1
    /// A ring (mouse move) rather than a filled dot (click).
    var hollow = false

    override func draw(_ dirtyRect: NSRect) {
        NSColor.clear.set()
        dirtyRect.fill()
        let c = NSPoint(x: bounds.midX, y: bounds.midY)
        func circle(_ r: CGFloat) -> NSBezierPath {
            NSBezierPath(ovalIn: NSRect(x: c.x - r, y: c.y - r, width: r * 2, height: r * 2))
        }
        if ripple >= 0 {
            let ring = circle(8 + ripple * 18)
            ring.lineWidth = 3 * (1 - ripple) + 0.5
            green.withAlphaComponent(1 - ripple).setStroke()
            ring.stroke()
        }
        let fade: CGFloat = ripple >= 0 ? max(0, 1 - ripple * 1.6) : 1
        if fade <= 0 { return }
        let r: CGFloat = 7
        // A faint dark edge so it shows up on light backgrounds too.
        let edge = circle(r + 1.5)
        edge.lineWidth = 1.5
        NSColor(white: 0, alpha: 0.45 * fade).setStroke()
        edge.stroke()
        let dot = circle(r)
        if hollow {
            dot.lineWidth = 2.5
            green.withAlphaComponent(fade).setStroke()
            dot.stroke()
        } else {
            green.withAlphaComponent(0.95 * fade).setFill()
            dot.fill()
            dot.lineWidth = 1.5
            NSColor(white: 1, alpha: 0.9 * fade).setStroke()
            dot.stroke()
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

let panel = NSPanel(
    contentRect: NSRect(x: 0, y: 0, width: boxSize, height: boxSize),
    styleMask: [.borderless, .nonactivatingPanel],
    backing: .buffered,
    defer: false)
panel.isOpaque = false
panel.backgroundColor = .clear
panel.hasShadow = false
panel.ignoresMouseEvents = true
panel.level = .screenSaver
panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
panel.sharingType = .none
panel.hidesOnDeactivate = false
panel.isReleasedWhenClosed = false
let view = DotView(frame: NSRect(x: 0, y: 0, width: boxSize, height: boxSize))
panel.contentView = view

var hideTimer: Timer?
var animTimer: Timer?

func hideDot() {
    animTimer?.invalidate()
    hideTimer?.invalidate()
    panel.orderOut(nil)
}

func showDot(_ x: CGFloat, _ y: CGFloat, hollow: Bool) {
    animTimer?.invalidate()
    view.ripple = -1
    view.hollow = hollow
    // Mouse coordinates have their origin at the top left of the main display;
    // Cocoa's are at the bottom left.
    let mainHeight = NSScreen.screens.first?.frame.height ?? 0
    panel.setFrameOrigin(NSPoint(x: x - boxSize / 2, y: mainHeight - y - boxSize / 2))
    view.needsDisplay = true
    panel.orderFrontRegardless()
    hideTimer?.invalidate()
    hideTimer = Timer.scheduledTimer(withTimeInterval: 3.5, repeats: false) { _ in hideDot() }
}

func pulseDot() {
    guard panel.isVisible else { return }
    hideTimer?.invalidate()
    animTimer?.invalidate()
    var t: CGFloat = 0
    animTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60.0, repeats: true) { timer in
        t += 1.0 / 18.0  // about 300 ms
        view.ripple = min(1, t)
        view.needsDisplay = true
        if t >= 1 {
            timer.invalidate()
            hideDot()
        }
    }
}

func handle(_ line: String) {
    let parts = line.split(separator: " ").map(String.init)
    guard let cmd = parts.first else { return }
    switch cmd {
    case "show" where parts.count >= 3:
        if let x = Double(parts[1]), let y = Double(parts[2]) {
            showDot(CGFloat(x), CGFloat(y), hollow: parts.count > 3 && parts[3] == "move")
        }
    case "pulse":
        pulseDot()
    case "hide":
        hideDot()
    default:
        break
    }
}

let reader = Thread {
    while let line = readLine() {
        DispatchQueue.main.async { handle(line) }
    }
    DispatchQueue.main.async { exit(0) }
}
reader.start()

print("ready")
fflush(stdout)
app.run()
