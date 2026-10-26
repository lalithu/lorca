import SwiftUI

struct AgentDiffView: View {
    let diff: String
    @State private var expandedGaps = Set<Int>()
    var body: some View {
        ScrollView([.horizontal, .vertical]) {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(UnifiedDiff(diff).rows()) { row in
                    switch row.content {
                    case .line(let line): diffLine(line)
                    case .gap(let hidden):
                        if expandedGaps.contains(row.id) {
                            ForEach(hidden) { line in diffLine(line) }
                        } else {
                            Button {
                                expandedGaps.insert(row.id)
                            } label: {
                                Label("Expand \(hidden.count) unchanged lines", systemImage: "arrow.up.and.down")
                                    .font(.system(size: 11)).foregroundStyle(.secondary)
                                    .fixedSize(horizontal: true, vertical: false)
                                    .padding(.horizontal, 10).padding(.vertical, 5)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }.buttonStyle(.plain)
                        }
                    }
                }
            }.fixedSize(horizontal: true, vertical: false).frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(minHeight: 70, maxHeight: 300)
        .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityLabel("Complete proposed code diff")
        .onChange(of: diff) { _, _ in expandedGaps.removeAll() }
    }
    private func diffLine(_ line: AgentDiffLine) -> some View {
        HStack(alignment: .top, spacing: 10) {
            if line.kind != .header {
                Text(line.oldLine.map(String.init) ?? "").frame(width: 38, alignment: .trailing).foregroundStyle(.secondary)
                Text(line.newLine.map(String.init) ?? "").frame(width: 38, alignment: .trailing).foregroundStyle(.secondary)
            }
            Text(line.text.isEmpty ? " " : line.text)
                .textSelection(.enabled).fixedSize(horizontal: true, vertical: true)
                .foregroundStyle(color(line.kind))
        }
        .font(.system(size: 11, design: .monospaced))
        .padding(.horizontal, 10).padding(.vertical, 2)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(color(line.kind).opacity(line.kind == .addition || line.kind == .removal ? 0.09 : 0))
    }
    private func color(_ kind: AgentDiffLine.Kind) -> Color {
        switch kind { case .addition: .green; case .removal: .red; case .header: .secondary; case .context: .primary }
    }
}
struct AgentOutputView: View {
    let text: String
    var body: some View {
        ScrollView([.horizontal, .vertical]) {
            Text(text).font(.system(size: 11, design: .monospaced))
                .textSelection(.enabled).fixedSize(horizontal: true, vertical: false)
                .frame(maxWidth: .infinity, alignment: .leading).padding(10)
        }
        .frame(minHeight: 35, maxHeight: 220)
        .background(.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityLabel("Tool output")
    }
}
