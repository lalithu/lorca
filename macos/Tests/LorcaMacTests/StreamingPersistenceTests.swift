import XCTest
@testable import LorcaMac

private final class SlowReply: URLProtocol {
    private var completion: DispatchWorkItem?
    override class func canInit(with request: URLRequest) -> Bool { request.url?.path == "/api/chat" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil,
            headerFields: ["Content-Type": "application/x-ndjson"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data("{\"message\":{\"content\":\"Saved partial\"},\"done\":false}\n".utf8))
        let completion = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.client?.urlProtocol(self, didLoad: Data("{\"message\":{\"content\":\" finished\"},\"done\":true}\n".utf8))
            self.client?.urlProtocolDidFinishLoading(self)
        }
        self.completion = completion
        DispatchQueue.global().asyncAfter(deadline: .now() + 1.5, execute: completion)
    }
    override func stopLoading() { completion?.cancel() }
}

@MainActor
final class StreamingPersistenceTests: XCTestCase {
    func testSwitchAndStopPreservePartialReplyWithoutLeakingIntoOtherChat() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let repository = ConversationRepository(root: root)
        let defaults = UserDefaults(suiteName: UUID().uuidString)!
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SlowReply.self, MockOllama.self]
        let transport = URLSession(configuration: config)
        defer { transport.invalidateAndCancel(); try? FileManager.default.removeItem(at: root) }
        MockOllama.manage = { _ in (200, #"{"done":true}"#) }
        let store = ChatStore(client: OllamaClient(session: transport), defaults: defaults, repository: repository)
        let first = store.library.selectedID
        store.draft = "Start a response"
        store.send()
        for _ in 0..<100 {
            if store.messages.last?.content == "Saved partial" { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(store.isGenerating)
        XCTAssertEqual(store.messages.last?.content, "Saved partial")
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(try repository.load().conversations.first?.messages.last?.content, "Saved partial")
        store.newChat()
        XCTAssertFalse(store.isGenerating)
        XCTAssertTrue(store.messages.isEmpty)
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertTrue(store.messages.isEmpty)
        let restored = ChatStore(client: OllamaClient(session: transport), defaults: defaults, repository: repository)
        restored.openConversation(first)
        XCTAssertEqual(restored.messages.map(\.content), ["Start a response", "Saved partial"])
        XCTAssertFalse(restored.isGenerating)
    }
}
