import Foundation

// Pure display helpers: event offsets always refer to the original assistant text.
enum AgentPresentation {
    static func responseText(_ task: AgentTaskSnapshot, saved: String? = nil) -> String {
        guard var text = task.assistantText else { return saved ?? task.result ?? "" }
        if !task.status.active, task.status != .completed, let result = task.result,
           !result.isEmpty, !text.trimmingCharacters(in: .whitespacesAndNewlines).hasSuffix(result.trimmingCharacters(in: .whitespacesAndNewlines)) {
            text += (text.isEmpty ? "" : "\n\n") + result
        } else if text.isEmpty, !task.status.active { text = task.result ?? "" }
        return text
    }
    static func clarification(_ task: AgentTaskSnapshot, events: [AgentEvent]) -> AgentJSON? {
        guard task.status == .waitingForInput else { return nil }
        return events.last { $0.kind == "tool_completed" && $0.data["result"]["needsUserInput"].boolean == true }?.data["result"]
    }
    static func visibleText(_ text: String, question: String?) -> String {
        guard let question, !question.isEmpty else { return text }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed == question.trimmingCharacters(in: .whitespacesAndNewlines) { return "" }
        if trimmed.hasSuffix(question) { return String(trimmed.dropLast(question.count)).trimmingCharacters(in: .whitespacesAndNewlines) }
        return text
    }
}
