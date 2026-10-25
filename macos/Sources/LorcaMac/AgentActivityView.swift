import SwiftUI

struct AgentActivityView: View {
    @ObservedObject var store: AgentStore
    let taskID: String
    var body: some View {
        if let task = store.runs[taskID] {
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 9) {
                    if task.status.active { ProgressView().controlSize(.mini) }
                    else { Image(systemName: task.status == .completed ? "checkmark.circle" : "circle.dashed").foregroundStyle(.secondary) }
                    Text(task.status.label).font(.system(size: 12, weight: .medium))
                    Text(URL(fileURLWithPath: task.workspace).lastPathComponent).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                    Spacer()
                    if task.status.active {
                        Button("Cancel") { Task { await store.cancel() } }.buttonStyle(.borderless).controlSize(.small)
                            .disabled(store.cancelling || task.owner != store.sessionID || !store.connected)
                            .accessibilityLabel("Cancel agent task")
                    }
                }
                if task.events.first?.sequence ?? 0 > 1 {
                    Text("Earlier activity is no longer in the backend’s bounded event buffer.").font(.caption).foregroundStyle(.secondary)
                }
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(store.activities[task.id] ?? []) { item in
                        if let note = item.note { MarkdownText(text: note, size: 13).textSelection(.enabled) }
                        else { AgentToolRow(store: store, task: task, item: item) }
                    }
                }
                if task.status == .running, task.events.last?.kind == "model_generating" {
                    Text("Model processing…").font(.system(size: 11)).foregroundStyle(.secondary)
                }
                if let error = store.error, task.status.active { agentError(error) }
            }
            .padding(14).background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(.primary.opacity(0.07)))
        }
    }
    private func agentError(_ error: String) -> some View {
        HStack(alignment: .top) {
            Text(error).font(.caption).foregroundStyle(.secondary)
            Spacer()
            Button("Reconnect") { Task { try? await store.restore() } }.controlSize(.small)
        }
    }
}
private struct AgentToolRow: View {
    @ObservedObject var store: AgentStore
    let task: AgentTaskSnapshot
    let item: AgentActivity
    @State private var expanded = false
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
            if expanded || pending || (item.running && task.status.active) {
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
                    Text(task.status == .awaitingApproval ? "Review agent action in Chats" : "Agent working · Open Chats")
                    Spacer(minLength: 0)
                    Image(systemName: "arrow.up.right")
                }.font(.system(size: 11)).foregroundStyle(Color.lorcaBlue).padding(9)
            }.buttonStyle(.plain).background(Color.lorcaBlue.opacity(0.06), in: RoundedRectangle(cornerRadius: 9))
        }
    }
}
