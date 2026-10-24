import Combine
import SwiftUI

enum Appearance: String, CaseIterable, Identifiable {
    case system = "System", dark = "Dark", light = "Light"
    var id: String { rawValue }
}

@MainActor
final class ChatStore: ObservableObject {
    let library: ConversationLibrary
    var messages: [ChatMessage] {
        get { library.current.messages }
        set { library.edit { $0.messages = newValue } }
    }
    var draft: String {
        get { library.current.draft }
        set { library.edit { $0.draft = newValue } }
    }
    var draftImages: [ChatAttachment] { library.current.draftImages }
    var imageStore: AttachmentStore { library.repository.imageStore }
    var canSend: Bool { (!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !draftImages.isEmpty)
        && !isGenerating && !isImportingImages && !modelSession.isLoading && !modelSession.isClosing }
    @Published private(set) var chatTextZoom: CGFloat = 0
    @Published var isImportingImages = false
    @Published var models: [String] = []
    @Published var selectedModel: String {
        didSet {
            defaults.set(selectedModel, forKey: "model")
            guard oldValue != selectedModel else { return }
            if isGenerating { stop() }
            usage = ChatUsage(contextUsed: messages.isEmpty ? 0 : nil)
            if !isChecking && !restoringModel { loadSelectedModel() }
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
    let modelSession: ModelSession

    private let client: OllamaClient
    private let defaults: UserDefaults
    private var generation: Task<Void, Never>?
    private var generationID: UUID?
    private var updates: Set<AnyCancellable> = []
    private var restoringModel = false
    private var visionCapabilities: [String: Bool] = [:]

    init(client: OllamaClient = OllamaClient(), defaults: UserDefaults = .standard,
         repository: ConversationRepository = ConversationRepository()) {
        self.client = client
        self.defaults = defaults
        library = ConversationLibrary(repository: repository)
        modelSession = ModelSession(client: client)
        selectedModel = defaults.string(forKey: "model") ?? "qwen3.5:9b"
        appearance = Appearance(rawValue: defaults.string(forKey: "appearance") ?? "Dark") ?? .dark
        resetUsage()
        for source in [modelSession.objectWillChange, library.objectWillChange] {
            source.sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &updates)
        }
    }

    var modelChoices: [String] { Array(Set(models + [selectedModel])).sorted() }
    var status: String {
        if modelSession.isClosing { return "Closing session" }
        if modelSession.isLoading { return "Loading model" }
        if isGenerating { return receivedText ? "Responding" : "Thinking" }
        if isChecking { return "Connecting" }
        return isConnected ? "Connected" : "Offline"
    }

    func refreshConnection() async {
        guard !isChecking else { return }
        isChecking = true
        defer { isChecking = false }
        do {
            models = try await client.models()
            visionCapabilities = [:]
            isConnected = true
            error = models.isEmpty ? "No local models yet. Run ‘ollama pull qwen3.5:9b’ to get started." : nil
            if !models.isEmpty && !models.contains(selectedModel) {
                selectedModel = models.contains("qwen3.5:9b") ? "qwen3.5:9b" : models[0]
            }
            if !models.isEmpty, await modelSession.prepare(selectedModel).value { await refreshContextLimit() }
        } catch { isConnected = false; self.error = friendlyError(error) }
    }

    private func loadSelectedModel() {
        let model = selectedModel
        let loading = modelSession.prepare(model)
        Task { [weak self] in
            let ready = await loading.value
            guard let self, self.selectedModel == model else { return }
            if ready { await self.refreshContextLimit() }
            else if let previous = self.modelSession.activeModel {
                self.restoringModel = true
                self.selectedModel = previous
                self.restoringModel = false
            }
        }
    }

    private func refreshContextLimit() async {
        let model = selectedModel
        let limit = try? await client.contextLimit(model: model)
        guard selectedModel == model else { return }
        usage.contextLimit = limit
    }

    func send() {
        guard canSend else { return }
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        let images = draftImages
        library.edit { conversation in
            conversation.draft = ""
            conversation.draftImages = []
            if conversation.title == "New chat", conversation.messages.isEmpty {
                conversation.title = String((text.isEmpty ? "Image: \(images.first?.name ?? "image")" : text).prefix(60))
            }
            conversation.messages.append(ChatMessage(role: .user, content: text, attachments: images))
        }
        let history = messages.filter { !$0.content.isEmpty || !$0.attachments.isEmpty }
        let response = ChatMessage(role: .assistant, content: "")
        messages.append(response)
        library.flush()
        error = nil
        let id = UUID()
        generationID = id
        isGenerating = true
        receivedText = false
        usage.contextUsed = nil
        usage.tokensPerSecond = nil
        let model = selectedModel
        let storage = imageStore
        generation = Task { [weak self] in
            guard let self else { return }
            defer {
                if self.generationID == id {
                    self.isGenerating = false
                    self.generationID = nil
                    self.generation = nil
                    self.removeEmptyReply(response.id)
                    self.library.flush()
                }
            }
            do {
                let attachments = history.flatMap(\.attachments)
                if !attachments.isEmpty {
                    if self.visionCapabilities[model] == nil, let supported = try await self.client.supportsVision(model: model) {
                        self.visionCapabilities[model] = supported
                    }
                    if self.visionCapabilities[model] == false { throw OllamaError.unsupportedVision(model) }
                }
                let imageData = try await Task.detached(priority: .userInitiated) {
                    var result: [UUID: Data] = [:]
                    var total = 0
                    for attachment in attachments where result[attachment.id] == nil {
                        let data = try storage.data(for: attachment)
                        total += data.count
                        guard total <= 24 * 1024 * 1024 else { throw ImageError.payloadTooLarge }
                        result[attachment.id] = data
                    }
                    return result
                }.value
                try Task.checkCancellation()
                guard self.generationID == id, await self.modelSession.prepare(model).value else { return }
                guard !Task.isCancelled, self.generationID == id else { return }
                let usage = try await self.client.reply(model: model, messages: history, imageData: imageData) { [weak self] text in
                    guard let self, self.generationID == id,
                          let index = self.messages.firstIndex(where: { $0.id == response.id }) else { return }
                    self.messages[index].content += text
                    self.receivedText = true
                    self.isConnected = true
                }
                if self.generationID == id { self.isConnected = true; self.usage = usage }
            } catch {
                guard self.generationID == id, !Task.isCancelled else { return }
                self.error = friendlyError(error)
                if error is URLError { self.isConnected = false }
            }
        }
    }

    func stop() {
        generationID = nil
        generation?.cancel()
        generation = nil
        isGenerating = false
        if let last = messages.last, last.role == .assistant { removeEmptyReply(last.id) }
        library.flush()
    }

    func resizeChatText(by amount: CGFloat) { chatTextZoom = min(10, max(-4, chatTextZoom + amount)) }
    func newChat() { stop(); library.create(); error = nil; resetUsage() }
    func openConversation(_ id: UUID) {
        guard id != library.selectedID else { return }
        stop(); library.open(id); error = nil; resetUsage()
    }
    func deleteConversation(_ id: UUID) {
        let active = id == library.selectedID
        if active { stop() }
        library.delete(id)
        if active { resetUsage() }
    }

    private func resetUsage() { usage = ChatUsage(contextUsed: messages.isEmpty ? 0 : nil, contextLimit: usage.contextLimit) }
    private func removeEmptyReply(_ id: UUID) {
        messages.removeAll { $0.id == id && $0.role == .assistant && $0.content.isEmpty }
    }
}
