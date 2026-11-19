import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class NotesControlsTests: XCTestCase {
    private func root() -> URL { FileManager.default.temporaryDirectory.appendingPathComponent("lorca-controls-\(UUID())") }
    private func key(_ shifted: Bool = false) -> NSEvent {
        NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: shifted ? [.command,.shift] : [.command], timestamp: 0, windowNumber: 0, context: nil, characters: shifted ? "Z" : "z", charactersIgnoringModifiers: "z", isARepeat: false, keyCode: 6)!
    }
    private func flush() { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }

    func testCommandZAndRedoReachTheDocumentUndoManager() throws {
        _ = NSApplication.shared
        let root = root(); defer { try? FileManager.default.removeItem(at: root) }
        let notes = NotesStore(root: root), source = "Study guide for counting"
        notes.updateText(source, selection: NSRange(location: 0, length: 0))
        let document = notes.document(for: notes.selectedID)
        try document.add(NotesProposal(kind: "breakdown", source: source, title: "Counting", text: "Count independent choices by multiplication.", headers: nil, rows: nil))
        flush()
        XCTAssertTrue(document.editor.responds(to: NSSelectorFromString("undo:")))
        XCTAssertTrue(document.editor.performKeyEquivalent(with: key()))
        XCTAssertEqual(notes.current.text, source)
        XCTAssertTrue(document.editor.performKeyEquivalent(with: key(true)))
        XCTAssertTrue(notes.current.text.contains("independent choices"))
    }

    func testOriginalStaysAndCheckedTasksMoveToBottomWithUndo() throws {
        _ = NSApplication.shared
        let root = root(); defer { try? FileManager.default.removeItem(at: root) }
        let notes = NotesStore(root: root), source = "Things to Do:\nWrite blog\nSend email\nStudy counting"
        notes.updateText(source, selection: NSRange(location: 0, length: 0))
        let document = notes.document(for: notes.selectedID)
        try document.add(NotesProposal(kind: "checklist", source: source, title: "Tasks", text: "- [ ] Write blog\n- [ ] Send email\n- [ ] Study counting", headers: nil, rows: nil))
        flush()
        let before = document.editor.string
        let marker = (before as NSString).range(of: "☐ Write blog").location
        document.editor.toggleChecklist(at: marker)
        flush()
        XCTAssertTrue(notes.current.text.hasPrefix(source))
        XCTAssertTrue(notes.current.text.contains("☐ Send email\n☐ Study counting\n☑ Write blog"))
        XCTAssertTrue(document.editor.performKeyEquivalent(with: key()))
        XCTAssertEqual(document.editor.string, before)
    }

    func testEditingResponseCreatesLocalFollowupAndAmberWordsWithoutLosingOriginal() throws {
        _ = NSApplication.shared
        let root = root(); defer { try? FileManager.default.removeItem(at: root) }
        let notes = NotesStore(root: root), source = "Study guide for counting and permutations"
        notes.updateText(source, selection: NSRange(location: 0, length: 0))
        let document = notes.document(for: notes.selectedID)
        try document.add(NotesProposal(kind: "breakdown", source: source, title: "Study guide", text: "### Circular permutations\nFormula: (n-1)!.\n\n### Repetitions\nDivide by repeated factorials.", headers: nil, rows: nil))
        let location = NSMaxRange((document.editor.string as NSString).range(of: "Formula: (n-1)!."))
        document.editor.insertText(" Give me more here", replacementRange: NSRange(location: location, length: 0))
        let followup = try XCTUnwrap(notes.current.passages.first { $0.context != nil })
        XCTAssertTrue(followup.source.contains("Give me more here"))
        XCTAssertTrue(followup.context!.contains("Circular permutations"))
        XCTAssertFalse(followup.context!.contains("Divide by repeated"))
        XCTAssertEqual(document.editor.textStorage?.attribute(.foregroundColor, at: location + 1, effectiveRange: nil) as? NSColor, .systemOrange)
        XCTAssertTrue(notes.current.modelBlocks!.first!.edited)
        try document.add(NotesProposal(kind: "breakdown", source: followup.source, title: "Example", text: "For five people around a table, 4! = 24 arrangements.", headers: nil, rows: nil))
        XCTAssertEqual(notes.current.passages.first { $0.context != nil }?.source, followup.source)
        XCTAssertEqual(notes.current.modelBlocks?.filter { !$0.edited }.count, 1)
        XCTAssertTrue(notes.current.text.contains("### Repetitions"))
        let reopened = NotesStore(root: root)
        XCTAssertFalse(reopened.document(for: reopened.selectedID).editor.userEdits().isEmpty)
    }

    func testScreenshotFileDropUsesTheSameImageImportPath() async throws {
        _ = NSApplication.shared
        let root = root(); defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let image = root.appendingPathComponent("Screenshot.jpg")
        try NotesDocumentTests.flyerJPEG().write(to: image)
        let notes = NotesStore(root: root), document = notes.document(for: notes.selectedID)
        let pasteboard = NSPasteboard.withUniqueName(); defer { pasteboard.releaseGlobally() }
        pasteboard.writeObjects([image as NSURL])
        XCTAssertTrue(document.editor.acceptsImageDrop(pasteboard))
        XCTAssertTrue(document.editor.acceptImageDrop(pasteboard, at: NSPoint(x: 25, y: 2)))
        for _ in 0..<100 {
            if !(notes.current.images ?? []).isEmpty { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(notes.current.images?.count, 1)
    }
}
