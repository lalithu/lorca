import Foundation
import Combine

struct RosterWork: Codable, Identifiable {
    let id: String
    let objective: String
    let status: String
    let conversationId: UUID?
    let runId: String?
    let model: String?
    let summary: String?
    let updatedAt: Double

    var title: String { String((objective.split(separator: "\n").first.map(String.init) ?? objective).prefix(100)) }
    var needsYou: Bool { ["awaiting_approval", "waiting_for_input"].contains(status) }
    var active: Bool { ["running", "awaiting_approval"].contains(status) }
    var label: String {
        switch status {
        case "queued": return "Queued"
        case "running": return "Working"
        case "awaiting_approval": return "Approval needed"
        case "waiting_for_input": return "Needs you"
        case "completed": return "Finished"
        case "cancelled": return "Cancelled"
        case "failed": return "Failed"
        default: return "Interrupted"
        }
    }
}
struct RosterSnapshot: Codable {
    var tasks: [RosterWork] = []
    var error: String?
}

@MainActor final class RosterStore: ObservableObject {
    @Published private(set) var state = RosterSnapshot()
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    var receive: ((BackendSnapshot) -> Void)?
    private var pendingAdd: (objective: String, id: String)?
    private let backend: BackendTransport
    init(backend: BackendTransport) { self.backend = backend }
    func apply(_ state: RosterSnapshot?) { self.state = state ?? RosterSnapshot() }
    @discardableResult func add(_ objective: String) async -> Bool {
        guard !busy else { return false }
        if pendingAdd?.objective != objective { pendingAdd = (objective, UUID().uuidString) }
        let saved = await command(["op": "roster_add", "taskId": pendingAdd!.id, "objective": objective])
        if saved { pendingAdd = nil }
        return saved
    }
    func run(_ task: RosterWork, model: String, effort: String, context: Int?) async -> UUID? {
        var fields: [String: Any] = ["op": "roster_run", "taskId": task.id, "model": model, "effort": effort]
        if let context { fields["contextWindow"] = context; fields["configureContext"] = true }
        guard await command(fields) else { return nil }
        return state.tasks.first { $0.id == task.id }?.conversationId
    }
    private func command(_ fields: [String: Any]) async -> Bool {
        guard !busy else { return false }
        busy = true
        defer { busy = false }
        do {
            let data = try await backend.command(fields)
            let snapshot = try JSONDecoder().decode(BackendSnapshot.self, from: data)
            receive?(snapshot)
            error = nil
            return true
        } catch {
            self.error = friendlyError(error)
            return false
        }
    }
}

@MainActor extension ChatStore {
    func openRosterConversation(_ id: UUID) {
        // Ordinary chat navigation stops work; opening its roster result must not.
        enqueue(["op": "open", "conversationId": id.uuidString])
    }
}
