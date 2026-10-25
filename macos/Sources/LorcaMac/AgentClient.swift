import Foundation

@MainActor
struct AgentClient {
    let backend: BackendTransport
    func snapshot() async throws -> AgentTaskSnapshot? {
        try JSONDecoder().decode(AgentTaskSnapshot?.self, from: await backend.command(["op": "agent_snapshot"]))
    }
    func start(task: String, workspace: URL?, conversation: UUID, request: UUID, model: String) async throws -> String {
        let data = try await backend.command(["op": "agent_start", "task": task, "workspace": workspace?.path ?? "",
            "conversationId": conversation.uuidString, "requestId": request.uuidString, "model": model,
            "limits": ["iterations": 24, "modelSeconds": 120, "commandSeconds": 120]])
        struct Started: Decodable { let taskId: String }
        return try JSONDecoder().decode(Started.self, from: data).taskId
    }
    func decide(task: String, approval: String, decision: String) async throws {
        _ = try await backend.command(["op": "agent_decide", "taskId": task, "approvalId": approval, "decision": decision])
    }
    func cancel(task: String) async throws { _ = try await backend.command(["op": "agent_cancel", "taskId": task]) }
}
