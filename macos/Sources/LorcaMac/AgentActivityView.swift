import SwiftUI

struct AgentActivityView: View {
    @ObservedObject var store: AgentStore
    @AppStorage("showModelReasoning") private var showModelReasoning = true
    let taskID: String
    var savedMessage: ChatMessage? = nil
    var textSize: CGFloat = 14
    var onChoose: ((String) -> Void)? = nil
    @State private var clarificationAnswer = ""
    var expandTools = false
    var onContinue: (() -> Void)? = nil
    var onChooseWorkspace: (() -> Void)? = nil
    private var snapshot: AgentTaskSnapshot? {
        if let current = store.runs[taskID] { return current }
        guard let message = savedMessage,
              let status = AgentTaskSnapshot.Status(rawValue: message.agentStatus ?? "") else { return nil }
        return AgentTaskSnapshot(id: taskID, owner: "", conversationId: nil, requestId: nil, task: "",
            workspace: message.agentWorkspace ?? "", status: status, events: message.agentEvents ?? [],
            approval: nil, result: message.content, usage: message.agentUsage)
    }
    var body: some View {
        if let task = snapshot {
            let text = AgentPresentation.responseText(task, saved: savedMessage?.content)
            let events = (savedMessage?.agentEvents?.count ?? 0) > task.events.count ? savedMessage!.agentEvents! : task.events
            let activities = events == task.events ? (store.activities[task.id] ?? AgentActivity.make(from: events)) : AgentActivity.make(from: events)
            let clarification = AgentPresentation.clarification(task, events: events)
            let question = clarification?["question"].string
            let timeline = AgentTimelineItem.make(text: text, events: events, activities: activities, thinking: showModelReasoning ? (task.thinking ?? []) : [])
            VStack(alignment: .leading, spacing: 10) {
                if task.status != .completed { AgentRunHeader(store: store, task: task, text: text) }
                if events.first?.sequence ?? 0 > 1 {
                    Text("Earlier activity is unavailable.").font(.caption).foregroundStyle(.secondary)
                }
                ForEach(timeline) { row in
                    switch row.content {
                    case .text(let text):
                        let visible = AgentPresentation.visibleText(text, question: question)
                        if !visible.isEmpty { MarkdownText(text: visible, size: textSize, showsCopyButton: false, editorial: true)
                            .equatable().textSelection(.enabled).chatResponseSurface() }
                    case .thinking(let block):
                        ThinkingPanel(block: block, selection: Binding(
                            get: { store.thinkingExpanded(taskID: taskID, block: block) },
                            set: { store.setThinkingExpanded($0, taskID: taskID, blockID: block.id) }))
                    case .tools(let items): AgentActivityGroup(store: store, task: task, items: items, initiallyExpanded: expandTools)
                    case .compaction:
                        Label("Context automatically compacted", systemImage: "arrow.down.right.and.arrow.up.left")
                            .font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
                if [.incomplete, .failed, .cancelled].contains(task.status),
                   events.contains(where: { $0.kind == "task_contract" || $0.kind == "task_interpreted" }),
                   let resume = onContinue {
                    Button("Continue task", action: resume).buttonStyle(.bordered).controlSize(.small)
                }
                if let clarification {
                    MarkdownText(text: question ?? "Your answer is needed", size: textSize, showsCopyButton: false, editorial: true)
                        .padding(.horizontal, 10).padding(.vertical, 6)
                        .chatInlineAccent(ChatAccent.approval)
                    if clarification["needsWorkspace"].boolean == true, let chooseWorkspace = onChooseWorkspace {
                        Button("Choose folder…", action: chooseWorkspace).buttonStyle(.bordered).controlSize(.small)
                    } else if let choose = onChoose {
                        HStack {
                            TextField("Your answer", text: $clarificationAnswer).textFieldStyle(.roundedBorder).onSubmit {
                                if !clarificationAnswer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { choose(clarificationAnswer) }
                            }
                            Button("Answer") { choose(clarificationAnswer) }.disabled(clarificationAnswer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        }.accessibilityIdentifier("clarification-answer")
                        ViewThatFits(in: .horizontal) {
                            HStack { ForEach(clarification["options"].strings, id: \.self) { option in Button(option) { choose(option) } } }
                            VStack(alignment: .leading) { ForEach(clarification["options"].strings, id: \.self) { option in Button(option) { choose(option) } } }
                        }.buttonStyle(.bordered).controlSize(.small)
                    }
                }
                if !task.status.active {
                    ResponseDetails(usage: task.usage ?? savedMessage?.agentUsage, model: task.model ?? savedMessage?.agentModel,
                        elapsed: AgentElapsed(events: events)?.elapsedSeconds, text: text)
                }
                if task.status.active && task.generating == true {
                    Text("Generating response…").font(.caption).foregroundStyle(.secondary)
                }
            }.frame(maxWidth: .infinity, alignment: .leading)
        } else if let message = savedMessage {
            MarkdownText(text: message.content, size: textSize, editorial: true)
                .equatable().textSelection(.enabled).chatResponseSurface()
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
            if task.status == .awaitingApproval {
                Image(systemName: "clock").foregroundStyle(ChatAccent.approval)
            } else if task.status.active { ProgressView().controlSize(.mini) }
            else { Image(systemName: task.status == .completed ? "checkmark" : "circle.dashed").font(.system(size: 10)) }
            let status = interrupted ? "Interrupted" : task.status == .awaitingApproval
                ? (store.connected ? "Awaiting approval" : "Reconnect required") : task.status.label
            if let elapsed = AgentElapsed(events: task.events) {
                let duration = AgentElapsed.label(elapsed.seconds(active: task.status.active))
                Text(task.status == .running ? "Working for \(duration)" : "\(status) · \(interrupted ? "at least " : "")\(duration)")
                    .monospacedDigit()
            } else { Text(status) }
            if task.correctionPending {Text("Correction queued · waiting for a safe boundary").foregroundStyle(Color.lorcaBlue)}
            Spacer(minLength: 0)

            if task.status.active {
                Button("Cancel") { Task { await store.cancel() } }.buttonStyle(.borderless)
                    .disabled(store.cancelling || task.owner != store.sessionID || !store.connected)
                    .accessibilityLabel("Cancel agent task")
            }
        }
        .font(.system(size: task.status == .awaitingApproval ? 12 : 11))
        .foregroundStyle(.secondary)
    }
}

private struct AgentActivityGroup: View {
    @ObservedObject var store: AgentStore
    let task: AgentTaskSnapshot
    let items: [AgentActivity]
    var initiallyExpanded = false
    @State private var expanded = false
    private var visibleItems: [AgentActivity] {
        expanded ? items : items.filter { $0.id == task.approval?.callId }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button { expanded.toggle() } label: {
                HStack(spacing: 7) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right").font(.system(size: 9))
                    Image(systemName: "terminal").foregroundStyle(ChatAccent.activity)
                    Text(AgentActivity.summary(items)).fontWeight(.medium).multilineTextAlignment(.leading)
                    Text(items.count == 1 ? "1 action" : "\(items.count) actions").font(.caption)
                    Spacer(minLength: 0)
                }
                .font(.system(size: 12)).foregroundStyle(.secondary).contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityLabel("Tool activity: \(AgentActivity.summary(items))")
            if !visibleItems.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(visibleItems) { item in
                        if item.id != visibleItems.first?.id { Divider().opacity(0.6) }
                        AgentToolRow(store: store, task: task, item: item, initiallyExpanded: initiallyExpanded)
                    }
                }
                .padding(.leading, 12)
                .overlay(alignment: .leading) {
                    Rectangle().fill(ChatAccent.activity.opacity(0.18)).frame(width: 1).allowsHitTesting(false)
                }
            }
        }.onAppear { expanded = initiallyExpanded }
    }
}

private struct AgentToolRow: View {
    @ObservedObject var store: AgentStore
    let task: AgentTaskSnapshot
    let item: AgentActivity
    var initiallyExpanded = false
    @State private var expanded = false
    private var liveApproval: AgentApproval? {
        task.approval.flatMap { task.status == .awaitingApproval && $0.callId == item.id ? $0 : nil }
    }
    private var denied: Bool {
        item.failed && item.approval != nil && item.result["error"].string?.hasPrefix("User denied this action;") == true
    }
    private var status: String {
        if liveApproval != nil { return store.connected ? "Approval pending" : "Reconnect required" }
        if denied { return "Denied" }
        if item.approval != nil && item.started { return "\(item.statusLabel) · Allowed once" }
        if item.approval != nil && !item.started && !item.completed && !task.status.active { return task.status.label }
        return item.statusLabel
    }
    private var statusColor: Color {
        if liveApproval != nil { return ChatAccent.approval }
        if item.failed { return ChatAccent.error }
        if item.completed && !item.running && !item.cancelled && !item.superseded { return ChatAccent.success }
        return .secondary
    }
    private var title: String {
        if let approval = liveApproval {
            return approval.preparedCommand ?? approval.action["path"].string ?? approval.tool
        }
        return item.title.trimmingCharacters(in: .whitespaces).isEmpty ? (item.tool ?? "Activity") : item.title
    }
    private var technicalTitle: Bool {
        ["run_command", "exec_command", "read_file", "apply_patch", "list_directory", "search_files", "view_image"]
            .contains(item.tool ?? "")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            if let approval = liveApproval {
                rowHeader.textSelection(.enabled)
                AgentApprovalView(store: store, task: task, approval: approval, inline: true)
                    .padding(.leading, 16)
            } else {
                Button { expanded.toggle() } label: { rowHeader }.buttonStyle(.plain)
            }
            if expanded && liveApproval == nil {
                VStack(alignment: .leading, spacing: 7) {
                    if let approval = item.approval {
                        AgentApprovalView(store: store, task: task, approval: approval)
                    } else if item.arguments != .null, item.tool != "request_user_input" {
                        AgentOutputView(text: item.arguments.redacted.pretty, copyLabel: "Copy arguments")
                    }
                    result
                    if let note = item.note { Text(verbatim: note).font(.caption).foregroundStyle(.secondary).textSelection(.enabled) }
                    if item.result["truncated"].boolean == true {
                        Text("Output reduced; captured evidence remains in saved tool activity.").font(.caption).foregroundStyle(.secondary)
                    }
                }.padding(.leading, 16)
            }
        }.padding(.vertical, 3).onAppear { expanded = initiallyExpanded }
    }

    private var rowHeader: some View {
        HStack(alignment: .firstTextBaseline, spacing: 7) {
            if liveApproval == nil {
                Image(systemName: expanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 8, weight: .semibold)).frame(width: 9)
            }
            if item.running && task.status.active { ProgressView().controlSize(.mini) }
            else {
                Image(systemName: liveApproval != nil ? "clock" : item.failed ? "exclamationmark.circle"
                      : item.completed && !item.cancelled && !item.superseded ? "checkmark" : "circle")
                    .font(.system(size: 10)).foregroundStyle(statusColor)
            }
            Text(verbatim: title)
                .font(.system(size: 12, design: technicalTitle ? .monospaced : .default))
                .lineLimit(liveApproval == nil ? 1 : nil).truncationMode(.middle)
            Spacer(minLength: 4)
            if let millis = item.elapsedMillis {
                Text(String(format: "%.2f s", Double(millis) / 1000))
                    .font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary)
            }
            Text(status).font(.system(size: 11)).foregroundStyle(statusColor)
        }.contentShape(Rectangle())
    }

    @ViewBuilder private var result: some View {
        if ["run_command", "exec_command", "write_stdin"].contains(item.tool ?? "") {
            if let code = item.result["exitCode"].int {
                Text("Exit \(code)").font(.system(size: 11, weight: .medium, design: .monospaced))
                    .foregroundStyle(code == 0 ? ChatAccent.success : ChatAccent.error)
            }
            if let output = item.result["output"].string { AgentOutputView(text: output) }
            else if !item.output.isEmpty { AgentOutputView(text: item.output) }
            if let error = item.result["error"].string {
                Text(verbatim: error).font(.caption).foregroundStyle(ChatAccent.error).textSelection(.enabled)
            }
        } else if item.tool == "request_user_input", item.result["needsUserInput"].boolean == true {
            Text("Question presented in the conversation").font(.caption).foregroundStyle(.secondary)
        } else if item.tool == "update_plan", case .array(let steps) = item.result["plan"] {
            Text("Model working plan · not completion evidence").font(.caption).foregroundStyle(.secondary)
            ForEach(Array(steps.enumerated()), id: \.offset) { _, step in
                HStack {
                    Image(systemName: step["status"].string == "in_progress" ? "circle.inset.filled" : "circle")
                    Text(step["step"].string ?? "")
                    Spacer()
                    Text(step["status"].string ?? "").foregroundStyle(.secondary)
                }.font(.caption)
            }
        } else if item.tool == "apply_patch", item.approval == nil, let diff = item.action["diff"].string {
            Label(item.action["changeKind"].string ?? "File change", systemImage: "doc.text")
                .font(.caption).foregroundStyle(ChatAccent.activity)
            AgentDiffView(diff: diff).equatable()
        } else if item.tool == "read_file", let content = item.result["content"].string {
            AgentOutputView(text: content)
        } else if item.result != .null {
            if item.tool == "git_diff", let diff = item.result["output"].string { AgentDiffView(diff: diff).equatable() }
            else { AgentOutputView(text: item.result.redacted.pretty) }
        }
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
