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
    var messages: [ChatMessage] { library.current.messages }
    var draft: String { get { library.current.draft } set { library.setDraft(newValue) } }
    var draftImages: [ChatAttachment] { library.current.draftImages }
    var imageStore: AttachmentStore { AttachmentStore(directory: backend.root.appendingPathComponent("attachments")) }
    var canSend: Bool { ready && (!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !draftImages.isEmpty)
        && !isGenerating && !isImportingImages && !modelSession.isLoading && !modelSession.isClosing }
    @Published private(set) var chatTextZoom: CGFloat = 0
    @Published var isImportingImages = false
    @Published private(set) var models: [String] = []
    @Published var selectedModel: String {
        didSet {
            defaults.set(selectedModel, forKey: "model")
            guard oldValue != selectedModel, !applying else { return }
            stop()
            let loading = modelSession.prepare(selectedModel)
            Task { _ = await loading.value }
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
    var appearanceChanged: (() -> Void)?
    private let defaults: UserDefaults
    private var updates: Set<AnyCancellable> = []
    private var pending: Task<Void, Never>?
    private var eventTask: Task<Void, Never>?
    private var applying = false
    private var revision: UInt64 = 0
    @Published private(set) var ready = false
    private var closing = false
    private var pendingTurn: UUID?

    init(backend: BackendTransport? = nil, defaults: UserDefaults = .standard) {
        self.backend = backend ?? BackendClient()
        self.defaults = defaults
        selectedModel = defaults.string(forKey: "model") ?? "qwen3.5:9b"
        appearance = Appearance(rawValue: defaults.string(forKey: "appearance") ?? "Dark") ?? .dark
        for source in [modelSession.objectWillChange, library.objectWillChange] {
            source.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &updates)
        }
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
            await self.pending?.value
            self.closing = true
            self.eventTask?.cancel()
            do {
                let data = try await self.backend.command(["op": "close", "unload": unload])
                struct Closed: Decodable { let failures: [String] }
                return try JSONDecoder().decode(Closed.self, from: data).failures
            } catch { return unload ? self.modelSession.modelsToRelease.sorted() : [] }
        }
    }
    var modelChoices: [String] { Array(Set(models + [selectedModel])).sorted() }
    var status: String {
        if modelSession.isClosing { return "Closing session" }
        if modelSession.isLoading { return "Loading model" }
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
                apply(try await backend.connect(model: selectedModel))
                ready = true
                library.restorePendingDrafts()
                observeEvents()
            }
            await pending?.value
            guard await perform(["op": "refresh"]) else { return }
            guard isConnected else { return }
            if models.isEmpty { error = "No local models yet. Run ‘ollama pull qwen3.5:9b’ to get started."; return }
            let chosen = models.contains(selectedModel) ? selectedModel : models.contains("qwen3.5:9b") ? "qwen3.5:9b" : models[0]
            _ = await modelSession.prepare(chosen).value
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
                        self.ready = false
                        self.isConnected = false
                        self.isGenerating = false
                        self.pendingTurn = nil
                        self.error = friendlyError(error)
                        do {
                            self.revision = 0
                            self.apply(try await self.backend.connect(model: self.selectedModel))
                            self.ready = true
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
        let preceding = pending
        pending = Task { [weak self] in
            await preceding?.value
            _ = await self?.perform(fields)
        }
    }
    @discardableResult private func perform(_ fields: [String: Any]) async -> Bool {
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
        guard snapshot.revision >= revision, let session = snapshot.sessions.first(where: { $0.id == backend.sessionID }) else { return }
        revision = snapshot.revision
        library.apply(snapshot, selected: session.selectedId)
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
        guard canSend else { return }
        let id = UUID()
        pendingTurn = id
        isGenerating = true
        error = nil
        enqueue(["op": "send", "conversationId": library.selectedID.uuidString, "requestId": id.uuidString])
    }
    func stop() { guard ready, !closing else { return }; enqueue(["op": "cancel", "conversationId": library.selectedID.uuidString]) }
    func resizeChatText(by amount: CGFloat) { chatTextZoom = min(10, max(-4, chatTextZoom + amount)) }
    func newChat() { stop(); enqueue(["op": "new"]) }
    func openConversation(_ id: UUID) {
        guard id != library.selectedID else { return }
        stop(); enqueue(["op": "open", "conversationId": id.uuidString])
    }
    func deleteConversation(_ id: UUID) { stop(); enqueue(["op": "delete", "conversationId": id.uuidString]) }
}
