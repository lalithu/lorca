import Foundation

// One atomic JSON file per chat. Images never go into the JSON.
struct ConversationRepository {
    let root: URL
    var chatsDirectory: URL { root.appendingPathComponent("conversations", isDirectory: true) }
    var imageStore: AttachmentStore { AttachmentStore(directory: root.appendingPathComponent("attachments", isDirectory: true)) }

    init(root: URL = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent(Bundle.main.object(forInfoDictionaryKey: "CFBundleName") as? String ?? "lorca", isDirectory: true)) {
        self.root = root
    }

    struct Loaded {
        var conversations: [Conversation] = []
        var selectedID: UUID?
        var warning: String?
    }

    func load() throws -> Loaded {
        try FileManager.default.createDirectory(at: chatsDirectory, withIntermediateDirectories: true)
        var result = Loaded()
        for url in try FileManager.default.contentsOfDirectory(at: chatsDirectory, includingPropertiesForKeys: nil) where url.pathExtension == "json" {
            do {
                let conversation = try JSONDecoder().decode(Conversation.self, from: Data(contentsOf: url))
                guard conversation.version == 1, UUID(uuidString: url.deletingPathExtension().lastPathComponent) == conversation.id else {
                    throw CocoaError(.fileReadCorruptFile)
                }
                result.conversations.append(conversation)
            } catch {
                result.warning = "Some saved chats couldn’t be read. Their files were left untouched in Application Support."
            }
        }
        result.conversations.sort { $0.updatedAt > $1.updatedAt }
        let selection = root.appendingPathComponent("selected.json")
        if FileManager.default.fileExists(atPath: selection.path) {
            do { result.selectedID = try JSONDecoder().decode(UUID.self, from: Data(contentsOf: selection)) }
            catch { result.warning = result.warning ?? "The last open chat couldn’t be restored. Your saved chats are still available." }
        }
        return result
    }

    func save(_ conversation: Conversation) throws {
        try FileManager.default.createDirectory(at: chatsDirectory, withIntermediateDirectories: true)
        try JSONEncoder().encode(conversation).write(to: chatURL(conversation.id), options: .atomic)
    }

    func select(_ id: UUID) throws {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try JSONEncoder().encode(id).write(to: root.appendingPathComponent("selected.json"), options: .atomic)
    }

    func delete(_ id: UUID) throws {
        let url = chatURL(id)
        if FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }
    }

    func removeUnusedAttachments(_ candidates: Set<UUID>) throws {
        guard !candidates.isEmpty else { return }
        let saved = try load()
        // An unreadable chat could reference an image. Prefer an orphan to losing that image.
        guard saved.warning == nil else { return }
        let referenced = Set(saved.conversations.flatMap(\.attachmentIDs))
        for id in candidates.subtracting(referenced) { try imageStore.remove(id) }
    }

    private func chatURL(_ id: UUID) -> URL { chatsDirectory.appendingPathComponent("\(id.uuidString).json") }
}
