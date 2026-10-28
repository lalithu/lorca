import Combine
import SwiftUI

enum Appearance: String, CaseIterable, Identifiable {
    case system = "System", dark = "Dark", light = "Light"
    var id: String { rawValue }
}

@MainActor
final class ChatStore: ObservableObject {
    let library = ConversationLibrary()
    let modelSession = ModelSession()
    let backend: BackendTransport
    let agent: AgentStore
    let jobs = JobAppsStore()
    var agentWorkspace: URL? { agent.workspaces[library.selectedID] }
    var currentAgentActive: Bool { agent.active?.conversationId == library.selectedID }
    var messages: [ChatMessage] { library.current.messages }
    var draft: String { get { library.current.draft } set { library.setDraft(newValue) } }
    var draftImages: [ChatAttachment] { library.current.draftImages }
    var imageStore: AttachmentStore { AttachmentStore(directory: backend.root.appendingPathComponent("attachments")) }
    var canSend: Bool { canSendChat && !agent.busy && !jobs.running }
    var canSendChat: Bool { ready && !closing && !isManagingModel && !agent.submitting && !currentAgentActive && (!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !draftImages.isEmpty)
        && !isGenerating && !isImportingImages && !modelSession.isLoading && !modelSession.isClosing }
    let uiZoom: AppZoom
    @Published var isImportingImages = false
    @Published private(set) var isReloadingModel = false
    @Published private(set) var isUnloadingModel = false
    var isManagingModel: Bool { isReloadingModel || isUnloadingModel }
    var canReloadModel: Bool { ready && !closing && !isManagingModel && !isGenerating && !agent.busy && !jobs.running && !jobs.busy && !isChecking && !modelSession.isLoading && !modelSession.isClosing }
    var canUnloadModel: Bool { canReloadModel && modelSession.activeModel != nil }
    @Published private(set) var models: [String] = []
    @Published var selectedModel: String {
        didSet {
            defaults.set(selectedModel, forKey: "model")
            guard oldValue != selectedModel, !applying else { return }
            stop()
            enqueue(["op": "selectModel", "model": selectedModel])
        }
    }
    @Published var appearance: Appearance {
        didSet { defaults.set(appearance.rawValue, forKey: "appearance"); appearanceChanged?() }
    }
    @Published private(set) var isConnected = false
    @Published private(set) var isChecking = false
    @Published private(set) var isGenerating = false
    @Published private(set) var receivedText = false
    @Published var error: String?
    @Published private(set) var usage = ChatUsage()
    var displayUsage: ChatUsage {
        if let task=agent.latest,task.conversationId==library.selectedID,task.model==nil || task.model==selectedModel {return task.usage ?? ChatUsage(contextUsed:nil)}
        return usage
    }
    var appearanceChanged: (() -> Void)?
    private let defaults: UserDefaults
    private var updates: Set<AnyCancellable> = []
    private var pending: Task<Void, Never>?
    private var eventTask: Task<Void, Never>?
    private var applying = false
    private var revision: UInt64 = 0
    @Published private(set) var ready = false
    private var closing = false
    private var requestsStopped = false
    private var pendingTurn: UUID?

    init(backend: BackendTransport? = nil, defaults: UserDefaults = .standard) {
        self.backend = backend ?? BackendClient()
        self.defaults = defaults
        uiZoom = AppZoom(defaults: defaults)
        agent = AgentStore(backend: self.backend)
        selectedModel = defaults.string(forKey: "model") ?? "qwen3.5:9b"
        appearance = Appearance(rawValue: defaults.string(forKey: "appearance") ?? "Dark") ?? .dark
        for source in [uiZoom.objectWillChange, modelSession.objectWillChange, library.objectWillChange, agent.objectWillChange, jobs.objectWillChange] {
            source.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &updates)
        }
        library.recoverDrafts(at: self.backend.root.appendingPathComponent("pending-drafts.json"))
        library.perform = { [weak self] in self?.enqueue($0) }
        modelSession.clearAction = { [weak self] in self?.enqueue(["op": "clearError"]) }
        modelSession.prepareAction = { [weak self] model, approve in
            guard let self else { return false }
            await self.pending?.value
            _ = await self.perform(["op": "prepare", "model": model, "unloadPrevious": approve])
            return self.modelSession.activeModel == model
        }
        modelSession.closeAction = { [weak self] unload in
            guard let self else { return [] }
            return await self.shutdown(unload: unload)
        }
    }
    func shutdown(unload: Bool, drainSeconds: TimeInterval = 3, closeSeconds: TimeInterval = 15) async -> [String] {
        guard !closing else { return [] }
        closing = true
        // Recovery is separate from Rust's canonical files; never overwrite a
        // conversation with a potentially stale UI projection.
        library.checkpointDrafts(at: backend.root.appendingPathComponent("pending-drafts.json"))
        eventTask?.cancel(); eventTask = nil
        agent.shutdown()
        let jobsTask = Task { await jobs.shutdown() }
        let queued = pending
        let drained = await finishWithin(drainSeconds) { await queued?.value; return true } ?? false
        if !drained { queued?.cancel() }
        pending = nil
        requestsStopped = true
        _ = await finishWithin(1) { [backend] in await backend.cancelRequests() }
        let result = await finishWithin(closeSeconds) { [self] in
            do {
                let data = try await backend.command(["op": "close", "unload": unload])
                struct Closed: Decodable { let failures: [String] }
                return try JSONDecoder().decode(Closed.self, from: data).failures
            } catch { return unload ? modelSession.modelsToRelease.sorted() : [] }
        }
        _ = await finishWithin(1) { [backend] in await backend.cancelRequests() }
        await jobsTask.value
        return result ?? (unload ? modelSession.modelsToRelease.sorted() : [])
    }
    var modelChoices: [String] { Array(Set(models + [selectedModel])).sorted() }
    var status: String {
        if modelSession.isClosing { return "Closing session" }
        if isUnloadingModel { return "Unloading model" }
        if isReloadingModel { return "Reloading model" }
        if modelSession.isLoading { return "Loading model" }
        if let task = agent.active { return task.presentationStatus }
        if isGenerating { return receivedText ? "Responding" : "Thinking" }
        if isChecking { return "Connecting" }
        return isConnected && ready ? "Connected" : "Offline"
    }
    func refreshConnection() async {
        guard !isChecking, !closing else { return }
        isChecking = true
        defer { isChecking = false }
        do {
            if !ready {
                revision = 0
                let state = try await backend.connect(model: selectedModel)
                guard !closing else { return }
                apply(state)
                ready = true
                library.restorePendingDrafts()
                observeEvents()
                agent.observe()
            }
            await pending?.value
            guard !closing else { return }
            if agent.unconfirmedSubmission || !agent.connected { try? await agent.restore() }
            guard await perform(["op": "refresh"]) else { return }
            guard isConnected else { return }
            if models.isEmpty { error = "No local models yet. Run ‘ollama pull qwen3.5:9b’ to get started."; return }
            let chosen = models.contains(selectedModel) ? selectedModel : models.contains("qwen3.5:9b") ? "qwen3.5:9b" : models[0]
            if selectedModel != chosen { selectedModel = chosen }
        } catch { ready = false; isConnected = false; self.error = friendlyError(error) }
    }
    private func observeEvents() {
        eventTask?.cancel()
        eventTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled && !self.closing {
                do { try await self.backend.events { [weak self] in self?.apply($0) } }
                catch {
                    if Task.isCancelled || self.closing { return }
                    // Reconnect state, never replay send commands. Rust saved any partial reply.
                    do {
                        let data = try await self.backend.command(["op": "snapshot"])
                        self.apply(try JSONDecoder().decode(BackendSnapshot.self, from: data))
                    } catch {
                        guard !Task.isCancelled, !self.closing else { return }
                        self.ready = false
                        self.isConnected = false
                        self.isGenerating = false
                        self.pendingTurn = nil
                        self.error = friendlyError(error)
                        do {
                            self.revision = 0
                            let state = try await self.backend.connect(model: self.selectedModel)
                            guard !self.closing, !Task.isCancelled else { return }
                            self.apply(state)
                            self.ready = true
                            self.agent.observe()
                            try? await self.agent.restore()
                            self.library.restorePendingDrafts()
                            self.error = nil
                            await self.refreshConnection()
                        } catch { self.error = friendlyError(error) }
                    }
                    try? await Task.sleep(for: .seconds(1))
                }
            }
        }
    }
    func enqueue(_ fields: [String: Any]) {
        guard !closing else { return }
        let preceding = pending
        pending = Task { [weak self] in
            await preceding?.value
            guard !Task.isCancelled else { return }
            _ = await self?.perform(fields)
        }
    }
    @discardableResult private func perform(_ fields: [String: Any]) async -> Bool {
        guard !requestsStopped, !Task.isCancelled else { return false }
        do {
            let data = try await backend.command(fields)
            if fields["op"] as? String == "draft", let id = (fields["conversationId"] as? String).flatMap(UUID.init(uuidString:)), let text = fields["text"] as? String {
                library.acknowledgedDraft(text, id: id)
            }
            if fields["op"] as? String != "send" { apply(try JSONDecoder().decode(BackendSnapshot.self, from: data)) }
            error = nil
            return true
        } catch {
            self.error = friendlyError(error)
            if fields["op"] as? String == "send" { pendingTurn = nil; isGenerating = false }
            return false
        }
    }
    func apply(_ snapshot: BackendSnapshot) {
        guard !closing, snapshot.revision >= revision, let session = snapshot.sessions.first(where: { $0.id == backend.sessionID }) else { return }
        revision = snapshot.revision
        library.apply(snapshot, selected: session.selectedId)
        agent.restoreArchived(snapshot.conversations,selected:session.selectedId)
        for chat in snapshot.conversations where agent.workspaces[chat.id] == nil {
            if let path = chat.messages.last(where: { $0.agentWorkspace != nil })?.agentWorkspace {
                agent.workspaces[chat.id] = URL(fileURLWithPath: path)
            }
        }
        modelSession.apply(session)
        models = session.models
        isConnected = session.isConnected
        applying = true
        selectedModel = session.model
        applying = false
        if let pendingTurn, snapshot.turns.contains(where: { $0.id == pendingTurn }) { self.pendingTurn = nil }
        let turn = snapshot.turns.last { $0.conversationId == session.selectedId }
        isGenerating = turn?.running == true || pendingTurn != nil
        receivedText = isGenerating && messages.last?.role == .assistant && messages.last?.content.isEmpty == false
        if let turn, turn.model == session.model {
            usage = turn.running ? ChatUsage(contextUsed: nil, contextLimit: session.usage.contextLimit) : turn.usage
        } else { usage = session.usage }
    }
    func send() {
        guard !closing, canSend, agent.reserveSubmission() else { return }
        let conversation = library.selectedID, request = UUID(), text = draft, model = selectedModel
        let workspace=agentWorkspace
        let preceding = pending
        pending = Task { [weak self] in
            await preceding?.value
            guard let self, !self.closing, !Task.isCancelled else { return }
            _ = await self.agent.submit(task: text, workspace: workspace, conversation: conversation, request: request, model: model)
            _ = await self.perform(["op": "snapshot"])
        }
    }
    func sendChat() { send() }
    // Called only after the UI's shared-model warning is accepted. Rust release
    // enforces other-session leases; prepare reuses the existing loading UI.
    func reloadModel() async {
        guard canReloadModel else { return }
        let model = selectedModel
        isReloadingModel = true
        defer { isReloadingModel = false }
        await pending?.value
        guard !closing, !isGenerating, !agent.busy else { return }
        do {
            _ = try await backend.command(["op": "release", "model": model])
            _ = await perform(["op": "snapshot"])
            _ = await perform(["op": "prepare", "model": model, "unloadPrevious": false])
        } catch { self.error = "Couldn’t reload the model. \(friendlyError(error))" }
    }
    // Release only; the next message prepares the selected model normally.
    func unloadModel(model requestedModel: String? = nil) async {
        guard canReloadModel, let model = requestedModel ?? modelSession.activeModel else { return }
        isUnloadingModel = true; error = nil
        defer { isUnloadingModel = false }
        await pending?.value
        guard !isGenerating, !agent.busy, !jobs.running, !jobs.busy, !closing else { return }
        do {
            _ = try await backend.command(["op": "release", "model": model])
            _ = await perform(["op": "snapshot"])
        } catch { self.error = "Couldn’t unload the model. \(friendlyError(error))" }
    }
    func stop() {
        if currentAgentActive { Task { await agent.cancel() }; return }
        stopReply()
    }
    private func stopReply() { guard ready, !closing else { return }; enqueue(["op": "cancel", "conversationId": library.selectedID.uuidString]) }
    func newChat() { stopReply(); enqueue(["op": "new"]) }
    func openConversation(_ id: UUID) {
        guard id != library.selectedID else { return }
        stopReply(); enqueue(["op": "open", "conversationId": id.uuidString])
    }
    func deleteConversation(_ id: UUID) { stopReply(); enqueue(["op": "delete", "conversationId": id.uuidString]) }
}
