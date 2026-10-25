import Foundation

struct AgentActivity: Identifiable, Equatable {
    let id: String
    var tool: String?
    var arguments: AgentJSON = .null
    var action: AgentJSON = .null
    var result: AgentJSON = .null
    var approval: AgentApproval?
    var output = ""
    var failed = false
    var started = false
    var completed = false
    var note: String?
    var running: Bool { started && !completed }
    var title: String {
        let path = arguments["path"].string ?? arguments["query"].string ?? ""
        switch tool {
        case "list_directory": return "List \(path)"
        case "read_file": return "Read \(path)"
        case "search_files": return "Search for \(arguments["query"].string ?? "")"
        case "apply_patch": return "Edit \(path)"
        case "git_diff": return "Inspect working-tree changes"
        case "run_command": return ([arguments["executable"].string ?? ""] + arguments["arguments"].strings).joined(separator: " ")
        default: return tool ?? "Activity"
        }
    }
    static func make(from events: [AgentEvent]) -> [Self] {
        var items: [Self] = []; var indices: [String: Int] = [:]
        for event in events {
            guard let id = event.data["callId"].string else { continue }
            if indices[id] == nil { indices[id] = items.count; items.append(Self(id: id)) }
            let index = indices[id]!
            switch event.kind {
            case "tool_requested": items[index].tool = event.data["tool"].string; items[index].arguments = event.data["arguments"]
            case "tool_started": items[index].started = true; items[index].action = event.data["action"]
            case "tool_completed", "tool_failed":
                items[index].completed = true; items[index].failed = event.kind == "tool_failed"
                items[index].result = event.data["result"]
                if items[index].tool == nil { items[index].tool = event.data["tool"].string }
            case "approval_required":
                if let data = try? JSONEncoder().encode(event.data) { items[index].approval = try? JSONDecoder().decode(AgentApproval.self, from: data) }
            case "terminal_output":
                let stream = event.data["output"]["stream"].string ?? "stdout"
                items[index].output += "[\(stream)] \(event.data["output"]["text"].string ?? "")"
            default: break
            }
        }
        return items
    }
}
