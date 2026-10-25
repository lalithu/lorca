import Combine
import Foundation

@MainActor
final class AgentStore: ObservableObject {
    @Published private(set) var latest: AgentTaskSnapshot?
    @Published private(set) var runs: [String: AgentTaskSnapshot] = [:]
    @Published private(set) var connected = false
    @Published private(set) var submitting = false
    @Published private(set) var deciding: String?
    @Published private(set) var cancelling = false
    @Published private(set) var unconfirmedSubmission = false
    @Published var error: String?
    @Published var workspaces: [UUID: URL] = [:]
    private(set) var activities: [String: [AgentActivity]] = [:]
    private let client: AgentClient
    private var stream: Task<Void, Never>?
    private var flush: Task<Void, Never>?
    private var buffered: AgentTaskSnapshot?
    private var blockedApprovals = Set<String>()
    private var order: [String] = []
    var active: AgentTaskSnapshot? { latest.flatMap { $0.status.active ? $0 : nil } }
    var busy: Bool { active != nil || submitting || unconfirmedSubmission }
    var sessionID: String? { client.backend.sessionID }

    init(backend: BackendTransport) { client = AgentClient(backend: backend) }
    func observe() {
        guard stream == nil else { return }
        stream = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                do {
                    try await self.restore()
                    try await self.client.backend.agentEvents { [weak self] in self?.buffer($0) }
                } catch {
                    if Task.isCancelled { return }
                    self.connected = false
                    self.error = friendlyError(error)
                    try? await Task.sleep(for: .seconds(1))
                }
            }
        }
    }
    func shutdown() { stream?.cancel(); stream = nil; flush?.cancel(); flush = nil; connected = false }
    func restore() async throws {
        let snapshot = try await client.snapshot()
        flush?.cancel(); flush = nil
        apply(snapshot)
        connected = true; error = nil; unconfirmedSubmission = false
        // Only a fresh authoritative snapshot permits retry after an uncertain network result.
        if let id = snapshot?.approval?.id, deciding == nil { blockedApprovals.remove(id) }
    }
    private func buffer(_ snapshot: AgentTaskSnapshot?) {
        if let buffered, buffered.id != snapshot?.id { apply(buffered) }
        buffered = snapshot
        guard flush == nil else { return }
        flush = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(100))
            guard !Task.isCancelled, let self else { return }
            self.apply(self.buffered); self.connected = true; self.flush = nil
        }
    }
    func apply(_ snapshot: AgentTaskSnapshot?) {
        guard var snapshot else {
            if let id = latest?.id, latest?.status.active == true { runs.removeValue(forKey: id); activities.removeValue(forKey: id) }
            latest = nil
            return
        }
        if let previous=runs[snapshot.id],let before=previous.revision,let after=snapshot.revision,after<before {return}
        var sequences = Set<UInt64>()
        snapshot.events = snapshot.events.filter { $0.taskId == snapshot.id && sequences.insert($0.sequence).inserted }.sorted { $0.sequence < $1.sequence }
        if let previous = runs[snapshot.id], (previous.events.last?.sequence ?? 0) > (snapshot.events.last?.sequence ?? 0) { return }
        guard runs[snapshot.id] != snapshot else { return }
        let known = runs[snapshot.id] != nil
        if runs[snapshot.id]?.events != snapshot.events {
            activities[snapshot.id] = AgentActivity.make(from: snapshot.events)
        }
        runs[snapshot.id] = snapshot
        if !known { order.append(snapshot.id) }
        while order.count > 8 {
            let old = order.removeFirst(); runs.removeValue(forKey: old); activities.removeValue(forKey: old)
        }
        if latest == nil || latest?.id == snapshot.id || !known { latest = snapshot }
    }
    func restoreArchived(_ chats:[Conversation], selected:UUID) {
        for chat in chats where chat.id==selected {
            for message in chat.messages.filter({$0.role == .assistant}).suffix(7) {
                guard let id=message.agentTaskId,runs[id]==nil,let events=message.agentEvents,
                    let status=AgentTaskSnapshot.Status(rawValue:message.agentStatus ?? ""),!status.active else{continue}
                let request=chat.messages.first{$0.agentTaskId==id && $0.role == .user}
                let snapshot=AgentTaskSnapshot(id:id,owner:"",conversationId:chat.id,requestId:request?.agentRequestId,
                    task:request?.content ?? "",workspace:request?.agentWorkspace ?? "",status:status,
                    events:events,approval:nil,result:message.content,usage:message.agentUsage)
                activities[id]=AgentActivity.make(from:events)
                runs[id]=snapshot;order.append(id)
                while order.count>8 {
                    let old=order.removeFirst()
                    if old==latest?.id {order.append(old);continue}
                    runs.removeValue(forKey:old);activities.removeValue(forKey:old)
                }
            }
        }
    }
    func reserveSubmission() -> Bool {
        guard !busy else { return false }
        submitting = true; error = nil
        return true
    }
    func submit(task: String, workspace: URL?, conversation: UUID, request: UUID, model: String) async -> Bool {
        defer { submitting = false }
        do {
            _ = try await client.start(task: task, workspace: workspace, conversation: conversation, request: request, model: model)
            try await restore()
            return true
        } catch {
            if let rejection=error as? BackendError,rejection.submissionRejected {
                self.error=friendlyError(error)
                unconfirmedSubmission=false
            }else{
                self.error = "Couldn’t confirm the task submission. Reconnect to check its state; lorca will not resend it. \(friendlyError(error))"
                unconfirmedSubmission = true
            }
            return false
        }
    }
    func canDecide(_ task: AgentTaskSnapshot, approval: AgentApproval) -> Bool {
        connected && deciding == nil && !cancelling && !blockedApprovals.contains(approval.id)
            && task.owner == sessionID && latest?.id == task.id && latest?.approval?.id == approval.id && task.status == .awaitingApproval
    }
    func decide(task: AgentTaskSnapshot, approval: AgentApproval, decision: String) async {
        guard canDecide(task, approval: approval) else { return }
        deciding = approval.id; blockedApprovals.insert(approval.id); error = nil
        defer { deciding = nil }
        do {
            try await client.decide(task: task.id, approval: approval.id, decision: decision)
            let snapshot = try await client.snapshot(); apply(snapshot)
        } catch {
            self.error = "Couldn’t confirm this decision. Reconnect to check whether the approval is still pending. \(friendlyError(error))"
        }
    }
    func cancel() async {
        guard let task = active, task.owner == sessionID, !cancelling else { return }
        cancelling = true; defer { cancelling = false }
        do { try await client.cancel(task: task.id); error = nil }
        catch { self.error = "Cancellation wasn’t confirmed. \(friendlyError(error))" }
    }
}
