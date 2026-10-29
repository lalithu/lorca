import SwiftUI

struct AgentActivityView: View {
    @ObservedObject var store: AgentStore
    let taskID: String
    var savedMessage: ChatMessage? = nil
    var textSize: CGFloat = 14
    var onChoose: ((String) -> Void)? = nil
    var onContinue: (() -> Void)? = nil
    private var snapshot: AgentTaskSnapshot? {
        if let current = store.runs[taskID] { return current }
        guard let message = savedMessage,
              let status = AgentTaskSnapshot.Status(rawValue: message.agentStatus ?? "") else { return nil }
        return AgentTaskSnapshot(id: taskID, owner: "", conversationId: nil, requestId: nil, task: "",
            workspace: message.agentWorkspace ?? "", status: status, events: message.agentEvents ?? [],
            approval: nil, result: message.content, usage: message.agentUsage)
    }
    private func response(_ task: AgentTaskSnapshot) -> String {
        guard let streamed = task.assistantText else { return savedMessage?.content ?? task.result ?? "" }
        var text = streamed.trimmingCharacters(in: .whitespacesAndNewlines)
        if !task.status.active && task.status != .completed, let result = task.result {
            text += (text.isEmpty ? "" : "\n\n") + result
        } else if text.isEmpty && !task.status.active { text = task.result ?? "" }
        return text
    }
    var body: some View {
        if let task = snapshot {
            let text = response(task)
            let events = (savedMessage?.agentEvents?.count ?? 0) > task.events.count ? savedMessage!.agentEvents! : task.events
            let activities = events == task.events ? (store.activities[task.id] ?? AgentActivity.make(from: events)) : AgentActivity.make(from: events)
            let timeline = AgentTimelineItem.make(text: text, events: events, activities: activities)
            VStack(alignment: .leading, spacing: 16) {
                AgentRunHeader(store: store, task: task, text: text)
                if events.first?.sequence ?? 0 > 1 {
                    Text("Earlier activity is unavailable.").font(.caption).foregroundStyle(.secondary)
                }
                ForEach(timeline) { row in
                    switch row.content {
                    case .text(let text): MarkdownText(text: text, size: textSize).textSelection(.enabled)
                    case .tools(let items): AgentActivityGroup(store: store, task: task, items: items)
                    case .compaction:
                        Label("Context automatically compacted", systemImage: "arrow.down.right.and.arrow.up.left")
                            .font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
                if [.incomplete, .failed, .cancelled].contains(task.status), let resume = onContinue {
                    Button("Continue task", action: resume).buttonStyle(.bordered).controlSize(.small)
                }
                if task.status == .waitingForInput, let choose = onChoose,
                   let question = events.last(where: { $0.kind == "tool_completed" && $0.data["result"]["needsUserInput"].boolean == true }) {
                    ViewThatFits(in: .horizontal) {
                        HStack { ForEach(question.data["result"]["options"].strings, id: \.self) { option in Button(option) { choose(option) } } }
                        VStack(alignment: .leading) { ForEach(question.data["result"]["options"].strings, id: \.self) { option in Button(option) { choose(option) } } }
                    }.buttonStyle(.bordered).controlSize(.small)
                }
                if task.status.active && task.generating == true {
                    Text(text.isEmpty ? "Thinking…" : "Working…").font(.caption).foregroundStyle(.secondary)
                }
            }.frame(maxWidth: .infinity, alignment: .leading)
        } else if let message = savedMessage {
            MarkdownText(text: message.content, size: textSize).textSelection(.enabled)
        }
    }
}

private struct AgentRunHeader: View {
    @ObservedObject var store: AgentStore
    let task: AgentTaskSnapshot
    let text: String
    private var interrupted: Bool {
        task.status == .cancelled && text.contains("Agent task interrupted when the Rust backend stopped.")
    }
    var body: some View {
        // Only this small view ticks. It owns no timer and never republishes ChatStore.
        if task.status.active {
            TimelineView(.periodic(from: .now, by: 1)) { _ in header }
        } else { header }
    }
    private var header: some View {
            HStack(spacing: 7) {
                if task.status.active { ProgressView().controlSize(.mini) }
                else { Image(systemName: task.status == .completed ? "checkmark" : "circle.dashed").font(.system(size: 10)) }
                let status = interrupted ? "Interrupted" : task.status == .awaitingApproval ? "Awaiting Approval" : task.status.label
                if let elapsed = AgentElapsed(events: task.events) {
                    let duration = AgentElapsed.label(elapsed.seconds(active: task.status.active))
                    Text(task.status == .running ? "Working for \(duration)" : "\(status) · \(interrupted ? "at least " : "")\(duration)")
                        .monospacedDigit()
                } else { Text(status) }
                Spacer(minLength: 0)
                if task.status.active {
                    Button("Cancel") { Task { await store.cancel() } }.buttonStyle(.borderless)
                        .disabled(store.cancelling || task.owner != store.sessionID || !store.connected)
                        .accessibilityLabel("Cancel agent task")
                }
            }.font(.system(size: 11)).foregroundStyle(.secondary)
    }
}

private struct AgentActivityGroup: View {
    @ObservedObject var store: AgentStore
    let task: AgentTaskSnapshot
    let items: [AgentActivity]
    @State private var expanded = false
    private var pending: Bool { items.contains { $0.id == task.approval?.callId } }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Button { expanded.toggle() } label: {
                HStack(spacing: 7) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right").font(.system(size: 9))
                    Image(systemName: "terminal").font(.system(size: 11))
                    Text(AgentActivity.summary(items)).multilineTextAlignment(.leading)
                    if pending { Text("· Awaiting approval").foregroundStyle(Color.lorcaBlue) }
                }.font(.system(size: 12)).foregroundStyle(.secondary).contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityLabel("Tool activity: \(AgentActivity.summary(items))")
            if expanded {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(items) { item in AgentToolRow(store: store, task: task, item: item) }
                }.padding(12).background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 10))
            } else if pending, let approval = task.approval {
                // Keep the actual approval actionable even while details are collapsed.
                AgentApprovalView(store: store, task: task, approval: approval)
            }
        }
    }
}
private struct AgentToolRow: View {
    @ObservedObject var store: AgentStore
    let task: AgentTaskSnapshot
    let item: AgentActivity
    @State private var expanded = true
    private var pending: Bool { task.approval?.callId == item.id }
    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            Button { expanded.toggle() } label: {
                HStack(spacing: 8) {
                    Image(systemName: expanded || pending ? "chevron.down" : "chevron.right").font(.system(size: 8, weight: .semibold)).frame(width: 9)
                    if item.running && task.status.active { ProgressView().controlSize(.mini) }
                    else { Image(systemName: item.failed ? "exclamationmark.circle" : item.completed ? "checkmark" : "circle").font(.system(size: 10)).foregroundStyle(item.failed ? Color.red : Color.secondary) }
                    Text(item.title).font(.system(size: 12)).lineLimit(1).truncationMode(.middle)
                    Spacer(minLength: 0)
                    if item.approval != nil { Text(pending ? "Review" : item.failed ? "Not executed / failed" : item.started ? "Allowed once" : "Approval ended").font(.system(size: 10)).foregroundStyle(.secondary) }
                }.contentShape(Rectangle())
            }.buttonStyle(.plain)
            if expanded || pending {
                VStack(alignment: .leading, spacing: 9) {
                    if let approval = item.approval { AgentApprovalView(store: store, task: task, approval: approval) }
                    else if item.arguments != .null { AgentOutputView(text: item.arguments.pretty) }
                    if item.tool == "run_command" {
                        if let code = item.result["exitCode"].int {
                            Text("Exit \(code)").font(.system(size: 11, weight: .medium))
                                .foregroundStyle(code == 0 ? Color.green : Color.red)
                        }
                        if let output = item.result["output"].string { AgentOutputView(text: output) }
                        else if !item.output.isEmpty { AgentOutputView(text: item.output) }
                        if let error = item.result["error"].string { Text(error).font(.caption).foregroundStyle(.red) }
                    } else if item.tool == "read_file", let content = item.result["content"].string {
                        AgentOutputView(text: content)
                    } else if item.result != .null {
                        if item.tool == "git_diff", let diff = item.result["output"].string { AgentDiffView(diff: diff) }
                        else { AgentOutputView(text: item.result.pretty) }
                    }
                    if item.result["truncated"].boolean == true {
                        Text("Output truncated by the Rust tool limit.").font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(.leading, 17)
            }
        }.padding(.vertical, 4)
    }
}
struct AgentMiniStatus: View {
    @ObservedObject var store: AgentStore
    let open: () -> Void
    var body: some View {
        if let task = store.active {
            Button(action: open) {
                HStack(spacing: 8) {
                    Image(systemName: task.status == .awaitingApproval ? "hand.raised" : "terminal")
                    Text(task.status == .awaitingApproval ? "Review action in Chats" : "Working · Open Chats")
                    Spacer(minLength: 0)
                    Image(systemName: "arrow.up.right")
                }.font(.system(size: 11)).foregroundStyle(Color.lorcaBlue).padding(9)
            }.buttonStyle(.plain).background(Color.lorcaBlue.opacity(0.06), in: RoundedRectangle(cornerRadius: 9))
        }
    }
}
