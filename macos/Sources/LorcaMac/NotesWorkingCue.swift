import AppKit
import QuartzCore

/// A quiet margin cue. It owns no document content and never intercepts typing.
final class NotesWorkingCue: NSView {
    init(reduceMotion: Bool = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion) {
        super.init(frame: NSRect(x: 0, y: 0, width: 18, height: 20))
        wantsLayer = true
        setAccessibilityElement(true)
        setAccessibilityRole(.progressIndicator)
        setAccessibilityLabel("Model working on this passage")
        setAccessibilityIdentifier("notes-model-working")
        toolTip = "Model following"
        for index in 0..<3 {
            let dot = CALayer()
            dot.frame = CGRect(x: 2 + index * 5, y: 8, width: 3, height: 3)
            dot.cornerRadius = 1.5
            dot.backgroundColor = NSColor.controlAccentColor.withAlphaComponent(0.8).cgColor
            dot.opacity = 0.55
            layer?.addSublayer(dot)
            guard !reduceMotion else { continue }
            let pulse = CAKeyframeAnimation(keyPath: "opacity")
            pulse.values = [0.25, 0.8, 0.25]
            pulse.keyTimes = [0, 0.5, 1]
            pulse.duration = 1.6
            pulse.beginTime = CACurrentMediaTime() + Double(index) * 0.15
            pulse.repeatCount = .infinity
            pulse.timingFunctions = [CAMediaTimingFunction(name: .easeInEaseOut), CAMediaTimingFunction(name: .easeInEaseOut)]
            dot.add(pulse, forKey: "following")
        }
    }

    required init?(coder: NSCoder) { nil }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override var acceptsFirstResponder: Bool { false }
}
