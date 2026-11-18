import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class NotesEmsTests: XCTestCase {
    func testMarginCueAppearsAfterPauseAndCanBeStopped() {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let editor = NotesNativeTextView(frame: NSRect(x: 0, y: 0, width: 600, height: 400))
        editor.textContainerInset = NSSize(width: 22, height: 0)
        editor.string = "Table with data:\nName | Cost\nOrca | Low"
        window.contentView = editor
        editor.format()
        editor.queueSuggestion()
        XCTAssertTrue(editor.subviews.compactMap { $0 as? NSButton }.isEmpty)
        let cue = expectation(description: "Margin cue after writing pause")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.4) {
            let marker = editor.subviews.compactMap { $0 as? NSButton }.first
            XCTAssertNotNil(marker)
            XCTAssertFalse(marker?.isHidden ?? true)
            XCTAssertLessThan(marker?.frame.maxX ?? 100, editor.textContainerOrigin.x)
            editor.previewsPaused = true
            editor.resetSuggestions()
            XCTAssertTrue(editor.subviews.compactMap { $0 as? NSButton }.isEmpty)
            cue.fulfill()
        }
        wait(for: [cue], timeout: 3)
    }

    func testCueNeedsDataAndPreservesTheRestOfTheDocument() throws {
        XCTAssertNil(NotesTableSuggestion.detect(in: "Table with data:"))
        let text = "Before 🐬\n\nTable with data:\nName | Cost\nA | Low\nB |\n\nKeep writing here."
        let suggestion = try XCTUnwrap(NotesTableSuggestion.detect(in: text))
        XCTAssertEqual(suggestion.headers, ["Name", "Cost"])
        XCTAssertEqual(suggestion.rows, [["A", "Low"], ["B", ""]])
        XCTAssertTrue(suggestion.stillMatches(text))
        XCTAssertFalse(suggestion.stillMatches(text.replacingOccurrences(of: "Low", with: "High")))
        let replaced = (text as NSString).replacingCharacters(in: suggestion.sourceRange, with: "[TABLE]")
        XCTAssertTrue(replaced.hasPrefix("Before 🐬\n\n"))
        XCTAssertTrue(replaced.hasSuffix("\nKeep writing here."))
    }

    func testUnstructuredRowsAreKeptWithoutInventedLabelsOrFacts() throws {
        let suggestion = try XCTUnwrap(NotesTableSuggestion.detect(in: "Table with data:\napproach A cheap, harder to maintain\napproach B expensive, straightforward"))
        XCTAssertEqual(suggestion.headers, ["Details"])
        XCTAssertEqual(suggestion.rows.count, 2)
        XCTAssertEqual(suggestion.rows[0], ["approach A cheap, harder to maintain"])
        XCTAssertNil(NotesTableSuggestion.detect(in: "Table:\nA | B\nx | y | do not drop me"))
    }

    func testNativeTableRoundTripsAndCanBeUndone() throws {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let editor = NotesNativeTextView(frame: NSRect(x: 0, y: 0, width: 600, height: 400))
        editor.isRichText = true
        editor.allowsUndo = true
        window.contentView = editor
        window.makeFirstResponder(editor)
        let original = "Table with data:\nName | Cost\nOrca | Low\n"
        editor.string = original
        editor.setSelectedRange(NSRange(location: (original as NSString).length, length: 0))
        editor.applySuggestion(try XCTUnwrap(NotesTableSuggestion.detect(in: original)))
        XCTAssertTrue(editor.string.contains("Orca"))
        let data = try XCTUnwrap(editor.richDocument())
        let restored = try NSAttributedString(data: data, options: [.documentType: NSAttributedString.DocumentType.rtf], documentAttributes: nil)
        XCTAssertEqual(restored.string, editor.string)
        editor.format()
        XCTAssertNotNil(try editor.richDocument(), "Restyling must keep the native table")
        XCTAssertTrue(editor.undoManager?.canUndo == true)
        editor.undoManager?.undo()
        XCTAssertEqual(editor.string, original)
    }

    func testRichTableIsSavedAndRestoredWithItsNote() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let suggestion = try XCTUnwrap(NotesTableSuggestion.detect(in: "Table:\nName | Cost\nOrca | Low"))
        let table = NotesNativeTable.attributed(suggestion)
        let rich = try table.data(from: NSRange(location: 0, length: table.length), documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf])
        let store = NotesStore(root: root)
        store.updateText(table.string, selection: NSRange(location: 0, length: 0), richText: rich)
        let loaded = NotesStore(root: root)
        XCTAssertEqual(loaded.current.text, table.string)
        XCTAssertEqual(loaded.current.richText, rich)
    }
}
