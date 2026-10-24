import XCTest
@testable import LorcaMac

@MainActor
final class StreamingPersistenceTests: XCTestCase {
    func testCancelledStreamRemainsInProjectionAndOpeningAnotherChatIsOrderedAfterCancel() async throws {
        let backend = MockBackend()
        let store = ChatStore(backend: backend)
        await store.refreshConnection()
        store.draft = "Hello"
        store.send()
        try await Task.sleep(for: .milliseconds(30))
        backend.chats[0].messages[1].content = "Saved partial"
        backend.revision += 1
        backend.emit()
        store.newChat()
        try await Task.sleep(for: .milliseconds(40))
        XCTAssertTrue(store.messages.isEmpty)
        XCTAssertEqual(store.library.conversations[0].messages.last?.content, "Saved partial")
        let operations = backend.requests.compactMap { $0["op"] as? String }
        XCTAssertEqual(Array(operations.suffix(2)), ["cancel", "new"])
        _ = await store.modelSession.close(unload: false)
    }
}
