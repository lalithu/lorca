import XCTest
@testable import LorcaMac

final class MockOllama: URLProtocol {
    static var respond: (URLRequest) throws -> (Int, String) = { _ in (200, "") }
    static var manage: (URLRequest) throws -> (Int, String) = { _ in (200, #"{"done":true,"done_reason":"unload"}"#) }
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, body) = try request.url?.path == "/api/generate" ? Self.manage(request) : Self.respond(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil,
                headerFields: ["Content-Type": "application/x-ndjson"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data(body.utf8))
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}

    static func payload(_ request: URLRequest) throws -> [String: Any] {
        var data = request.httpBody ?? Data()
        if data.isEmpty, let stream = request.httpBodyStream {
            stream.open()
            defer { stream.close() }
            var buffer = [UInt8](repeating: 0, count: 4096)
            while stream.hasBytesAvailable {
                let count = stream.read(&buffer, maxLength: buffer.count)
                if count <= 0 { break }
                data.append(buffer, count: count)
            }
        }
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }
}

@MainActor
final class ChatTests: XCTestCase {
    private var session: URLSession!
    private var client: OllamaClient!
    private var defaults: UserDefaults!
    private var repository: ConversationRepository!
    private let suite = "dev.lorca.macos.tests"

    override func setUp() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockOllama.self]
        session = URLSession(configuration: config)
        client = OllamaClient(session: session)
        repository = ConversationRepository(root: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
        defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        MockOllama.manage = { _ in (200, #"{"done":true,"done_reason":"unload"}"#) }
    }

    override func tearDown() async throws {
        session.invalidateAndCancel()
        try? FileManager.default.removeItem(at: repository.root)
        defaults.removePersistentDomain(forName: suite)
    }

    func testInstalledModelsAndFallback() async {
        MockOllama.respond = { request in
            if request.url?.path == "/api/ps" {
                return (200, #"{"models":[{"name":"other:latest","context_length":4096}]}"#)
            }
            XCTAssertEqual(request.url?.path, "/api/tags")
            return (200, #"{"models":[{"name":"small:latest"},{"name":"other:latest"}]}"#)
        }
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        await store.refreshConnection()
        XCTAssertTrue(store.isConnected)
        XCTAssertEqual(store.models, ["other:latest", "small:latest"])
        XCTAssertEqual(store.selectedModel, "other:latest")
        XCTAssertEqual(store.usage.contextLimit, 4096)
    }

    func testStreamAndConversationHistory() async throws {
        var requests = 0
        MockOllama.respond = { request in
            if request.url?.path == "/api/ps" {
                return (200, #"{"models":[{"name":"unrelated:latest","context_length":32768},{"name":"qwen3.5:9b","context_length":4096}]}"#)
            }
            requests += 1
            XCTAssertEqual(request.httpMethod, "POST")
            let payload = try MockOllama.payload(request)
            XCTAssertEqual(payload["model"] as? String, "qwen3.5:9b")
            XCTAssertEqual(payload["stream"] as? Bool, true)
            XCTAssertEqual(payload["keep_alive"] as? Int, -1)
            XCTAssertEqual((payload["messages"] as? [[String: String]])?.count, requests == 1 ? 1 : 3)
            return (200, "{\"message\":{\"content\":\"Hello\"},\"done\":false}\n{\"message\":{\"content\":\" there\"},\"done\":true,\"prompt_eval_count\":64,\"eval_count\":8,\"eval_duration\":400000000}\n")
        }
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        store.draft = "Hi"
        store.send()
        try await waitForReply(store)
        XCTAssertEqual(store.messages.map(\.content), ["Hi", "Hello there"])
        store.draft = "Again"
        store.send()
        try await waitForReply(store)
        XCTAssertEqual(store.messages.map(\.content), ["Hi", "Hello there", "Again", "Hello there"])
        XCTAssertEqual(requests, 2)
        XCTAssertNil(store.error)
        XCTAssertEqual(store.usage.contextUsed, 72)
        XCTAssertEqual(store.usage.contextLimit, 4096)
        XCTAssertEqual(store.usage.tokensPerSecond, 20)
        store.newChat()
        XCTAssertEqual(store.usage.contextUsed, 0)
        XCTAssertEqual(store.usage.contextLimit, 4096)
        XCTAssertNil(store.usage.tokensPerSecond)
    }

    func testUnavailableServerAndServerError() async throws {
        MockOllama.respond = { _ in throw URLError(.cannotConnectToHost) }
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        await store.refreshConnection()
        XCTAssertFalse(store.isConnected)
        XCTAssertTrue(store.error?.contains("ollama serve") == true)
        MockOllama.respond = { _ in (404, #"{"error":"model not found"}"#) }
        store.draft = "Hello"
        store.send()
        try await waitForReply(store)
        XCTAssertEqual(store.error, "model not found")
        XCTAssertEqual(store.messages.count, 1)
        XCTAssertFalse(store.isGenerating)
    }

    func testStopAndNewChatIgnoreLateResponse() async throws {
        MockOllama.respond = { _ in
            (200, "{\"message\":{\"content\":\"Late reply\"},\"done\":true}\n")
        }
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        store.draft = "Hello"
        store.send()
        store.stop()
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertFalse(store.isGenerating)
        XCTAssertEqual(store.messages.map(\.content), ["Hello"])
        store.newChat()
        XCTAssertTrue(store.messages.isEmpty)
        XCTAssertEqual(store.library.conversations.count, 2)
    }

    func testMissingUsageAndFailedMetadataDoNotFailReply() async throws {
        MockOllama.respond = { request in
            if request.url?.path == "/api/ps" { throw URLError(.timedOut) }
            return (200, "{\"message\":{\"content\":\"Hello\"},\"done\":true}\n")
        }
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        store.draft = "Hi"
        store.send()
        try await waitForReply(store)
        XCTAssertEqual(store.messages.last?.content, "Hello")
        XCTAssertNil(store.error)
        XCTAssertNil(store.usage.contextUsed)
        XCTAssertNil(store.usage.contextLimit)
        XCTAssertNil(store.usage.fraction)
        XCTAssertNil(store.usage.tokensPerSecond)
    }

    func testSwitchingModelClearsPreviousUsage() async throws {
        MockOllama.respond = { request in
            if request.url?.path == "/api/ps" {
                return (200, #"{"models":[{"name":"qwen3.5:9b","context_length":4096}]}"#)
            }
            return (200, "{\"message\":{\"content\":\"Hi\"},\"done\":true,\"prompt_eval_count\":10,\"eval_count\":2,\"eval_duration\":0}\n")
        }
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        store.draft = "Hello"
        store.send()
        try await waitForReply(store)
        XCTAssertEqual(store.usage.contextUsed, 12)
        XCTAssertNil(store.usage.tokensPerSecond)
        store.selectedModel = "other:latest"
        _ = await store.modelSession.prepare("other:latest").value
        XCTAssertNil(store.usage.contextUsed)
        XCTAssertNil(store.usage.contextLimit)
        XCTAssertNil(store.usage.fraction)
    }

    func testImagesAreEncodedInHistoryAndRestoredWithTheirFiles() async throws {
        var requests = 0
        MockOllama.respond = { request in
            if request.url?.path == "/api/show" { return (200, #"{"capabilities":["completion","vision"]}"#) }
            if request.url?.path == "/api/ps" { return (200, #"{"models":[]}"#) }
            requests += 1
            let payload = try MockOllama.payload(request)
            let messages = try XCTUnwrap(payload["messages"] as? [[String: Any]])
            let images = try XCTUnwrap(messages[0]["images"] as? [String])
            XCTAssertEqual(images.count, 2)
            XCTAssertTrue(images.allSatisfy { Data(base64Encoded: $0)?.starts(with: [0xff, 0xd8]) == true })
            XCTAssertEqual(messages.count, requests == 1 ? 1 : 3)
            if messages.count > 1 { XCTAssertNil(messages[1]["images"]) }
            return (200, "{\"message\":{\"content\":\"A red image\"},\"done\":true}\n")
        }
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        let first = try repository.imageStore.importData(imageFixture(), name: "first.png")
        let second = try repository.imageStore.importData(imageFixture(), name: "second.png")
        store.library.addImages([first, second], to: store.library.selectedID)
        store.draft = "Describe these"
        store.send()
        try await waitForReply(store)
        let restored = ChatStore(client: client, defaults: defaults, repository: repository)
        XCTAssertEqual(restored.messages[0].attachments, [first, second])
        XCTAssertNoThrow(try restored.imageStore.data(for: first))
        restored.draft = "What color?"
        restored.send()
        try await waitForReply(restored)
        XCTAssertEqual(requests, 2)
        XCTAssertNil(restored.error)
    }

    func testTextOnlyModelsKeepImagesButDoNotSendAnUnsupportedRequest() async throws {
        var chatRequests = 0
        MockOllama.respond = { request in
            if request.url?.path == "/api/chat" { chatRequests += 1 }
            return (200, #"{"capabilities":["completion"]}"#)
        }
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        let image = try repository.imageStore.importData(imageFixture(), name: "photo")
        store.library.addImages([image], to: store.library.selectedID)
        store.send()
        try await waitForReply(store)
        XCTAssertEqual(chatRequests, 0)
        XCTAssertTrue(store.error?.contains("doesn’t support images") == true)
        XCTAssertEqual(store.messages.count, 1)
        XCTAssertEqual(store.messages[0].attachments, [image])
        XCTAssertNoThrow(try store.imageStore.data(for: image))
    }

    func testImageCountLimitAndRemovingDraftImage() async throws {
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        let image = try repository.imageStore.importData(imageFixture(), name: "photo")
        store.library.addImages([image], to: store.library.selectedID)
        store.attachFiles(Array(repeating: URL(fileURLWithPath: "/missing.png"), count: 4))
        XCTAssertTrue(store.error?.contains("four images") == true)
        XCTAssertFalse(store.isImportingImages)
        store.library.removeDraftImage(image.id)
        XCTAssertTrue(store.draftImages.isEmpty)
        XCTAssertThrowsError(try store.imageStore.data(for: image))
    }

    func testSwitchingChatsDoesNotReloadTheModel() async throws {
        var pins = 0
        MockOllama.manage = { _ in pins += 1; return (200, #"{"done":true}"#) }
        let store = ChatStore(client: client, defaults: defaults, repository: repository)
        _ = await store.modelSession.prepare(store.selectedModel).value
        let first = store.library.selectedID
        store.newChat()
        store.openConversation(first)
        XCTAssertEqual(pins, 1)
        XCTAssertEqual(store.modelSession.activeModel, store.selectedModel)
    }

    func testImagePayloadLimitCountsEveryImageSentAcrossHistory() async throws {
        let image = ChatAttachment(id: UUID(), name: "repeated", width: 10, height: 10)
        let message = ChatMessage(role: .user, content: "Images", attachments: Array(repeating: image, count: 4))
        do {
            _ = try await client.reply(model: "orca", messages: [message],
                imageData: [image.id: Data(count: AttachmentStore.maxStoredBytes)]) { _ in }
            XCTFail("An oversized payload must fail before being sent")
        } catch ImageError.payloadTooLarge {
            // Reusing one stored file still counts once for every wire occurrence.
        }
    }

    private func waitForReply(_ store: ChatStore) async throws {
        for _ in 0..<200 {
            if !store.isGenerating { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Generation did not finish")
        store.stop()
    }
}
