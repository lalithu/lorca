import XCTest
@testable import LorcaMac

@MainActor
final class ConversationTests: XCTestCase {
    func testProjectionKeepsTypingAheadOfOlderServerSnapshots() throws {
        let backend = MockBackend()
        let library = ConversationLibrary()
        library.apply(try backend.snapshot(), selected: backend.selected)
        library.setDraft("New text")
        library.apply(try backend.snapshot(), selected: backend.selected)
        XCTAssertEqual(library.current.draft, "New text")
        library.acknowledgedDraft("old text", id: backend.selected)
        library.apply(try backend.snapshot(), selected: backend.selected)
        XCTAssertEqual(library.current.draft, "New text")
        library.acknowledgedDraft("New text", id: backend.selected)
        backend.chats[0].draft = "Confirmed"
        library.apply(try backend.snapshot(), selected: backend.selected)
        XCTAssertEqual(library.current.draft, "Confirmed")
    }
    func testHistoryActionsDelegateToRustAndDoNotWriteStorage() {
        let library = ConversationLibrary()
        var operations: [String] = []
        library.perform = { operations.append($0["op"] as! String) }
        library.rename(library.selectedID, to: "Notes")
        library.removeDraftImage(UUID())
        XCTAssertEqual(operations, ["rename", "removeImage"])
    }
    func testExistingFoundationDatesAndAttachmentIDsRemainCompatible() throws {
        let chat = Conversation()
        let decoded = try JSONDecoder().decode(Conversation.self, from: JSONEncoder().encode(chat))
        XCTAssertEqual(chat, decoded)
        let message = Data("{\"id\":\"\(UUID())\",\"role\":\"assistant\",\"content\":\"Partial\"}".utf8)
        XCTAssertTrue(try JSONDecoder().decode(ChatMessage.self, from: message).attachments.isEmpty)
    }
}
