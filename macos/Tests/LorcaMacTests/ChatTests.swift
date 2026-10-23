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
    private let suite = "dev.lorca.macos.tests"

    override func setUp() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockOllama.self]
        session = URLSession(configuration: config)
        client = OllamaClient(session: session)
        defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        MockOllama.manage = { _ in (200, #"{"done":true,"done_reason":"unload"}"#) }
    }

    override func tearDown() async throws {
        session.invalidateAndCancel()
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
        let store = ChatStore(client: client, defaults: defaults)
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
        let store = ChatStore(client: client, defaults: defaults)
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
        let store = ChatStore(client: client, defaults: defaults)
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
        let store = ChatStore(client: client, defaults: defaults)
        store.draft = "Hello"
        store.send()
        store.stop()
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertFalse(store.isGenerating)
        XCTAssertEqual(store.messages.map(\.content), ["Hello"])
        store.newChat()
        XCTAssertTrue(store.messages.isEmpty)
    }

    func testMissingUsageAndFailedMetadataDoNotFailReply() async throws {
        MockOllama.respond = { request in
            if request.url?.path == "/api/ps" { throw URLError(.timedOut) }
            return (200, "{\"message\":{\"content\":\"Hello\"},\"done\":true}\n")
        }
        let store = ChatStore(client: client, defaults: defaults)
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
        let store = ChatStore(client: client, defaults: defaults)
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

    private func waitForReply(_ store: ChatStore) async throws {
        for _ in 0..<200 {
            if !store.isGenerating { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Generation did not finish")
        store.stop()
    }
}
