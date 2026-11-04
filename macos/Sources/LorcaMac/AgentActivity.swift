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
    var superseded = false
    var cancelled = false
    var elapsedMillis: Int?
    var startedMillis: Int?
    var statusLabel: String {
        if superseded {return "Superseded"}; if cancelled {return "Cancelled"}; if failed {return result["exitCode"].int.map {"Failed · exit \($0)"} ?? "Failed"}; if result["running"].boolean == true {return "Process running"}; if completed {return "Completed"}; if started {return "Running"}; if approval != nil {return "Waiting for approval"}; return "Queued"
    }
    var running: Bool { result["running"].boolean == true || started && !completed }
    var title: String {
        let path = arguments["path"].string ?? arguments["query"].string ?? ""
        switch tool {
        case "list_directory": return "List \(path)"
        case "read_file": return "Read \(path)"
        case "search_files": return "Search for \(arguments["query"].string ?? "")"
        case "apply_patch": return "Edit \(path)"
        case "write_stdin": return arguments["chars"].string?.isEmpty == false ? "Send command input" : "Check command result"
        case "view_image": return "Inspect image \(path)"
        case "request_user_input": return "Question for you"
        case "update_plan": return "Update task plan"
        case "git_diff": return "Inspect working-tree changes"
        case "run_command", "exec_command": return ([arguments["executable"].string ?? ""] + arguments["arguments"].strings).joined(separator: " ")
        default: return tool ?? "Activity"
        }
    }
    static func make(from events: [AgentEvent]) -> [Self] {
        var items: [Self] = []; var indices: [String: Int] = [:]
        for event in events {
            if event.kind == "task_revision_requested" {
                for index in items.indices where !items[index].started && !items[index].completed { items[index].superseded = true }
            }
            if event.kind == "task_cancelled" {for index in items.indices where !items[index].completed && !items[index].superseded {items[index].cancelled = true}}
            guard let id = event.data["callId"].string else { continue }
            if indices[id] == nil { indices[id] = items.count; items.append(Self(id: id)) }
            let index = indices[id]!
            switch event.kind {
            case "tool_requested": items[index].tool = event.data["tool"].string; items[index].arguments = event.data["arguments"]
            case "tool_started": items[index].started = true; items[index].action = event.data["action"]; items[index].startedMillis=event.data["presentation"]["elapsedMillis"].int
            case "tool_completed", "tool_failed":
                items[index].completed = true; items[index].failed = event.kind == "tool_failed"
                items[index].result = event.data["result"]
                if let start=items[index].startedMillis,let end=event.data["presentation"]["elapsedMillis"].int {items[index].elapsedMillis=max(0,end-start)}
                if let session=items[index].result["sessionId"].string, items[index].result["running"].boolean == false {
                    for prior in items.indices where prior != index && items[prior].result["sessionId"].string==session {
                        items[prior].result=items[index].result;items[prior].failed=items[index].failed
                    }
                }
                if items[index].tool == nil { items[index].tool = event.data["tool"].string }
            case "tool_skipped":
                items[index].completed=true;items[index].note=event.data["result"]["error"].string
                items[index].superseded=(items[index].note ?? "").lowercased().contains("superseded")
                items[index].cancelled = !items[index].superseded
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
        case text(String), tools([AgentActivity]), thinking(AgentThinkingBlock), compaction
    }
    let id: String
    var content: Content

    static func make(text: String, events: [AgentEvent], activities: [AgentActivity], thinking: [AgentThinkingBlock] = []) -> [Self] {
        let bytes = Array(text.utf8)
        let byID = Dictionary(activities.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var result: [Self] = [], seen = Set<String>(), cursor = 0
        func appendText(to end: Int) {
            guard end > cursor else { return }
            let value = String(decoding: bytes[cursor..<end], as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            if !value.isEmpty { result.append(Self(id: "text-\(cursor)", content: .text(value))) }
            cursor = end
        }
        var reasoning=thinking.filter {$0.state != "off"}
        func appendThinking(before sequence: UInt64) {
            while let block=reasoning.first, block.afterSequence < sequence {
                var offset=min(bytes.count,max(cursor,block.textOffset))
                while offset<bytes.count && bytes[offset] & 0xC0 == 0x80 {offset+=1}
                appendText(to:offset);result.append(Self(id:"thinking-\(block.id)",content:.thinking(block)));reasoning.removeFirst()
            }
        }
        for event in events {
            appendThinking(before:event.sequence)
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
        appendThinking(before: UInt64.max)
        appendText(to: bytes.count)
        return result
    }
}

extension AgentActivity {
    static func summary(_ items: [Self]) -> String {
        var labels: [String] = []
        let edited = Set(items.filter { $0.tool == "apply_patch" && $0.completed && !$0.failed && !$0.superseded && !$0.cancelled }
            .compactMap { $0.arguments["path"].string })
        for item in items {
            let done = item.completed && !item.failed && !item.superseded && !item.cancelled && item.result["running"].boolean != true
            let label: String
            switch item.tool {
            case "request_user_input": label = "Question for you"
            case "update_plan": label = "Working plan"
            case "read_file": label = done ? "Read files" : item.running ? "Reading files" : "File reads"
            case "search_files": label = done ? "Searched files" : item.running ? "Searching files" : "File searches"
            case "list_directory": label = done ? "Listed files" : item.running ? "Listing files" : "Directory inspection"
            case "git_diff": label = done ? "Reviewed changes" : item.running ? "Reviewing changes" : "Change inspection"
            case "apply_patch": label = done ? "Edited \(edited.count) \(edited.count == 1 ? "file" : "files")" : item.running ? "Editing files" : "File edits"
            case "run_command", "exec_command":
                let tests = item.arguments["executable"].string == "cargo" && item.arguments["arguments"].strings.first == "test"
                label = done ? (tests ? "Ran tests" : "Ran commands") : item.running ? (tests ? "Running tests" : "Running commands") : "Commands"
            default: label = item.tool ?? "Tool activity"
            }
            let qualified = item.superseded ? "\(label) · superseded" : item.cancelled ? "\(label) · cancelled" : item.failed ? "\(label) · failed" : label
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
