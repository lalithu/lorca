import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class NotesStoreTests: XCTestCase {
    func testNewNotePreservesEarlierWritingAndRestoresSelectedNote() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = NotesStore(root: root)
        let firstID = store.selectedID
        let text = "First note 🐬\nwith a second line"
        store.updateText(text, selection: NSRange(location: 3, length: 0))
        store.create()
        XCTAssertNotEqual(store.selectedID, firstID)
        XCTAssertEqual(store.current.text, "")
        XCTAssertEqual(store.current.selection, NSRange(location: 0, length: 0))
        let secondID = store.selectedID
        store.updateText("Latest note", selection: NSRange(location: 6, length: 0))
        let restored = NotesStore(root: root)
        XCTAssertEqual(restored.selectedID, secondID)
        XCTAssertEqual(restored.current.text, "Latest note")
        XCTAssertEqual(restored.current.selection.location, 6)
        restored.select(firstID)
        XCTAssertEqual(restored.current.text, text)
        XCTAssertEqual(restored.current.selection.location, 3)
        XCTAssertNil(restored.error)
    }

    func testUnreadableNotesAreNotOverwritten() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let folder = root.appendingPathComponent("notes")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let file = folder.appendingPathComponent("workspace.json")
        let original = Data("not valid JSON".utf8)
        try original.write(to: file)
        let store = NotesStore(root: root)
        XCTAssertTrue(store.isReadOnly)
        XCTAssertNotNil(store.error)
        store.create()
        store.updateText("replacement", selection: NSRange(location: 0, length: 0))
        XCTAssertEqual(try Data(contentsOf: file), original)
    }

    func testNativeEditorKeepsUnicodeAndSelectionWhenFormatting() {
        _ = NSApplication.shared
        let editor = NotesNativeTextView()
        editor.string = "🐬 **A note**"
        let selection = NSRange(location: (editor.string as NSString).length, length: 0)
        editor.setSelectedRange(selection)
        editor.format()
        XCTAssertEqual(editor.string, "🐬 **A note**")
        XCTAssertEqual(editor.selectedRange(), selection)
        XCTAssertEqual(editor.insertionPointColor, .controlAccentColor)
    }
}
