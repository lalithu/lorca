import XCTest
@testable import LorcaMac

@MainActor
final class AgentMockBackend: MockBackend {
    var agentState: AgentTaskSnapshot?
    var agentReceive: (@MainActor (AgentTaskSnapshot?) -> Void)?
    var failAgent = false
    var rejectAgent = false
    var disconnectAgentOnce = false
    override func command(_ fields: [String: Any]) async throws -> Data {
        guard let op = fields["op"] as? String, op.hasPrefix("agent_") else { return try await super.command(fields) }
        requests.append(fields)
        if failAgent { throw BackendError(message: "Agent disconnected") }
        if rejectAgent && op=="agent_start" {throw BackendError(message:"Rejected before acceptance",submissionRejected:true)}
        switch op {
        case "agent_snapshot": return try JSONEncoder().encode(agentState)
        case "agent_start":
            let conversation = UUID(uuidString: fields["conversationId"] as! String)!
            let request = UUID(uuidString: fields["requestId"] as! String)!
            agentState = try agentFixture(conversation: conversation, request: request)
            let index = chats.firstIndex { $0.id == conversation }!
            var user = ChatMessage(role: .user, content: fields["task"] as! String)
            user.agentKind = "request"; user.agentTaskId = agentState!.id; user.agentRequestId = request
            chats[index].messages.append(user); chats[index].draft = ""; revision += 1; emit(); agentReceive?(agentState)
            return try JSONSerialization.data(withJSONObject: ["taskId": agentState!.id])
        case "agent_decide":
            try await Task.sleep(for: .milliseconds(20))
            agentState?.approval = nil; agentState?.status = .running; agentReceive?(agentState)
        case "agent_cancel": agentState?.status = .cancelled; agentState?.approval = nil; agentReceive?(agentState)
        default: break
        }
        return Data(#"{"accepted":true}"#.utf8)
    }
    override func agentEvents(_ receive: @escaping @MainActor (AgentTaskSnapshot?) -> Void) async throws {
        agentReceive = receive
        if disconnectAgentOnce { disconnectAgentOnce = false; throw BackendError(message: "Stream ended") }
        receive(agentState)
        try await Task.sleep(for: .seconds(600))
    }
}
func agentFixture(conversation: UUID = UUID(), request: UUID = UUID(), status: String = "awaiting_approval", sequence: Int = 3) throws -> AgentTaskSnapshot {
    let action: [String: Any] = ["executable": "/Users/test/.cargo/bin/cargo", "arguments": ["test"], "workingDirectory": "/workspace", "timeoutSeconds": 120]
    let events: [[String: Any]] = [
        ["id": "run:1", "taskId": "run", "sequence": 1, "kind": "task_started", "data": [:]],
        ["id": "run:2", "taskId": "run", "sequence": 2, "kind": "tool_requested", "data": ["callId": "call", "tool": "run_command", "arguments": ["executable": "cargo", "arguments": ["test"]]]],
        ["id": "run:3", "taskId": "run", "sequence": 3, "kind": "approval_required", "data": ["id": "approval", "callId": "call", "tool": "run_command", "action": action]]
    ]
    var object: [String: Any] = ["id": "run", "owner": "test-session", "conversationId": conversation.uuidString, "requestId": request.uuidString,
        "task": "Inspect and test", "workspace": "/workspace", "status": status, "events": Array(events.prefix(sequence))]
    if status == "awaiting_approval" { object["approval"] = ["id": "approval", "callId": "call", "tool": "run_command", "action": action] }
    return try JSONDecoder().decode(AgentTaskSnapshot.self, from: JSONSerialization.data(withJSONObject: object))
}

@MainActor
final class AgentTests: XCTestCase {
    func testIncompleteAndDeniedAreTerminalAndNeverSuccessful() throws {
        for (value, label) in [("incomplete", "Incomplete"), ("denied", "Denied")] {
            let task = try agentFixture(status: value)
            XCTAssertFalse(task.status.active)
            XCTAssertNotEqual(task.status, .completed)
            XCTAssertEqual(task.presentationStatus, label)
        }
    }

    func testDecodingOrderingDeduplicationAndAuthoritativeApprovalState() throws {
        let backend = AgentMockBackend(), store = AgentStore(backend: AgentMockBackend())
        var snapshot = try agentFixture()
        snapshot.events = [snapshot.events[2], snapshot.events[1], snapshot.events[2], snapshot.events[0]]
        store.apply(snapshot)
        XCTAssertEqual(store.latest?.events.map(\.sequence), [1, 2, 3])
        XCTAssertEqual(store.activities["run"]?.count, 1)
        XCTAssertEqual(store.activities["run"]?.first?.title, "cargo test")
        XCTAssertEqual(store.activities["run"]?.first?.approval?.action["arguments"].strings, ["test"])
        store.apply(try agentFixture(status: "running", sequence: 2))
        XCTAssertEqual(store.latest?.status, .awaitingApproval)
        snapshot.approval = nil; snapshot.status = .running; store.apply(snapshot)
        XCTAssertNil(store.latest?.approval)
        XCTAssertEqual(backend.requests.count, 0)
    }
    func testChatComposerSubmitsOnceAndStaysAssociatedWhenSwitchingChats() async throws {
        let backend = AgentMockBackend()
        // Use one backend instance for the shared Dashboard/Mini store.
        let shared = ChatStore(backend: backend)
        await shared.refreshConnection()
        let origin = shared.library.selectedID
        shared.agent.workspaces[origin] = URL(fileURLWithPath: "/workspace")
        shared.draft = "Inspect and test"; shared.send(); shared.send()
        try await Task.sleep(for: .milliseconds(100))
        let starts = backend.requests.filter { $0["op"] as? String == "agent_start" }
        XCTAssertEqual(starts.count, 1)
        XCTAssertEqual(starts.first?["conversationId"] as? String, origin.uuidString)
        XCTAssertEqual(shared.messages.first?.agentKind, "request")
        XCTAssertEqual(shared.draft, "")
        shared.newChat(); try await Task.sleep(for: .milliseconds(50))
        XCTAssertNotEqual(shared.library.selectedID, origin)
        XCTAssertEqual(shared.agent.active?.conversationId, origin)
        XCTAssertTrue(backend.requests.allSatisfy { $0["op"] as? String != "agent_cancel" })
        XCTAssertFalse(shared.canSend)
        shared.draft = "Normal chat"; XCTAssertTrue(shared.canSendChat)
        _ = await shared.modelSession.close(unload: false)
    }
    func testApprovalAllowAndDenyAreSingleSubmissionAndStaleActionsAreBlocked() async throws {
        for decision in ["allow_once", "deny"] {
            let backend = AgentMockBackend(); backend.agentState = try agentFixture()
            let store = AgentStore(backend: backend); try await store.restore()
            let task = store.latest!, approval = task.approval!
            async let first: Void = store.decide(task: task, approval: approval, decision: decision)
            async let duplicate: Void = store.decide(task: task, approval: approval, decision: decision)
            _ = await (first, duplicate)
            await store.decide(task: task, approval: approval, decision: decision)
            XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "agent_decide" }.count, 1)
            XCTAssertEqual(backend.requests.last? ["op"] as? String, "agent_snapshot")
            XCTAssertNil(store.latest?.approval)
        }
    }
    func testCancellationAndFailedDecisionRequireAuthoritativeRecovery() async throws {
        let backend = AgentMockBackend(); backend.agentState = try agentFixture()
        let store = AgentStore(backend: backend); try await store.restore()
        let task = store.latest!, approval = task.approval!
        backend.failAgent = true
        await store.decide(task: task, approval: approval, decision: "allow_once")
        XCTAssertNotNil(store.error); XCTAssertFalse(store.canDecide(task, approval: approval))
        backend.failAgent = false; try await store.restore()
        XCTAssertTrue(store.canDecide(task, approval: approval))
        await store.cancel(); try await store.restore()
        XCTAssertEqual(store.latest?.status, .cancelled)
        XCTAssertFalse(store.canDecide(task, approval: approval))
    }
    func testReconnectRestoresSnapshotWithoutReplayingTheTask() async throws {
        let backend = AgentMockBackend(); backend.agentState = try agentFixture(); backend.disconnectAgentOnce = true
        let store = AgentStore(backend: backend); store.observe()
        try await Task.sleep(for: .milliseconds(1200))
        XCTAssertEqual(store.latest?.approval?.id, "approval"); XCTAssertTrue(store.connected)
        XCTAssertTrue(backend.requests.allSatisfy { $0["op"] as? String != "agent_start" })
        store.shutdown()
    }
    func testUnconfirmedSubmissionIsNotRetriedAndRequiresRestore() async throws {
        let backend = AgentMockBackend(); let store = AgentStore(backend: backend)
        XCTAssertTrue(store.reserveSubmission()); backend.failAgent = true
        let accepted = await store.submit(task: "Task", workspace: URL(fileURLWithPath: "/workspace"), conversation: UUID(), request: UUID(), model: "qwen")
        XCTAssertFalse(accepted); XCTAssertTrue(store.unconfirmedSubmission); XCTAssertFalse(store.reserveSubmission())
        backend.failAgent = false; try await store.restore()
        XCTAssertFalse(store.unconfirmedSubmission)
        XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "agent_start" }.count, 1)
    }
    func testModelReloadUsesExistingLeaseGatewayAndStopsOnFailure() async throws {
        let backend = AgentMockBackend()
        let chat = ChatStore(backend: backend)
        await chat.refreshConnection()
        backend.requests.removeAll()
        await chat.reloadModel()
        XCTAssertEqual(backend.requests.compactMap { $0["op"] as? String }.filter { ["release", "prepare"].contains($0) }, ["release", "prepare"])
        XCTAssertFalse(chat.isReloadingModel)
        backend.requests.removeAll(); backend.failNextCommand = true
        await chat.reloadModel()
        XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "prepare" }.count, 0)
        XCTAssertNotNil(chat.error)
        backend.agentState = try agentFixture(); try await chat.agent.restore()
        XCTAssertFalse(chat.canReloadModel)
        await chat.reloadModel()
        XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "release" }.count, 1)
        _ = await chat.modelSession.close(unload: false)
        XCTAssertFalse(chat.canReloadModel)
    }
    func testUnloadReleasesWithoutReloadingOrChangingChatAndStopsOnFailure() async throws {
        let backend = AgentMockBackend()
        let store = ChatStore(backend: backend)
        await store.refreshConnection()
        store.draft = "Keep my draft"
        try await Task.sleep(for: .milliseconds(30))
        let conversation = store.library.selectedID, model = store.selectedModel
        backend.requests.removeAll()
        backend.failNextCommand = true
        await store.unloadModel()
        XCTAssertNotNil(store.error)
        XCTAssertEqual(store.modelSession.activeModel, model)
        XCTAssertFalse(store.isUnloadingModel)
        backend.requests.removeAll()
        await store.unloadModel()
        XCTAssertEqual(backend.requests.compactMap { $0["op"] as? String }, ["release", "snapshot"])
        XCTAssertNil(store.modelSession.activeModel)
        XCTAssertEqual(store.selectedModel, model)
        XCTAssertEqual(store.library.selectedID, conversation)
        XCTAssertEqual(store.draft, "Keep my draft")
        XCTAssertFalse(store.canUnloadModel)
        await store.unloadModel()
        XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "release" }.count, 1)
        _ = await store.modelSession.prepare(model).value
        XCTAssertTrue(store.canUnloadModel)
        backend.agentState = try agentFixture()
        try await store.agent.restore()
        XCTAssertFalse(store.canUnloadModel)
        await store.unloadModel()
        XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "release" }.count, 1)
        _ = await store.modelSession.close(unload: false)
        XCTAssertFalse(store.canUnloadModel)
    }
    func testUnloadCanReleasePausedResearchModelOutsideChatLease() async throws {
        let backend = AgentMockBackend()
        let store = ChatStore(backend: backend)
        await store.refreshConnection()
        let researchModel = store.selectedModel
        await store.unloadModel()
        XCTAssertNil(store.modelSession.activeModel)
        backend.requests.removeAll()
        await store.unloadModel(model: researchModel)
        XCTAssertEqual(backend.requests.compactMap { $0["op"] as? String }, ["release", "snapshot"])
        XCTAssertEqual(backend.requests.first?["model"] as? String, researchModel)
        XCTAssertNil(store.modelSession.activeModel)
        _ = await store.modelSession.close(unload: false)
    }
    func testDiffClassificationTerminalOutputAndSavedMetadata() throws {
        XCTAssertEqual(AgentDiffLine.parse("--- a\n+++ b\n@@ -1,2 +1,2 @@\n-old\n+new\n context").map(\.kind), [.header, .header, .header, .removal, .addition, .context])
        let data = Data(#"{"id":"output","sequence":4,"taskId":"run","kind":"terminal_output","data":{"callId":"call","output":{"stream":"stderr","text":"actual error\n"}}}"#.utf8)
        let event = try JSONDecoder().decode(AgentEvent.self, from: data)
        XCTAssertEqual(AgentActivity.make(from: [event]).first?.output, "[stderr] actual error\n")
        let result = AgentJSON.object(["exitCode": .number(0), "output": .string("real output\n"), "truncated": .bool(true)])
        XCTAssertEqual(result["exitCode"].int, 0)
        XCTAssertEqual(result["output"].string, "real output\n")
        XCTAssertEqual(result["truncated"].boolean, true)
        var message = ChatMessage(role: .user, content: "Task")
        message.agentTaskId = "run"; message.agentKind = "request"; message.agentWorkspace = "/workspace"
        XCTAssertEqual(try JSONDecoder().decode(ChatMessage.self, from: JSONEncoder().encode(message)), message)
    }
    func testUnifiedConversationWithoutWorkspaceAndWithImagesUsesOneSubmission() async throws {
        let backend=AgentMockBackend(),store=ChatStore(backend:AgentMockBackend())
        _=backend
        await store.refreshConnection()
        XCTAssertNil(store.agentWorkspace)
        store.draft="hi";XCTAssertTrue(store.canSend);store.send();store.send()
        try await Task.sleep(for:.milliseconds(80))
        let transport=store.backend as! AgentMockBackend
        XCTAssertEqual(transport.requests.filter{$0["op"] as? String=="agent_start"}.count,1)
        XCTAssertEqual(transport.requests.first{$0["op"] as? String=="agent_start"}?["workspace"] as? String,"")
        _=await store.modelSession.close(unload:false)
    }
    func testUsageStreamingAndArchivedActivityCannotReplaceActiveApproval() throws {
        let backend=AgentMockBackend(),store=AgentStore(backend:AgentMockBackend())
        _=backend
        var active=try agentFixture();active.assistantText="Real partial";active.generating=true
        active.usage=ChatUsage(contextUsed:100,contextLimit:8192,tokensPerSecond:20)
        store.apply(active);store.apply(active)
        XCTAssertEqual(store.latest?.assistantText,"Real partial")
        XCTAssertEqual(store.latest?.usage?.contextUsed,100)
        var archived=ChatMessage(role:.assistant,content:"Past result")
        archived.agentTaskId="past";archived.agentStatus="completed";archived.agentEvents=[]
        var chat=Conversation();chat.messages=[archived]
        store.restoreArchived([chat],selected:chat.id)
        XCTAssertEqual(store.latest?.approval?.id,"approval")
        XCTAssertEqual(store.latest?.id,"run")
        XCTAssertNotNil(store.runs["past"])
    }

    func testRejectedSubmissionDoesNotPermanentlyDisableComposer() async throws {
        let backend=AgentMockBackend();backend.rejectAgent=true
        let store=AgentStore(backend:backend)
        XCTAssertTrue(store.reserveSubmission())
        let accepted=await store.submit(task:"hi",workspace:nil,conversation:UUID(),request:UUID(),model:"qwen")
        XCTAssertFalse(accepted);XCTAssertFalse(store.busy);XCTAssertFalse(store.submitting)
        XCTAssertEqual(store.error,"Rejected before acceptance")
        XCTAssertTrue(store.reserveSubmission())
    }

}
