import XCTest
@testable import LorcaMac

@MainActor
class MockBackend: BackendTransport {
    var sessionID: String? = "test-session"
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    var requests: [[String: Any]] = []
    var chats = [Conversation()]
    var selected: UUID
    var model = "qwen3.5:9b"
    var activeModel: String? = "qwen3.5:9b"
    var fail = false
    var breakNextStream = false
    var failNextCommand = false
    var connects = 0
    var conversationTask: AgentTaskSnapshot?
    var turn: [String: Any]?
    var receive: (@MainActor (BackendSnapshot) -> Void)?
    var revision = 1
    init() { selected = chats[0].id }
    func snapshot() throws -> BackendSnapshot {
        let encoded = try JSONEncoder().encode(chats)
        return try JSONDecoder().decode(BackendSnapshot.self, from: JSONSerialization.data(withJSONObject: [
            "revision": revision, "conversations": JSONSerialization.jsonObject(with: encoded),
            "sessions": [["id": sessionID!, "selectedId": selected.uuidString, "model": model, "activeModel": activeModel as Any? ?? NSNull(),
                          "modelsToRelease": activeModel.map { [$0] } ?? [], "models": [model, "other"], "isConnected": true,
                          "usage": ["contextUsed": 0, "contextLimit": 4096]]], "turns": turn.map { [$0] } ?? []
        ]))
    }
    func connect(model: String) async throws -> BackendSnapshot { connects += 1; self.model = model; activeModel = model; return try snapshot() }
    func command(_ fields: [String: Any]) async throws -> Data {
        requests.append(fields)
        if failNextCommand { failNextCommand = false; throw BackendError(message: "Backend restarted") }
        if fail { throw BackendError(message: "Backend disconnected") }
        revision += 1
        let index = chats.firstIndex { $0.id == selected }!
        switch fields["op"] as? String {
        case "draft": chats[index].draft = fields["text"] as! String
        case "prepare": model = fields["model"] as! String; activeModel = model
        case "selectModel": model = fields["model"] as! String
        case "release":
            activeModel = nil; emit()
            return Data(#"{"unloaded":true}"#.utf8)
        case "new": let chat = Conversation(); chats.append(chat); selected = chat.id
        case "open": selected = UUID(uuidString: fields["conversationId"] as! String)!
        case "send":
            chats[index].messages.append(ChatMessage(role: .user, content: chats[index].draft))
            chats[index].messages.append(ChatMessage(role: .assistant, content: ""))
            chats[index].draft = ""
            turn = ["id": fields["requestId"]!, "conversationId": selected.uuidString, "sessionId": sessionID!, "model": model, "running": true, "usage": [:]]
            emit()
            return try JSONSerialization.data(withJSONObject: turn!)
        case "agent_start":
            conversationTask=try agentFixture(conversation:selected,request:UUID(uuidString:fields["requestId"] as! String)!,status:"running",sequence:1)
            var user=ChatMessage(role:.user,content:fields["task"] as! String,attachments:chats[index].draftImages)
            user.agentTaskId=conversationTask!.id;user.agentKind="request"
            chats[index].messages.append(user)
            var response=ChatMessage(role:.assistant,content:"");response.agentTaskId=conversationTask!.id;response.agentKind="result"
            chats[index].messages.append(response);chats[index].draft="";chats[index].draftImages=[]
            emit()
            return try JSONSerialization.data(withJSONObject:["taskId":conversationTask!.id])
        case "agent_snapshot": return try JSONEncoder().encode(conversationTask)
        case "agent_cancel": conversationTask?.status = .cancelled
        case "cancel": turn?["running"] = false
        case "close": return Data(#"{"failures":[]}"#.utf8)
        default: break
        }
        emit()
        return try JSONEncoder().encode(snapshot())
    }
    func cancelRequests() async {}
    func events(_ receive: @escaping @MainActor (BackendSnapshot) -> Void) async throws {
        self.receive = receive
        if breakNextStream { breakNextStream = false; failNextCommand = true; throw BackendError(message: "Stream broke") }
        try await Task.sleep(for: .seconds(600))
    }
    func agentEvents(_ receive: @escaping @MainActor (AgentTaskSnapshot?) -> Void) async throws {
        receive(conversationTask)
        try await Task.sleep(for: .seconds(600))
    }
    func emit() { if let state = try? snapshot() { receive?(state) } }
}
@MainActor
final class ChatTests: XCTestCase {
    func testStartupRefreshAndSelectionDoNotPrepareAModel() async throws {
        let backend = MockBackend()
        let suite = "lorca.lazy-model-test.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set("qwen3.5:9b", forKey: "model")
        let store = ChatStore(backend: backend, defaults: defaults)
        await store.refreshConnection()
        await store.refreshConnection()
        store.selectedModel = "other"
        for _ in 0..<200 {
            if backend.requests.contains(where: { $0["op"] as? String == "selectModel" }) { break }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertEqual(store.selectedModel, "other")
        XCTAssertTrue(backend.requests.contains { $0["op"] as? String == "selectModel" })
        XCTAssertFalse(backend.requests.contains { $0["op"] as? String == "prepare" })
        store.draft = "Hello"
        store.send()
        for _ in 0..<200 {
            if backend.requests.contains(where: { $0["op"] as? String == "agent_start" }) { break }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertEqual(backend.requests.last(where: { $0["op"] as? String == "agent_start" })?["model"] as? String, "other")
        _ = await store.modelSession.close(unload: false)
    }
    func testDraftIsSentBeforePromptAndNeverReplayedAfterFailure() async throws {
        let backend = MockBackend()
        let store = ChatStore(backend: backend)
        await store.refreshConnection()
        store.draft = "Hello"
        store.send()
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "agent_start" }.count, 1)
        XCTAssertEqual(store.messages.first?.content, "Hello")
        XCTAssertEqual(store.draft, "")
        XCTAssertTrue(store.isGenerating || store.currentAgentActive)
        store.stop()
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertFalse(store.isGenerating)
        _ = await store.modelSession.close(unload: false)
    }
    func testBackendFailureKeepsUnacknowledgedDraftAndReportsError() async throws {
        let backend = MockBackend()
        let store = ChatStore(backend: backend)
        await store.refreshConnection()
        backend.fail = true
        store.draft = "Keep my draft"
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertEqual(store.draft, "Keep my draft")
        XCTAssertEqual(store.error, "Backend disconnected")
        backend.fail = false
        _ = await store.modelSession.close(unload: false)
    }
    func testSharedProjectionSwitchingAndFontZoomDoNotReloadModel() async throws {
        let backend = MockBackend()
        let store = ChatStore(backend: backend)
        await store.refreshConnection()
        let first = store.library.selectedID
        let loads = backend.requests.filter { $0["op"] as? String == "prepare" }.count
        store.newChat()
        try await Task.sleep(for: .milliseconds(30))
        store.openConversation(first)
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertEqual(store.library.selectedID, first)
        XCTAssertEqual(store.library.conversations.count, 2)
        XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "prepare" }.count, loads)
        store.resizeChatText(by: 1)
        XCTAssertEqual(store.chatTextZoom, 1)
        _ = await store.modelSession.close(unload: false)
    }
    func testStreamedSnapshotsUpdateSharedChatWithoutAddingMessagesTwice() async throws {
        let backend = MockBackend()
        let store = ChatStore(backend: backend)
        await store.refreshConnection()
        store.draft = "Hello"
        store.send()
        try await Task.sleep(for: .milliseconds(40))
        backend.chats[0].messages[1].content = "Partial reply"
        backend.revision += 1
        backend.emit()
        backend.emit()
        XCTAssertEqual(store.messages.count, 2)
        XCTAssertEqual(store.messages.last?.content, "Partial reply")
        XCTAssertEqual(store.messages.last?.content,"Partial reply")
        _ = await store.modelSession.close(unload: false)
    }
    func testBackendRestartReconnectsWithoutReplayingPrompt() async throws {
        let backend = MockBackend()
        backend.breakNextStream = true
        let store = ChatStore(backend: backend)
        await store.refreshConnection()
        try await Task.sleep(for: .milliseconds(80))
        XCTAssertGreaterThanOrEqual(backend.connects, 2)
        XCTAssertTrue(store.ready)
        XCTAssertNil(store.error)
        XCTAssertTrue(backend.requests.allSatisfy { $0["op"] as? String != "send" })
        _ = await store.modelSession.close(unload: false)
    }

}
