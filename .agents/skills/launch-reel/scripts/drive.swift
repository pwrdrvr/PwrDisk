// Guest-only input and window helper for recording PwrDisk. Never run it on
// the operator's desktop: its events land wherever the operator is working.
// drive wid PID | frame PID | resize PID W H X Y | front PID | move X Y
//   | key CODE [cmd|shift] | type TEXT | screen
//   | click X Y [COUNT] | glide X1 Y1 X2 Y2 MS | scroll X Y DY STEPS
import AppKit
import CoreGraphics
import ApplicationServices
let a = CommandLine.arguments
let need = ["wid": 3, "frame": 3, "front": 3, "resize": 7, "move": 4, "key": 3,
            "type": 3, "screen": 2, "click": 4, "glide": 7, "scroll": 6]
guard a.count >= 2, let n = need[a[1]], a.count >= n else {
    FileHandle.standardError.write("usage: see the header of drive.swift\n".data(using: .utf8)!)
    exit(2)
}
func windows(_ pid: Int) -> [[String: Any]] {
    let all = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
    return all.filter { $0[kCGWindowOwnerPID as String] as? Int == pid && $0[kCGWindowLayer as String] as? Int == 0 }
}
func moved(_ p: CGPoint) {
    CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
}
switch a[1] {
case "wid":
    for w in windows(Int(a[2])!) { print(w[kCGWindowNumber as String]!, (w[kCGWindowName as String] as? String) ?? "") }
case "frame":
    if let w = windows(Int(a[2])!).first, let b = w[kCGWindowBounds as String] as? [String: Any] { print(b["X"]!, b["Y"]!, b["Width"]!, b["Height"]!) }
case "screen":
    let f = NSScreen.main!.frame; print(Int(f.width), Int(f.height), NSScreen.main!.backingScaleFactor)
case "front":
    NSRunningApplication(processIdentifier: pid_t(a[2])!)?.activate(options: [.activateIgnoringOtherApps])
case "resize":
    let app = AXUIElementCreateApplication(pid_t(a[2])!)
    var v: CFTypeRef?
    AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &v)
    for w in (v as? [AXUIElement]) ?? [] {
        var p = CGPoint(x: Double(a[5])!, y: Double(a[6])!); var s = CGSize(width: Double(a[3])!, height: Double(a[4])!)
        AXUIElementSetAttributeValue(w, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &p)!)
        AXUIElementSetAttributeValue(w, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &s)!)
        break
    }
case "move":
    moved(CGPoint(x: Double(a[2])!, y: Double(a[3])!))
case "key":
    let code = CGKeyCode(a[2])!
    var flags: CGEventFlags = []
    if a.count > 3 { if a[3] == "cmd" { flags = .maskCommand } else if a[3] == "shift" { flags = .maskShift } }
    for down in [true, false] {
        let e = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)!
        e.flags = flags; e.post(tap: .cghidEventTap); usleep(30000)
    }
case "type":
    for ch in a[2].utf16 {
        for down in [true, false] {
            let e = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
            var c = ch; e.keyboardSetUnicodeString(stringLength: 1, unicodeString: &c); e.post(tap: .cghidEventTap); usleep(20000)
        }
    }
case "click":
    let p = CGPoint(x: Double(a[2])!, y: Double(a[3])!)
    let n = a.count > 4 ? Int(a[4])! : 1
    moved(p); usleep(30000)
    for i in 1...n {
        for t in [CGEventType.leftMouseDown, .leftMouseUp] {
            let e = CGEvent(mouseEventSource: nil, mouseType: t, mouseCursorPosition: p, mouseButton: .left)!
            e.setIntegerValueField(.mouseEventClickState, value: Int64(i))
            e.post(tap: .cghidEventTap); usleep(35000)
        }
        usleep(70000)
    }
case "glide":
    let x1 = Double(a[2])!, y1 = Double(a[3])!, x2 = Double(a[4])!, y2 = Double(a[5])!
    let steps = max(1, Int(Double(a[6])! / 16))
    for i in 0...steps {
        let t = Double(i) / Double(steps), e = t < 0.5 ? 2 * t * t : 1 - pow(-2 * t + 2, 2) / 2
        moved(CGPoint(x: x1 + (x2 - x1) * e, y: y1 + (y2 - y1) * e))
        usleep(16000)
    }
case "scroll":
    moved(CGPoint(x: Double(a[2])!, y: Double(a[3])!)); usleep(30000)
    for _ in 0..<Int(a[5])! {
        CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: Int32(a[4])!, wheel2: 0, wheel3: 0)?.post(tap: .cghidEventTap)
        usleep(16000)
    }
default: print("?")
}
