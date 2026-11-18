import AppKit

/// A document decoration, never editable text or an undo operation.
final class NotesInlineActivity: NSButton {
    let cardHeight: CGFloat
    var perform: (() -> Void)?
    private let headline = NSTextField(labelWithString: "")
    private let excerpt = NSTextField(wrappingLabelWithString: "")

    init(state: NotesInlineState) {
        cardHeight = state.proposal == nil ? 24 : 104
        super.init(frame: .zero)
        isBordered = false
        title = ""
        target = self; action = #selector(activate)
        wantsLayer = true
        layer?.cornerRadius = 7
        layer?.backgroundColor = (state.unread ? NSColor.controlAccentColor.withAlphaComponent(0.10) : NSColor.clear).cgColor
        headline.stringValue = state.label + (state.proposal == nil ? "" : "  ↗")
        headline.font = .systemFont(ofSize: 11, weight: .medium)
        headline.textColor = state.unread || state.busy ? .controlAccentColor : .secondaryLabelColor
        headline.lineBreakMode = .byTruncatingTail
        headline.isSelectable = false
        addSubview(headline)
        if let proposal = state.proposal {
            let body: String
            if let text = proposal.text { body = text }
            else { body = ([proposal.headers?.joined(separator: " · ") ?? ""] + (proposal.rows ?? []).prefix(3).map { $0.joined(separator: " · ") }).joined(separator: "\n") }
            if let markdown = try? AttributedString(markdown: String(body.prefix(700)), options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)) {
                excerpt.attributedStringValue = NSAttributedString(markdown)
            } else { excerpt.stringValue = String(body.prefix(700)) }
            excerpt.font = .systemFont(ofSize: 12)
            excerpt.textColor = .secondaryLabelColor
            excerpt.maximumNumberOfLines = 4
            excerpt.isSelectable = false
            addSubview(excerpt)
        }
        setAccessibilityIdentifier("notes-inline-activity")
        setAccessibilityLabel(state.label)
        toolTip = state.proposal == nil ? state.label : "Open result · review or insert"
    }
    required init?(coder: NSCoder) { nil }
    override var acceptsFirstResponder: Bool { false }
    override var isFlipped: Bool { true }
    override func layout() {
        super.layout()
        headline.frame = NSRect(x: 9, y: 4, width: max(0, bounds.width - 18), height: 17)
        excerpt.frame = NSRect(x: 9, y: 27, width: max(0, bounds.width - 18), height: max(0, bounds.height - 34))
    }
    override func hitTest(_ point: NSPoint) -> NSView? { bounds.contains(point) ? self : nil }
    @objc private func activate() { perform?() }
}
