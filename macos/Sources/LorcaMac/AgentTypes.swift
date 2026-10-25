import Foundation

enum AgentJSON: Codable, Equatable, Sendable {
    case object([String: AgentJSON]), array([AgentJSON]), string(String), number(Double), bool(Bool), null
    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let v = try? c.decode(Bool.self) { self = .bool(v) }
        else if let v = try? c.decode(Double.self) { self = .number(v) }
        else if let v = try? c.decode(String.self) { self = .string(v) }
        else if let v = try? c.decode([String: AgentJSON].self) { self = .object(v) }
        else { self = .array(try c.decode([AgentJSON].self)) }
    }
    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .object(let v): try c.encode(v)
        case .array(let v): try c.encode(v)
        case .string(let v): try c.encode(v)
        case .number(let v): try c.encode(v)
        case .bool(let v): try c.encode(v)
        case .null: try c.encodeNil()
        }
    }
    subscript(_ key: String) -> AgentJSON { if case .object(let v) = self { return v[key] ?? .null }; return .null }
    var string: String? { if case .string(let v) = self { return v }; return nil }
    var int: Int? { if case .number(let v) = self { return Int(exactly: v) }; return nil }
    var boolean: Bool? { if case .bool(let v) = self { return v }; return nil }
    var strings: [String] { if case .array(let v) = self { return v.compactMap(\.string) }; return [] }
    var pretty: String {
        let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        return (try? encoder.encode(self)).flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }
}
struct AgentEvent: Codable, Equatable, Identifiable, Sendable {
    let id: String
    let sequence: UInt64
    let taskId: String
    let kind: String
    let data: AgentJSON
}
struct AgentApproval: Codable, Equatable, Identifiable, Sendable {
    let id: String
    let callId: String
    let tool: String
    let action: AgentJSON
}
struct AgentTaskSnapshot: Codable, Equatable, Identifiable, Sendable {
    enum Status: String, Codable, Sendable {
        case running, awaitingApproval = "awaiting_approval", completed, cancelled, failed
        var active: Bool { self == .running || self == .awaitingApproval }
        var label: String {
            switch self {
            case .running: "Working"
            case .awaitingApproval: "Approval required"
            case .completed: "Completed"
            case .cancelled: "Cancelled"
            case .failed: "Failed"
            }
        }
    }
    let id: String
    let owner: String
    let conversationId: UUID?
    let requestId: UUID?
    let task: String
    let workspace: String
    var status: Status
    var events: [AgentEvent]
    var approval: AgentApproval?
    let result: String?
}
