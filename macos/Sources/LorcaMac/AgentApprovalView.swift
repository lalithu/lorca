import SwiftUI

struct AgentApprovalView: View {
    @ObservedObject var store: AgentStore
    let task: AgentTaskSnapshot
    let approval: AgentApproval
    var inline = false

    private var pending: Bool { task.approval?.id == approval.id && task.status == .awaitingApproval }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            summary
            if let diff = approval.action["diff"].string {
                if let kind = approval.action["changeKind"].string {
                    Label(kind, systemImage: "doc.text").font(.caption).foregroundStyle(ChatAccent.activity)
                }
                AgentDiffView(diff: diff).equatable()
            }
            if pending {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) { actions }
                    VStack(alignment: .leading, spacing: 6) { actions }
                }
                .controlSize(.small)
                .disabled(!store.canDecide(task, approval: approval))
                if store.deciding == approval.id {
                    Text("Submitting decision…").font(.caption).foregroundStyle(.secondary)
                } else if !store.connected {
                    Label("Reconnect to verify this approval before deciding.",
                          systemImage: "exclamationmark.circle")
                        .font(.caption).foregroundStyle(ChatAccent.approval)
                } else if task.owner != store.sessionID {
                    Text("Approval belongs to the lorca session that started this run.").font(.caption).foregroundStyle(.secondary)
                } else if task.correctionPending {
                    Text("A correction is queued. Review the next request when it arrives.").font(.caption).foregroundStyle(.secondary)
                }
            }
            DisclosureGroup(approval.preparedCommand == nil ? "Show exact request details" : "Show exact command details") {
                // Use the existing display redaction and keep the original approval intact.
                AgentOutputView(text: approval.action.redacted.pretty, copyLabel: "Copy request details")
                    .padding(.top, 5)
            }
            .font(.caption).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var summary: some View {
        VStack(alignment: .leading, spacing: 4) {
            if !inline {
                Text(pending ? "Approval required" : "Requested action")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(pending ? ChatAccent.approval : Color.secondary)
            }
            if let command = approval.preparedCommand {
                if !inline { Text(verbatim: command).font(.system(size: 12, design: .monospaced)) }
                if let directory = approval.action["workingDirectory"].string {
                    let path = Text(verbatim: directory).font(.system(.callout, design: .monospaced))
                    Text(pending ? "Allow Lorca to run this command in \(path)?" : "Working directory \(path)")
                        .font(.callout)
                } else if pending {
                    Text("Allow Lorca to run this command?").font(.callout)
                }
            } else if let path = approval.action["path"].string {
                let file = Text(verbatim: path).font(.system(.callout, design: .monospaced))
                Text(pending ? "Allow Lorca to change \(file)?" : "File \(file)").font(.callout)
            } else {
                let tool = Text(verbatim: approval.tool).font(.system(.callout, design: .monospaced))
                Text(pending ? "Allow Lorca to use \(tool)?" : "Tool \(tool)").font(.callout)
                if let target = approval.action["url"].string ?? approval.action["query"].string {
                    Text(verbatim: target).font(.system(.callout, design: .monospaced))
                }
            }
            if let scope = approval.action["scope"].string {
                Text(verbatim: scope).font(.caption).foregroundStyle(.secondary)
            }
        }.textSelection(.enabled)
    }

    @ViewBuilder private var actions: some View {
        Button("Allow Once") { decide("allow_once") }.buttonStyle(.borderedProminent)
            .accessibilityIdentifier("agent-allow-once")
        Button("Deny") { decide("deny") }.buttonStyle(.bordered)
            .accessibilityIdentifier("agent-deny")
        Button("Cancel Task", role: .destructive) { decide("cancel") }.buttonStyle(.borderless)
    }

    private func decide(_ decision: String) {
        Task { await store.decide(task: task, approval: approval, decision: decision) }
    }
}

extension AgentApproval {
    var preparedCommand: String? {
        guard tool == "exec_command" || tool == "run_command",
              let executable = action["executable"].string, !executable.isEmpty,
              case .array(let arguments) = action["arguments"],
              arguments.allSatisfy({ $0.string != nil }) else { return nil }
        // Quote unusual values as JSON strings so argument boundaries stay readable.
        return ([AgentJSON.string(executable)] + arguments).map { value in
            let token = value.string ?? ""
            return !token.isEmpty && token.unicodeScalars.allSatisfy {
                CharacterSet.alphanumerics.contains($0) || "/._-:=".unicodeScalars.contains($0)
            } ? token : value.pretty
        }.joined(separator: " ")
    }
}
