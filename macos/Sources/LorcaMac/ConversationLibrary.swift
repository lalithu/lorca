import Combine
import Foundation

// An observable projection of Rust state. No conversation files are written here.
@MainActor
final class ConversationLibrary: ObservableObject {
    @Published private(set) var conversations: [Conversation] = [Conversation()]
    @Published private(set) var selectedID = UUID()
    @Published var error: String?
    var perform: (([String: Any]) -> Void)?
    private var recoveryURL: URL?
    private var pendingDrafts: [UUID: String] = [:]
    init() { selectedID = conversations[0].id }
    var current: Conversation { conversations.first { $0.id == selectedID } ?? conversations[0] }
    var recent: [Conversation] { conversations.sorted { $0.updatedAt > $1.updatedAt } }
    func apply(_ snapshot: BackendSnapshot, selected: UUID) {
        var chats = snapshot.conversations
        for index in chats.indices {
            if let draft = pendingDrafts[chats[index].id] { chats[index].draft = draft }
        }
        guard !chats.isEmpty else { return }
        conversations = chats
        selectedID = selected
        error = snapshot.warning
    }
    func restorePendingDrafts() {
        for (id, text) in pendingDrafts where conversations.contains(where: { $0.id == id }) {
            perform?(["op": "draft", "conversationId": id.uuidString, "text": text])
        }
    }
    func setDraft(_ text: String) {
        let id = selectedID
        pendingDrafts[id] = text
        if let index = conversations.firstIndex(where: { $0.id == id }) { conversations[index].draft = text }
        perform?(["op": "draft", "conversationId": id.uuidString, "text": text])
    }
    func acknowledgedDraft(_ text: String, id: UUID) {
        if pendingDrafts[id] == text { pendingDrafts.removeValue(forKey: id); saveRecovery() }
    }
    func recoverDrafts(at url: URL) {
        recoveryURL = url
        if let data = try? Data(contentsOf: url), let drafts = try? JSONDecoder().decode([UUID: String].self, from: data) {
            pendingDrafts = drafts
        }
    }
    func checkpointDrafts(at url: URL) { recoveryURL = url; saveRecovery() }
    private func saveRecovery() {
        guard let url = recoveryURL else { return }
        do {
            if pendingDrafts.isEmpty { if FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }; return }
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(pendingDrafts).write(to: url, options: .atomic)
        } catch { self.error = "Couldn’t save pending drafts: \(error.localizedDescription)" }
    }
    func rename(_ id: UUID, to title: String) { perform?(["op": "rename", "conversationId": id.uuidString, "title": title]) }
    func removeDraftImage(_ id: UUID) { perform?(["op": "removeImage", "conversationId": selectedID.uuidString, "imageId": id.uuidString]) }
}
