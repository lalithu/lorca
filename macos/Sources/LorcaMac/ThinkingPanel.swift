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
                    Image(systemName: "brain").font(.system(size: 12)).foregroundStyle(ChatAccent.reasoning)
                    if block.active { ProgressView().controlSize(.mini) }
                    Text(block.active ? "Reasoning…" : "Reasoning").font(.system(size: 12, weight: .medium))
                    Spacer(minLength: 0)
                }.foregroundStyle(.secondary).contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityLabel(block.label).accessibilityIdentifier("thinking-disclosure-" + block.id)
            if expanded {
                if block.text.isEmpty {
                    Text(block.active ? "Waiting for model thinking…" : block.label).font(.caption).foregroundStyle(.secondary)
                } else { ReasoningText(text: block.text) }
                if block.truncated { Text("Thinking display reached its session limit.").font(.caption).foregroundStyle(.secondary) }
            }
        }
        .onAppear { if selection == nil { disclosure.appear(active: block.active, initiallyExpanded: initiallyExpanded) } }
        .onChange(of: block.state) { _, _ in if selection == nil { disclosure.update(active: block.active) } }
    }
}

struct ReasoningText: View {
    let text: String
    var body: some View {
        MarkdownText(text: text, size: 12, showsCopyButton: false, editorial: true)
            .textSelection(.enabled).foregroundStyle(.secondary)
            .padding(.leading, 14).padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(alignment: .leading) {
                Rectangle().fill(ChatAccent.reasoning.opacity(0.35)).frame(width: 1).allowsHitTesting(false)
            }
            .accessibilityLabel("Model reasoning")
    }
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
