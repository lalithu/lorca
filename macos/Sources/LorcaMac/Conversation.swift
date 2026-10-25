import Foundation

struct ChatAttachment: Identifiable, Equatable, Codable, Sendable {
    let id: UUID
    let name: String
    let width: Int
    let height: Int
    var fileName: String { "\(id.uuidString).jpg" }
}

struct ChatMessage: Identifiable, Equatable, Codable, Sendable {
    enum Role: String, Codable { case user, assistant }
    let id: UUID
    let role: Role
    var content: String
    var attachments: [ChatAttachment]
    var agentTaskId: String?
    var agentRequestId: UUID?
    var agentKind: String?
    var agentStatus: String?
    var agentWorkspace: String?

    init(role: Role, content: String, attachments: [ChatAttachment] = []) {
        id = UUID()
        self.role = role
        self.content = content
        self.attachments = attachments
    }

    private enum CodingKeys: CodingKey { case id, role, content, attachments, agentTaskId, agentRequestId, agentKind, agentStatus, agentWorkspace }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(UUID.self, forKey: .id)
        role = try values.decode(Role.self, forKey: .role)
        content = try values.decode(String.self, forKey: .content)
        attachments = try values.decodeIfPresent([ChatAttachment].self, forKey: .attachments) ?? []
        agentTaskId = try values.decodeIfPresent(String.self, forKey: .agentTaskId)
        agentRequestId = try values.decodeIfPresent(UUID.self, forKey: .agentRequestId)
        agentKind = try values.decodeIfPresent(String.self, forKey: .agentKind)
        agentStatus = try values.decodeIfPresent(String.self, forKey: .agentStatus)
        agentWorkspace = try values.decodeIfPresent(String.self, forKey: .agentWorkspace)
    }
}

struct Conversation: Identifiable, Equatable, Codable {
    var version = 1
    var id = UUID()
    var title = "New chat"
    var createdAt = Date()
    var updatedAt = Date()
    var messages: [ChatMessage] = []
    var draft = ""
    var draftImages: [ChatAttachment] = []

    var attachmentIDs: Set<UUID> { Set((messages.flatMap(\.attachments) + draftImages).map(\.id)) }
}
