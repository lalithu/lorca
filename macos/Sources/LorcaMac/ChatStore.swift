import Combine
import SwiftUI

enum Appearance: String, CaseIterable, Identifiable {
    case system = "System", dark = "Dark", light = "Light"
    var id: String { rawValue }
}

@MainActor
final class ChatStore: ObservableObject {
    @Published var messages: [ChatMessage] = []
    @Published var draft = ""
    @Published private(set) var chatTextZoom: CGFloat = 0
    @Published var models: [String] = []
    @Published var selectedModel: String {
        didSet {
            defaults.set(selectedModel, forKey: "model")
            usage = ChatUsage(contextUsed: messages.isEmpty ? 0 : nil)
            if !isChecking && oldValue != selectedModel { loadSelectedModel() }
        }
    }
    @Published var appearance: Appearance {
        didSet {
            defaults.set(appearance.rawValue, forKey: "appearance")
            appearanceChanged?()
        }
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
    private var sessionUpdates: AnyCancellable?

    init(client: OllamaClient = OllamaClient(), defaults: UserDefaults = .standard) {
        self.client = client
        self.defaults = defaults
        modelSession = ModelSession(client: client)
        selectedModel = defaults.string(forKey: "model") ?? "qwen3.5:9b"
        appearance = Appearance(rawValue: defaults.string(forKey: "appearance") ?? "Dark") ?? .dark
        sessionUpdates = modelSession.objectWillChange.sink { [weak self] _ in
            self?.objectWillChange.send()
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
            isConnected = true
            error = models.isEmpty ? "No local models yet. Run ‘ollama pull qwen3.5:9b’ to get started." : nil
            if !models.isEmpty && !models.contains(selectedModel) {
                selectedModel = models.contains("qwen3.5:9b") ? "qwen3.5:9b" : models[0]
            }
            if !models.isEmpty, await modelSession.prepare(selectedModel).value {
                await refreshContextLimit()
            }
        } catch {
            isConnected = false
            self.error = friendlyError(error)
        }
    }

    private func loadSelectedModel() {
        let model = selectedModel
        let loading = modelSession.prepare(model)
        Task { [weak self] in
            let ready = await loading.value
            guard let self, self.selectedModel == model, ready else { return }
            await self.refreshContextLimit()
        }
    }

    private func refreshContextLimit() async {
        let model = selectedModel
        let limit = try? await client.contextLimit(model: model)
        guard selectedModel == model else { return }
        usage.contextLimit = limit
    }

    func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !isGenerating, !modelSession.isLoading, !modelSession.isClosing else { return }
        draft = ""
        error = nil
        messages.append(ChatMessage(role: .user, content: text))
        let history = messages.filter { !$0.content.isEmpty }
        let response = ChatMessage(role: .assistant, content: "")
        messages.append(response)
        let id = UUID()
        generationID = id
        isGenerating = true
        receivedText = false
        usage.contextUsed = nil
        usage.tokensPerSecond = nil
        let model = selectedModel
        generation = Task { [weak self] in
            guard let self else { return }
            defer {
                if self.generationID == id {
                    self.isGenerating = false
                    self.generationID = nil
                    self.generation = nil
                    self.removeEmptyReply(response.id)
                }
            }
            do {
                guard await self.modelSession.prepare(model).value else { return }
                guard !Task.isCancelled, self.generationID == id else { return }
                let usage = try await self.client.reply(model: model, messages: history) { [weak self] text in
                    guard let self, self.generationID == id,
                          let index = self.messages.firstIndex(where: { $0.id == response.id }) else { return }
                    self.messages[index].content += text
                    self.receivedText = true
                    self.isConnected = true
                }
                if self.generationID == id {
                    self.isConnected = true
                    self.usage = usage
                }
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
    }

    func resizeChatText(by amount: CGFloat) {
        chatTextZoom = min(10, max(-4, chatTextZoom + amount))
    }

    func newChat() {
        stop()
        messages = []
        draft = ""
        error = nil
        usage = ChatUsage(contextLimit: usage.contextLimit)
    }

    private func removeEmptyReply(_ id: UUID) {
        messages.removeAll { $0.id == id && $0.role == .assistant && $0.content.isEmpty }
    }
}
