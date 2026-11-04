import SwiftUI

// Local-model prose, separate from execution evidence and terminal output.
struct ThinkingPanel: View {
    let block: AgentThinkingBlock
    var initiallyExpanded: Bool? = nil
    var selection: Binding<Bool>? = nil
    @State private var disclosure = ThinkingDisclosure()
    private var expanded: Bool { selection?.wrappedValue ?? disclosure.expanded }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                if let selection { selection.wrappedValue.toggle() }
                else { disclosure.toggle() }
            } label: {
                HStack(spacing: 7) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right").font(.system(size: 9))
                    if block.active { ProgressView().controlSize(.mini) }
                    else { Image(systemName: "brain").font(.system(size: 12)) }
                    Text(block.label).font(.system(size: 12, weight: .medium))
                    Spacer()
                    Text("Local model · session only").font(.system(size: 10)).foregroundStyle(.tertiary)
                }.foregroundStyle(.secondary).contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityLabel(block.label).accessibilityIdentifier("thinking-disclosure-" + block.id)
            if expanded {
                if block.text.isEmpty {
                    Text(block.active ? "Waiting for model thinking…" : block.label).font(.caption).foregroundStyle(.secondary)
                } else { ReasoningText(text: block.text) }
                if block.truncated { Text("Thinking display reached its session limit.").font(.caption).foregroundStyle(.secondary) }
            }
        }
        .padding(8).background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 8))
        .onAppear { if selection == nil { disclosure.appear(active: block.active, initiallyExpanded: initiallyExpanded) } }
        .onChange(of: block.state) { _, _ in if selection == nil { disclosure.update(active: block.active) } }
    }
}

struct ReasoningText: View {
    let text: String
    @State private var contentHeight: CGFloat = 28
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Spacer()
                Button("Copy thinking") { NativeClipboard.copy(text) }.buttonStyle(.borderless)
            }.font(.caption).foregroundStyle(.secondary)
            ScrollView(.vertical) {
                Text(text).font(.system(size: 12)).lineSpacing(3)
                    .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(8)
                    .background(GeometryReader { geometry in
                        Color.clear.preference(key: ReasoningHeight.self, value: geometry.size.height)
                    }.allowsHitTesting(false))
            }
            .frame(height: min(180, max(28, contentHeight)))
            .background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 6))
            .onPreferenceChange(ReasoningHeight.self) { height in
                if abs(contentHeight - height) > 0.5 { contentHeight = height }
            }
            .accessibilityLabel("Model reasoning")
        }
    }
}
private struct ReasoningHeight: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

// Used by standalone panels; cached task panels bind to AgentStore's bounded UI choices.
struct ThinkingDisclosure {
    private(set) var expanded = false
    private var manuallyChanged = false
    mutating func appear(active: Bool, initiallyExpanded: Bool? = nil) {
        if !manuallyChanged { expanded = initiallyExpanded ?? active }
    }
    mutating func toggle() { manuallyChanged = true; expanded.toggle() }
    mutating func update(active: Bool) { if !manuallyChanged { expanded = active } }
}
