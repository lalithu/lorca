import SwiftUI

struct AgentComposerControls: View {
    @ObservedObject var store: ChatStore
    @AppStorage("showModelReasoning") private var showModelReasoning = true
    var body: some View {
        ViewThatFits(in: .horizontal) {
            effort.fixedSize()
            effort.labelStyle(.iconOnly)
        }
    }
    private var effort: some View {
        Toggle(isOn:Binding(get:{store.thinkingEffort != .off},set:{store.thinkingEffort=$0 ? .nominal:.off})) {
            ThinkingControlLabel(enabled: store.thinkingEffort != .off, nextTask: store.currentAgentActive)
        }
        .toggleStyle(.button).controlSize(.small)
        .accessibilityLabel("Model thinking").accessibilityIdentifier("thinking-toggle")
        .help(store.currentAgentActive ? "Applies to the next task. Current inference and task remain unchanged." : "Show actual local-model thinking when supported. Reasoning is retained only in this session.")
        .overlay(alignment: .trailing) {
            Menu {
                Button("Off") { store.thinkingEffort = .off }
                Button("Standard") { store.thinkingEffort = .nominal }
                Button("High") { store.thinkingEffort = .high }
                Divider()
                Toggle("Show emitted reasoning", isOn: $showModelReasoning)
                Text("Options are model-dependent; no fixed thinking-token cap is promised.")
                    .font(.caption)
            } label: {
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(7)
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .offset(x: 25)
            .accessibilityIdentifier("thinking-effort-menu")
            .help("Choose the reasoning mode for the next task.")
        }
        .padding(.trailing, 26)
    }

}

// The selected label/icon change immediately; the toggle's behavior is unchanged.
struct ThinkingControlLabel: View {
    let enabled: Bool
    var nextTask = false
    static let enabledColor = Color(red: 34 / 255, green: 197 / 255, blue: 94 / 255)
    var body: some View {
        HStack(spacing: 4) {
            if nextTask { Text("Next task:").foregroundStyle(.secondary) }
            Label(enabled ? "Thinking ON" : "Thinking OFF", systemImage: "brain")
                .foregroundStyle(enabled ? Self.enabledColor : Color.primary)
        }
    }
}
