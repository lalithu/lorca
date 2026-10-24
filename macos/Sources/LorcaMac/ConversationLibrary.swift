import Combine
import Foundation

@MainActor
final class ConversationLibrary: ObservableObject {
    @Published private(set) var conversations: [Conversation] = []
    @Published private(set) var selectedID = UUID()
    @Published var error: String?
    let repository: ConversationRepository
    private var dirty: Set<UUID> = []
    private var pendingSave: Task<Void, Never>?

    init(repository: ConversationRepository) {
        self.repository = repository
        do {
            let saved = try repository.load()
            conversations = saved.conversations.map { conversation in
                var restored = conversation
                restored.messages.removeAll { $0.role == .assistant && $0.content.isEmpty && $0.attachments.isEmpty }
                return restored
            }
            error = saved.warning
            if let selected = saved.selectedID, conversations.contains(where: { $0.id == selected }) {
                selectedID = selected
            } else if let first = conversations.first { selectedID = first.id }
        } catch { self.error = "Saved chats couldn’t be opened: \(error.localizedDescription)" }
        if conversations.isEmpty { create() }
    }

    var current: Conversation { conversations.first { $0.id == selectedID }! }
    var recent: [Conversation] { conversations.sorted { $0.updatedAt > $1.updatedAt } }

    func edit(_ change: (inout Conversation) -> Void) {
        guard let index = conversations.firstIndex(where: { $0.id == selectedID }) else { return }
        change(&conversations[index])
        conversations[index].updatedAt = Date()
        dirty.insert(selectedID)
        if pendingSave == nil {
            // Periodic, rather than trailing debounce: long streams are checkpointed too.
            pendingSave = Task { [weak self] in
                do { try await Task.sleep(for: .milliseconds(200)) } catch { return }
                self?.flush()
            }
        }
    }

    @discardableResult func flush() -> Bool {
        pendingSave?.cancel()
        pendingSave = nil
        do {
            for conversation in conversations where dirty.contains(conversation.id) {
                try repository.save(conversation)
                dirty.remove(conversation.id)
            }
            try repository.select(selectedID)
            return true
        } catch {
            self.error = "Chat changes couldn’t be saved: \(error.localizedDescription)"
            return false
        }
    }

    func create() {
        flush()
        let conversation = Conversation()
        conversations.insert(conversation, at: 0)
        selectedID = conversation.id
        dirty.insert(selectedID)
        flush()
    }

    func open(_ id: UUID) {
        guard conversations.contains(where: { $0.id == id }) else { return }
        flush()
        selectedID = id
        flush()
    }

    func rename(_ id: UUID, to title: String) {
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, let index = conversations.firstIndex(where: { $0.id == id }) else { return }
        conversations[index].title = String(title.prefix(120))
        dirty.insert(id)
        flush()
    }

    func delete(_ id: UUID) {
        guard let conversation = conversations.first(where: { $0.id == id }), flush() else { return }
        do {
            try repository.delete(id)
            conversations.removeAll { $0.id == id }
            dirty.remove(id)
            if conversations.isEmpty { create() }
            else if selectedID == id { selectedID = recent[0].id }
            flush()
            try repository.removeUnusedAttachments(conversation.attachmentIDs)
        } catch { self.error = "The chat or its unused images couldn’t be deleted: \(error.localizedDescription)" }
    }

    func removeDraftImage(_ id: UUID) {
        edit { $0.draftImages.removeAll { $0.id == id } }
        if flush() { cleanupImages([id]) }
    }

    func addImages(_ images: [ChatAttachment], to id: UUID) {
        guard let index = conversations.firstIndex(where: { $0.id == id }) else {
            cleanupImages(Set(images.map(\.id)))
            return
        }
        conversations[index].draftImages += images
        dirty.insert(id)
        flush()
    }

    private func cleanupImages(_ ids: Set<UUID>) {
        do { try repository.removeUnusedAttachments(ids) }
        catch { self.error = "Unused images couldn’t be removed: \(error.localizedDescription)" }
    }
}
