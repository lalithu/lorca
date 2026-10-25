import SwiftUI

struct AgentDiffLine: Identifiable, Equatable {
    enum Kind { case addition, removal, context, header }
    let id: Int
    let text: String
    var kind: Kind {
        if text.hasPrefix("+++") || text.hasPrefix("---") || text.hasPrefix("@@") || text.hasPrefix("diff ") { return .header }
        if text.hasPrefix("+") { return .addition }
        if text.hasPrefix("-") { return .removal }
        return .context
    }
    static func parse(_ text: String) -> [Self] { text.components(separatedBy: "\n").enumerated().map { Self(id: $0.offset, text: $0.element) } }
}
struct AgentDiffView: View {
    let diff: String
    var body: some View {
        ScrollView([.horizontal, .vertical]) {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(AgentDiffLine.parse(diff)) { line in
                    Text(line.text.isEmpty ? " " : line.text)
                        .font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                        .foregroundStyle(color(line.kind)).padding(.horizontal, 10).padding(.vertical, 2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(color(line.kind).opacity(line.kind == .addition || line.kind == .removal ? 0.09 : 0))
                }
            }.fixedSize(horizontal: true, vertical: false).frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(minHeight: 70, maxHeight: 300)
        .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityLabel("Complete proposed code diff")
    }
    private func color(_ kind: AgentDiffLine.Kind) -> Color {
        switch kind { case .addition: .green; case .removal: .red; case .header: .secondary; case .context: .primary }
    }
}
struct AgentOutputView: View {
    let text: String
    var body: some View {
        ScrollView([.horizontal, .vertical]) {
            Text(text).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading).padding(10)
        }
        .frame(minHeight: 35, maxHeight: 220)
        .background(.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityLabel("Tool output")
    }
}
