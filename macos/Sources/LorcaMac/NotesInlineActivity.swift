import AppKit

/// Only transient activity or a local retry. Results live in NSTextStorage.
final class NotesInlineActivity: NSButton {
    let cardHeight: CGFloat = 24
    var perform: (() -> Void)?
    private let headline = NSTextField(labelWithString: "")
    init(state: NotesInlineState) {
        super.init(frame: .zero)
        isBordered = false
        title = ""
        target = self; action = #selector(activate)
        headline.stringValue = state.label
        headline.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        headline.textColor = state.busy ? .controlAccentColor : .secondaryLabelColor
        headline.lineBreakMode = .byTruncatingTail
        headline.isSelectable = false
        addSubview(headline)
        setAccessibilityIdentifier("notes-inline-activity")
        setAccessibilityLabel(state.label)
        toolTip = state.detail ?? state.label
    }
    required init?(coder: NSCoder) { nil }
    override var acceptsFirstResponder: Bool { false }
    override var isFlipped: Bool { true }
    override func layout() {
        super.layout()
        headline.frame = NSRect(x: 1, y: 4, width: max(0, bounds.width - 2), height: 17)
    }
    override func hitTest(_ point: NSPoint) -> NSView? { bounds.contains(point) ? self : nil }
    @objc private func activate() { perform?() }
}
