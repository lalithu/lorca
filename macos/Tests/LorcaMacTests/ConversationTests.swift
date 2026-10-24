import XCTest
@testable import LorcaMac

@MainActor
final class ConversationTests: XCTestCase {
    private var repository: ConversationRepository!
    override func setUp() async throws {
        repository = ConversationRepository(root: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
    }
    override func tearDown() async throws { try? FileManager.default.removeItem(at: repository.root) }

    func testCreateSwitchRenameAndRestoreMessagesAndDrafts() throws {
        let library = ConversationLibrary(repository: repository)
        let first = library.selectedID
        library.edit { $0.messages = [ChatMessage(role: .user, content: "Hello"), ChatMessage(role: .assistant, content: "Partial reply")]; $0.draft = "Next question" }
        library.create()
        let second = library.selectedID
        library.edit { $0.messages = [ChatMessage(role: .user, content: "Second chat")] }
        library.rename(first, to: "  Orca notes  ")
        library.open(first)
        XCTAssertTrue(library.flush())
        let restored = ConversationLibrary(repository: repository)
        XCTAssertEqual(restored.selectedID, first)
        XCTAssertEqual(restored.current.title, "Orca notes")
        XCTAssertEqual(restored.current.messages.map(\.content), ["Hello", "Partial reply"])
        XCTAssertEqual(restored.current.draft, "Next question")
        restored.open(second)
        XCTAssertEqual(restored.current.messages.map(\.content), ["Second chat"])
    }

    func testStreamCheckpointsBeforeCompletion() async throws {
        let library = ConversationLibrary(repository: repository)
        library.edit { $0.messages = [ChatMessage(role: .assistant, content: "A partial response")] }
        try await Task.sleep(for: .milliseconds(350))
        XCTAssertEqual(try repository.load().conversations.first?.messages.last?.content, "A partial response")
    }

    func testCorruptChatAndSelectionLeaveHealthyChatsUsableAndFilesIntact() throws {
        let library = ConversationLibrary(repository: repository)
        library.rename(library.selectedID, to: "Healthy")
        let corrupt = repository.chatsDirectory.appendingPathComponent("\(UUID()).json")
        try Data("broken json".utf8).write(to: corrupt)
        try Data("broken selection".utf8).write(to: repository.root.appendingPathComponent("selected.json"))
        let restored = ConversationLibrary(repository: repository)
        XCTAssertEqual(restored.current.title, "Healthy")
        XCTAssertNotNil(restored.error)
        XCTAssertEqual(try Data(contentsOf: corrupt), Data("broken json".utf8))
        restored.create()
        XCTAssertEqual(try Data(contentsOf: corrupt), Data("broken json".utf8))
    }

    func testDeleteCleansOnlyUnreferencedImagesAndKeepsAnotherChat() throws {
        let library = ConversationLibrary(repository: repository)
        let shared = try repository.imageStore.importData(imageFixture(), name: "shared")
        let unique = try repository.imageStore.importData(imageFixture(), name: "unique")
        let first = library.selectedID
        library.edit { $0.messages = [ChatMessage(role: .user, content: "First", attachments: [shared, unique])] }
        library.create()
        let second = library.selectedID
        library.edit { $0.draftImages = [shared] }
        library.flush()
        library.delete(first)
        XCTAssertEqual(library.selectedID, second)
        XCTAssertEqual(library.conversations.count, 1)
        XCTAssertFalse(FileManager.default.fileExists(atPath: repository.imageStore.url(for: unique).path))
        XCTAssertNoThrow(try repository.imageStore.data(for: shared))
        library.delete(second)
        XCTAssertEqual(library.conversations.count, 1)
        XCTAssertTrue(library.current.messages.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: repository.imageStore.url(for: shared).path))
        XCTAssertFalse(try repository.load().conversations.contains { $0.id == first || $0.id == second })
    }

    func testCorruptMetadataPreventsUnsafeImageCleanup() throws {
        let attachment = try repository.imageStore.importData(imageFixture(), name: "preserve")
        _ = ConversationLibrary(repository: repository)
        let corrupt = repository.chatsDirectory.appendingPathComponent("\(UUID()).json")
        try Data("broken".utf8).write(to: corrupt)
        try repository.removeUnusedAttachments([attachment.id])
        XCTAssertNoThrow(try repository.imageStore.data(for: attachment))
    }

    func testWriteFailureDoesNotDeleteUnsavedMessages() throws {
        let blocked = repository.root.appendingPathComponent("blocked")
        try FileManager.default.createDirectory(at: repository.root, withIntermediateDirectories: true)
        try Data().write(to: blocked)
        let library = ConversationLibrary(repository: ConversationRepository(root: blocked))
        library.edit { $0.messages = [ChatMessage(role: .user, content: "Keep me")] }
        XCTAssertFalse(library.flush())
        XCTAssertNotNil(library.error)
        library.delete(library.selectedID)
        XCTAssertEqual(library.current.messages.first?.content, "Keep me")
    }
}
