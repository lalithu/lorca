import SwiftUI
import AppKit

struct AgentComposerControls: View {
    @ObservedObject var store: ChatStore
    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) { workspace; Spacer(minLength: 0); effort; approval.fixedSize() }
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 12) { workspace; Spacer(minLength: 0); effort }
                approval
            }
        }
    }
    private var workspace: some View {
        Menu {
            Button("Choose Workspace…", action: chooseWorkspace)
        } label: {
            Label(store.agentWorkspace?.lastPathComponent ?? "Choose workspace", systemImage: "folder")
                .font(.system(size: 11)).lineLimit(1).truncationMode(.middle)
        }
        .menuStyle(.borderlessButton).fixedSize()
        .help(store.agentWorkspace?.path ?? "Select a folder to make file and command tools available")
        .accessibilityIdentifier("agent-workspace")
    }
    private var effort: some View {
        Toggle(isOn:Binding(get:{store.thinkingEffort != .off},set:{store.thinkingEffort=$0 ? .nominal:.off})) {
            ThinkingControlLabel(enabled: store.thinkingEffort != .off, nextTask: store.currentAgentActive)
        }
        .toggleStyle(.button).controlSize(.small)
        .accessibilityLabel("Model thinking").accessibilityIdentifier("thinking-toggle")
        .help(store.currentAgentActive ? "Applies to the next task. Current inference and task remain unchanged." : "Show actual local-model thinking when supported. Reasoning is retained only in this session.")
        .contextMenu {
            Picker("Thinking effort",selection:$store.thinkingEffort) {ForEach(ThinkingEffort.allCases) {Text($0.label).tag($0)}}
        }
    }
    @ViewBuilder private var approval: some View {
        if store.agentWorkspace != nil {
            Text("Edits and commands require approval").font(.system(size: 10)).foregroundStyle(.tertiary)
        }
    }
    private func chooseWorkspace() {
        let panel=NSOpenPanel()
        panel.title="Choose a workspace";panel.prompt="Use Workspace"
        panel.canChooseFiles=false;panel.canChooseDirectories=true;panel.allowsMultipleSelection=false
        guard let window=NSApp.keyWindow else{return}
        let conversation=store.library.selectedID
        panel.beginSheetModal(for:window){response in
            if response == .OK,let url=panel.url{store.agent.workspaces[conversation]=url}
        }
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
