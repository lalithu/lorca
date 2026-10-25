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

// A presentation of existing events, never a second execution or conversation store.
struct AgentTimelineItem: Identifiable, Equatable {
    enum Content: Equatable {
        case text(String), tools([AgentActivity]), compaction
    }
    let id: String
    var content: Content

    static func make(text: String, events: [AgentEvent], activities: [AgentActivity]) -> [Self] {
        let bytes = Array(text.utf8)
        let byID = Dictionary(activities.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var result: [Self] = [], seen = Set<String>(), cursor = 0
        func appendText(to end: Int) {
            guard end > cursor else { return }
            let value = String(decoding: bytes[cursor..<end], as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            if !value.isEmpty { result.append(Self(id: "text-\(cursor)", content: .text(value))) }
            cursor = end
        }
        for event in events {
            let call = event.data["callId"].string
            let item = call.flatMap { byID[$0] }
            let isTool = item != nil && !seen.contains(call!)
            guard event.kind == "context_compacted" || isTool else { continue }
            // Old archives have no offsets. Keep their text intact rather than
            // inventing where tools occurred inside a historical response.
            var offset = min(bytes.count, max(cursor, event.data["presentation"]["textBytes"].int ?? bytes.count))
            while offset < bytes.count && bytes[offset] & 0xC0 == 0x80 { offset += 1 }
            appendText(to: offset)
            if event.kind == "context_compacted" {
                result.append(Self(id: event.id, content: .compaction))
            } else if let item, let call {
                seen.insert(call)
                if let last = result.last, case .tools(var group) = last.content {
                    group.append(item); result[result.count - 1].content = .tools(group)
                } else { result.append(Self(id: "tools-\(call)", content: .tools([item]))) }
            }
        }
        appendText(to: bytes.count)
        return result
    }
}

extension AgentActivity {
    static func summary(_ items: [Self]) -> String {
        var labels: [String] = []
        let edited = Set(items.filter { $0.tool == "apply_patch" && $0.completed && !$0.failed }
            .compactMap { $0.arguments["path"].string })
        for item in items {
            let done = item.completed && !item.failed
            let label: String
            switch item.tool {
            case "read_file": label = done ? "Read files" : item.running ? "Reading files" : "File reads"
            case "search_files": label = done ? "Searched files" : item.running ? "Searching files" : "File searches"
            case "list_directory": label = done ? "Listed files" : item.running ? "Listing files" : "Directory inspection"
            case "git_diff": label = done ? "Reviewed changes" : item.running ? "Reviewing changes" : "Change inspection"
            case "apply_patch": label = done ? "Edited \(edited.count) \(edited.count == 1 ? "file" : "files")" : item.running ? "Editing files" : "File edits"
            case "run_command":
                let tests = item.arguments["executable"].string == "cargo" && item.arguments["arguments"].strings.first == "test"
                label = done ? (tests ? "Ran tests" : "Ran commands") : item.running ? (tests ? "Running tests" : "Running commands") : "Commands"
            default: label = item.tool ?? "Tool activity"
            }
            let qualified = item.failed ? "\(label) · failed" : label
            if !labels.contains(qualified) { labels.append(qualified) }
        }
        return labels.joined(separator: ", ")
    }
}

struct AgentElapsed {
    let startedNanos: Double
    let elapsedSeconds: Double
    let clock: String

    init?(events: [AgentEvent]) {
        guard let data = events.last(where: { $0.data["presentation"]["elapsedMillis"].int != nil })?.data["presentation"],
              let started = data["startedNanos"].int, let elapsed = data["elapsedMillis"].int,
              let clock = data["clock"].string else { return nil }
        startedNanos = Double(started); elapsedSeconds = Double(elapsed) / 1000; self.clock = clock
    }
    func seconds(active: Bool) -> Double {
        guard active else { return elapsedSeconds }
        let now: Double
        if clock == "mac_continuous" {
            var scale = mach_timebase_info_data_t()
            mach_timebase_info(&scale)
            now = Double(mach_continuous_time()) * Double(scale.numer) / Double(scale.denom)
        } else { now = Date().timeIntervalSince1970 * 1_000_000_000 }
        return max(elapsedSeconds, (now - startedNanos) / 1_000_000_000)
    }
    static func label(_ seconds: Double) -> String {
        let value = max(0, Int(seconds))
        if value >= 3600 { return "\(value / 3600)h \((value % 3600) / 60)m \(value % 60)s" }
        if value >= 60 { return "\(value / 60)m \(value % 60)s" }
        return "\(value)s"
    }
}
