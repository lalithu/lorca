import SwiftUI

struct NotesProposalPreview: View {
    let proposal: NotesProposal
    let apply: () -> Void
    let dismiss: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(proposal.title).font(.system(size: 12, weight: .medium))
                Spacer()
                Button(action: dismiss) { Image(systemName: "xmark") }
                    .buttonStyle(.plain).accessibilityLabel("Dismiss suggestion")
            }
            ScrollView {
                Text(.init(proposal.text ?? ""))
                    .font(.system(size: 13)).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxHeight: 260)
            HStack {
                Text("Review before adding to your note")
                    .font(.system(size: 10)).foregroundStyle(.secondary)
                Spacer()
                Button(proposal.kind == "breakdown" ? "Add to note" : "Use this", action: apply).controlSize(.small)
            }
        }.padding(16).frame(width: 390)
    }
}
