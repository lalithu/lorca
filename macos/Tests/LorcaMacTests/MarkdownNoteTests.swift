import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class MarkdownNoteTests: XCTestCase {
    func testFormattingKeepsMarkdownAndUnicodeSourceIntact() {
        let source = "🐋\n# Heading\n\nA **bold** and *italic* note, ~~removed~~, and [link](https://example.com)."
        let text = NSMutableAttributedString(string: source)
        MarkdownNoteStyle.apply(to: text, size: 13)
        XCTAssertEqual(text.string, source)
        func font(_ needle: String) -> NSFont {
            text.attribute(.font, at: (source as NSString).range(of: needle).location, effectiveRange: nil) as! NSFont
        }
        XCTAssertGreaterThan(font("Heading").pointSize, 13)
        XCTAssertTrue(NSFontManager.shared.traits(of: font("bold")).contains(.boldFontMask))
        XCTAssertTrue(NSFontManager.shared.traits(of: font("italic")).contains(.italicFontMask))
        XCTAssertLessThan(font("#").pointSize, 1)
        XCTAssertNotNil(text.attribute(.link, at: (source as NSString).range(of: "link").location, effectiveRange: nil))
    }

    func testCodeDoesNotInterpretMarkdownAndBlankLinesRemain() {
        let source = "```swift\n# literal\n**literal**\n```\n\n`**inline**`\n\nLast line"
        let text = NSMutableAttributedString(string: source)
        MarkdownNoteStyle.apply(to: text, size: 14)
        XCTAssertEqual(text.string, source)
        for needle in ["# literal", "**literal**", "**inline**"] {
            let font = text.attribute(.font, at: (source as NSString).range(of: needle).location, effectiveRange: nil) as! NSFont
            XCTAssertEqual(font.pointSize, 14)
            XCTAssertTrue(font.fontDescriptor.symbolicTraits.contains(.monoSpace))
        }
    }

    func testRestylingAndZoomPreserveSelectionAndTypedText() {
        _ = NSApplication.shared
        let editor = MarkdownNoteEditor.Editor()
        editor.string = "# Heading\n\n**orca** 🐋"
        let caret = NSRange(location: (editor.string as NSString).length, length: 0)
        editor.setSelectedRange(caret)
        editor.format(size: 13)
        XCTAssertEqual(editor.selectedRange(), caret)
        editor.insertText("!", replacementRange: caret)
        let typed = editor.string, selection = editor.selectedRange()
        editor.format(size: 16)
        XCTAssertEqual(editor.string, typed)
        XCTAssertEqual(editor.selectedRange(), selection)
        XCTAssertTrue(editor.string.hasSuffix("🐋!"))
    }
}
