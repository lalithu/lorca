import AppKit
import SwiftUI

// One editable document. Formatting changes attributes, never the saved Markdown.
struct MarkdownNoteEditor: NSViewRepresentable {
    @Binding var text: String
    var size: CGFloat

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let editor = Editor()
        editor.isRichText = false
        editor.importsGraphics = false
        editor.allowsUndo = true
        editor.drawsBackground = false
        editor.isVerticallyResizable = true
        editor.isHorizontallyResizable = false
        editor.autoresizingMask = [.width]
        editor.textContainer?.widthTracksTextView = true
        editor.textContainer?.lineFragmentPadding = 0
        editor.textContainerInset = NSSize(width: 12, height: 10)
        editor.isAutomaticQuoteSubstitutionEnabled = false
        editor.isAutomaticDashSubstitutionEnabled = false
        editor.isAutomaticSpellingCorrectionEnabled = false
        editor.delegate = context.coordinator
        editor.setAccessibilityIdentifier("job-notes-editor")
        editor.setAccessibilityLabel("Notes")
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = editor
        return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let editor = scroll.documentView as? Editor else { return }
        if editor.string != text { editor.string = text }
        if editor.formattedText != text || editor.textSize != size { editor.format(size: size) }
    }
    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: MarkdownNoteEditor
        init(_ parent: MarkdownNoteEditor) { self.parent = parent }
        func textDidChange(_ notification: Notification) {
            guard let editor = notification.object as? Editor else { return }
            parent.text = editor.string
            editor.format(size: parent.size)
        }
    }
    final class Editor: NSTextView {
        var formattedText: String?
        var textSize: CGFloat = 13
        func format(size: CGFloat) {
            guard let storage = textStorage else { return }
            let selection = selectedRanges
            let undo = undoManager
            let recording = undo?.isUndoRegistrationEnabled == true
            if recording { undo?.disableUndoRegistration() }
            MarkdownNoteStyle.apply(to: storage, size: size)
            if recording { undo?.enableUndoRegistration() }
            selectedRanges = selection
            typingAttributes = MarkdownNoteStyle.base(size: size)
            insertionPointColor = .labelColor
            formattedText = string; textSize = size
        }
    }
}

enum MarkdownNoteStyle {
    static func base(size: CGFloat) -> [NSAttributedString.Key: Any] {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = 3
        return [.font: NSFont.systemFont(ofSize: size), .foregroundColor: NSColor.labelColor, .paragraphStyle: paragraph]
    }
    static func apply(to text: NSMutableAttributedString, size: CGFloat) {
        let source = text.string as NSString
        let whole = NSRange(location: 0, length: source.length)
        text.beginEditing()
        defer { text.endEditing() }
        text.setAttributes(base(size: size), range: whole)
        func matches(_ pattern: String) -> [NSTextCheckingResult] {
            (try? NSRegularExpression(pattern: pattern))?.matches(in: source as String, range: whole) ?? []
        }
        func conceal(_ range: NSRange) {
            guard range.length > 0 else { return }
            text.addAttributes([.foregroundColor: NSColor.clear, .font: NSFont.systemFont(ofSize: 0.1)], range: range)
        }
        let fences = matches(#"(?ms)^(`{3,}|~{3,})[^\n]*\n(.*?)(?:^\1[ \t]*(?:\n|$)|\z)"#)
        let codes = matches(#"(`+)([^`\n]+)\1"#).filter { match in
            !fences.contains { NSIntersectionRange($0.range, match.range).length > 0 }
        }
        func outsideCode(_ range: NSRange) -> Bool {
            !(fences + codes).contains { NSIntersectionRange($0.range, range).length > 0 }
        }
        for match in matches(#"(?m)^(#{1,6})[ \t]+(.+)$"#) where outsideCode(match.range) {
            text.addAttribute(.font, value: NSFont.systemFont(ofSize: size + CGFloat(7 - match.range(at: 1).length), weight: .semibold), range: match.range)
            conceal(NSRange(location: match.range.location, length: match.range(at: 2).location - match.range.location))
        }
        for (pattern, trait) in [(#"(\*\*|__)([^\n]+?)\1"#, NSFontTraitMask.boldFontMask), (#"(?<![\*_])(\*|_)([^\*_\n]+)\1(?![\*_])"#, .italicFontMask)] {
            for match in matches(pattern) where outsideCode(match.range) {
                let content = match.range(at: 2)
                let font = text.attribute(.font, at: content.location, effectiveRange: nil) as? NSFont ?? .systemFont(ofSize: size)
                text.addAttribute(.font, value: NSFontManager.shared.convert(font, toHaveTrait: trait), range: content)
                conceal(match.range(at: 1))
                conceal(NSRange(location: NSMaxRange(content), length: match.range(at: 1).length))
            }
        }
        for match in matches(#"~~([^\n]+?)~~"#) where outsideCode(match.range) {
            text.addAttribute(.strikethroughStyle, value: NSUnderlineStyle.single.rawValue, range: match.range(at: 1))
            conceal(NSRange(location: match.range.location, length: 2))
            conceal(NSRange(location: NSMaxRange(match.range) - 2, length: 2))
        }
        for match in matches(#"\[([^\]\n]+)\]\((https?://[^\s)]+)\)"#) where outsideCode(match.range) {
            let label = match.range(at: 1)
            if let url = URL(string: source.substring(with: match.range(at: 2))) {
                text.addAttributes([.link: url, .foregroundColor: NSColor.linkColor], range: label)
            }
            conceal(NSRange(location: match.range.location, length: 1))
            conceal(NSRange(location: NSMaxRange(label), length: NSMaxRange(match.range) - NSMaxRange(label)))
        }
        for match in fences + codes {
            let body = match.range(at: 2)
            text.addAttribute(.font, value: NSFont.monospacedSystemFont(ofSize: size, weight: .regular), range: body)
            conceal(NSRange(location: match.range.location, length: body.location - match.range.location))
            conceal(NSRange(location: NSMaxRange(body), length: NSMaxRange(match.range) - NSMaxRange(body)))
        }
    }
}
