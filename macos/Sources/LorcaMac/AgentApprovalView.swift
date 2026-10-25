import SwiftUI

struct AgentApprovalView: View {
    @ObservedObject var store: AgentStore
    let task: AgentTaskSnapshot
    let approval: AgentApproval
    var pending: Bool { task.approval?.id == approval.id && task.status == .awaitingApproval }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(pending ? "Approval required" : "Requested action", systemImage: "hand.raised")
                .font(.system(size: 12, weight: .semibold)).foregroundStyle(Color.lorcaBlue)
            if let diff = approval.action["diff"].string {
                Text(approval.action["path"].string ?? "File change").font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                AgentDiffView(diff: diff)
            } else {
                // JSON preserves exact argument boundaries, whitespace, and escaping.
                AgentOutputView(text: approval.action.pretty)
            }
            if pending {
                HStack(spacing: 12) {
                    Button("Allow Once") { decide("allow_once") }.buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("agent-allow-once")
                    Button("Deny") { decide("deny") }.buttonStyle(.bordered)
                        .accessibilityIdentifier("agent-deny")
                    Spacer(minLength: 0)
                    Button("Cancel Task", role: .destructive) { decide("cancel") }.buttonStyle(.borderless)
                }
                .controlSize(.small).disabled(!store.canDecide(task, approval: approval))
                if store.deciding == approval.id {
                    Text("Submitting decision…").font(.caption).foregroundStyle(.secondary)
                } else if task.owner != store.sessionID {
                    Text("Approval belongs to the lorca session that started this run.").font(.caption).foregroundStyle(.secondary)
                } else if !store.connected {
                    Text("Reconnect to verify this approval before deciding.").font(.caption).foregroundStyle(.secondary)
                }
            } else {
                Text("This approval is no longer pending.").font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(12).background(Color.lorcaBlue.opacity(0.055), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.lorcaBlue.opacity(pending ? 0.35 : 0.12)))
    }
    private func decide(_ decision: String) { Task { await store.decide(task: task, approval: approval, decision: decision) } }
}
