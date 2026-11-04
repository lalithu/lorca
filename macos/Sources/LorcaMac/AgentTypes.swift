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
        case running, awaitingApproval = "awaiting_approval", waitingForInput = "waiting_for_input", completed, incomplete, denied, cancelled, failed
        var active: Bool { self == .running || self == .awaitingApproval }
        var label: String {
            switch self {
            case .running: "Working"
            case .awaitingApproval: "Approval required"
            case .waitingForInput: "Waiting for you"
            case .completed: "Completed"
            case .incomplete: "Incomplete"
            case .denied: "Denied"
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
    var revision: UInt64? = nil
    var model: String? = nil
    var assistantText: String? = nil
    var usage: ChatUsage? = nil
    var generating: Bool? = nil
    var thinking: [AgentThinkingBlock]? = nil
    var correctionPending: Bool {
        let requested=events.last(where:{$0.kind == "task_revision_requested"})?.sequence ?? 0
        let applied=events.last(where:{$0.kind == "task_revised"})?.sequence ?? 0
        return status.active && requested > applied
    }
    var presentationStatus: String {
        if status == .awaitingApproval {return status.label}
        if status == .running {
            if generating == true {return "Processing"}
            if let last=events.last, ["tool_started","terminal_output"].contains(last.kind){return "Executing"}
        }
        return status.label
    }
}

// Decoded only from the live TaskView. Never stored on Conversation/ChatMessage.
struct AgentThinkingBlock: Codable, Equatable, Identifiable, Sendable {
    let id: String
    var state: String
    var text: String
    let afterSequence: UInt64
    let textOffset: Int
    var truncated: Bool
    enum Stage: String, Codable, Sendable { case interpretation, execution }
    var stage: Stage? = nil
    var stageTitle: String { stage == .interpretation ? "Understanding request" : "Thinking" }
    var active: Bool { state == "running" }
    var label: String {
        switch state {case "running": stageTitle;case "complete": "\(stageTitle) · complete";case "empty": "No thinking emitted";case "interrupted": "Thinking interrupted";case "unavailable": "Thinking unavailable for this model";default: "Thinking off"}
    }
}

extension AgentJSON {
    var redacted: AgentJSON {
        switch self {
        case .object(let fields): return .object(fields.mapValuesWithKeys { key,value in
            let lower=key.lowercased().replacingOccurrences(of:"_",with:"")
            return ["token","apikey","authorization","password","secret","cookie","clientsecret","session","sessionid","taskid"].contains(where: {lower == $0 || lower.hasSuffix($0)}) ? .string("[redacted]") : value.redacted
        })
        case .array(let values): return .array(values.map(\.redacted))
        default: return self
        }
    }
}
private extension Dictionary where Key == String, Value == AgentJSON {
    func mapValuesWithKeys(_ transform:(String,AgentJSON)->AgentJSON)->[String:AgentJSON] { Dictionary(uniqueKeysWithValues:map {($0.key,transform($0.key,$0.value))}) }
}
