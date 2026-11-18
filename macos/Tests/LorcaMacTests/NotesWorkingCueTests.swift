import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class NotesWorkingCueTests: XCTestCase {
    func testWorkingCueLeavesWritingAloneAndCannotReturnAfterCancellation() {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let editor = NotesNativeTextView(frame: NSRect(x: 0, y: 0, width: 600, height: 400))
        editor.textContainerInset = NSSize(width: 22, height: 0)
        editor.string = "Study guide for NLP\nTransformers and attention"
        editor.setSelectedRange(NSRange(location: 4, length: 0))
        window.contentView = editor
        editor.format()
        let before = editor.string, selection = editor.selectedRange()
        editor.showWorking(true, document: before, at: 4)
        XCTAssertTrue(editor.subviews.compactMap { $0 as? NotesWorkingCue }.isEmpty)
        let shown = expectation(description: "Quiet delayed indicator")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            XCTAssertEqual(editor.subviews.compactMap { $0 as? NotesWorkingCue }.count, 1)
            XCTAssertEqual(editor.string, before)
            XCTAssertEqual(editor.selectedRange(), selection)
            editor.showWorking(false, document: before, at: 4)
            XCTAssertTrue(editor.subviews.compactMap { $0 as? NotesWorkingCue }.isEmpty)
            editor.showWorking(true, document: before, at: 4)
            editor.showWorking(false, document: before, at: 4)
            shown.fulfill()
        }
        wait(for: [shown], timeout: 2)
        let cancelled = expectation(description: "Old timer stays cancelled")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            XCTAssertTrue(editor.subviews.compactMap { $0 as? NotesWorkingCue }.isEmpty)
            cancelled.fulfill()
        }
        wait(for: [cancelled], timeout: 2)
    }

    func testInlineActivityReservesSpaceWithoutChangingTextSelectionOrUndo() throws {
        _ = NSApplication.shared
        let editor = NotesNativeTextView(frame: NSRect(x: 0, y: 0, width: 650, height: 450))
        editor.modelDriven = true
        editor.allowsUndo = true
        editor.textContainerInset = NSSize(width: 22, height: 0)
        let first = "Resturants in Noda:"
        editor.string = first + "\n\nStudy guide for NLP"
        editor.setSelectedRange(NSRange(location: (editor.string as NSString).length, length: 0))
        editor.format()
        let text = editor.string, selection = editor.selectedRange()
        editor.showInline([NotesInlineState(source: first, label: "Thinking…", busy: true)])
        editor.layout()
        XCTAssertEqual(editor.subviews.compactMap { $0 as? NotesInlineActivity }.count, 1)
        editor.showInline([NotesInlineState(source: first, label: "Finding places…", busy: true)])
        editor.layout()
        XCTAssertEqual(editor.string, text)
        XCTAssertEqual(editor.selectedRange(), selection)
        XCTAssertFalse(editor.undoManager?.canUndo ?? false)
        let card = try XCTUnwrap(editor.subviews.compactMap { $0 as? NotesInlineActivity }.first)
        let manager = try XCTUnwrap(editor.layoutManager), container = try XCTUnwrap(editor.textContainer)
        let next = (text as NSString).range(of: "Study guide")
        let glyphs = manager.glyphRange(forCharacterRange: next, actualCharacterRange: nil)
        let rect = manager.boundingRect(forGlyphRange: glyphs, in: container)
        XCTAssertLessThanOrEqual(card.frame.maxY, rect.minY + editor.textContainerOrigin.y)
        if let path = ProcessInfo.processInfo.environment["LORCA_NOTES_PREVIEW"] {
            editor.appearance = NSAppearance(named: .darkAqua)
            editor.displayIfNeeded()
            if let bitmap = editor.bitmapImageRepForCachingDisplay(in: editor.bounds) {
                editor.cacheDisplay(in: editor.bounds, to: bitmap)
                try bitmap.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
            }
        }
        editor.showInline([])
        XCTAssertTrue(editor.subviews.compactMap { $0 as? NotesInlineActivity }.isEmpty)
    }

    func testResultAboveCaretPreservesItsVisualPositionWhileWriting() throws {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 300), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 600, height: 300))
        let editor = NotesNativeTextView(frame: NSRect(x: 0, y: 0, width: 600, height: 1800))
        editor.isVerticallyResizable = true
        editor.textContainer?.widthTracksTextView = true
        editor.textContainerInset = NSSize(width: 22, height: 0)
        let source = "Study guide on GNN architecture"
        editor.string = source + "\n\n" + String(repeating: "More draft text while another passage is running.\n", count: 60)
        editor.format()
        let caret = NSRange(location: (source as NSString).length + 2 + 49 * 35, length: 0)
        editor.setSelectedRange(caret)
        scroll.documentView = editor
        window.contentView = scroll
        window.makeFirstResponder(editor)
        editor.scrollRangeToVisible(caret)
        let manager = try XCTUnwrap(editor.layoutManager), container = try XCTUnwrap(editor.textContainer)
        func visualY() -> CGFloat {
            manager.ensureLayout(for: container)
            let glyphs = manager.glyphRange(forCharacterRange: NSRange(location: caret.location, length: 1), actualCharacterRange: nil)
            return manager.boundingRect(forGlyphRange: glyphs, in: container).minY - scroll.contentView.bounds.minY
        }
        let before = visualY()
        let proposal = NotesProposal(kind: "breakdown", source: source, title: "GNN study guide", text: "- Message passing\n- Aggregation", headers: nil, rows: nil)
        editor.showInline([NotesInlineState(source: source, label: proposal.title, unread: true, proposal: proposal)])
        XCTAssertEqual(visualY(), before, accuracy: 1)
        XCTAssertEqual(editor.selectedRange(), caret)
        XCTAssertTrue(window.firstResponder === editor)
    }

    func testReducedMotionCueIsStaticAndNeverTakesInput() {
        let cue = NotesWorkingCue(reduceMotion: true)
        XCTAssertNil(cue.hitTest(NSPoint(x: 5, y: 5)))
        XCTAssertFalse(cue.acceptsFirstResponder)
        XCTAssertEqual(cue.layer?.sublayers?.count, 3)
        XCTAssertTrue(cue.layer?.sublayers?.allSatisfy { $0.animationKeys()?.isEmpty ?? true } == true)
    }
}
