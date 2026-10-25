import SwiftUI
import AppKit

struct AgentComposerControls: View {
    @ObservedObject var store: ChatStore
    var body: some View {
        HStack(spacing: 12) {
            Menu {
                Button { store.agentMode = false } label: { Label("Chat", systemImage: store.agentMode ? "bubble.left" : "checkmark") }
                Button { store.agentMode = true } label: { Label("Agent", systemImage: store.agentMode ? "checkmark" : "terminal") }
            } label: {
                Label(store.agentMode ? "Agent" : "Chat", systemImage: store.agentMode ? "terminal" : "bubble.left")
                    .font(.system(size: 11, weight: .medium))
            }
            .menuStyle(.borderlessButton).fixedSize().accessibilityIdentifier("chat-execution-mode")
            if store.agentMode {
                Button(action: chooseWorkspace) {
                    Label(store.agentWorkspace?.lastPathComponent ?? "Choose workspace", systemImage: "folder")
                        .font(.system(size: 11)).lineLimit(1).truncationMode(.middle)
                }
                .buttonStyle(.plain).foregroundStyle(store.agentWorkspace == nil ? Color.lorcaBlue : .secondary)
                .help(store.agentWorkspace?.path ?? "Select the folder the Rust agent may work in")
                .accessibilityIdentifier("agent-workspace")
                Spacer(minLength: 0)
                Text("Actions require approval").font(.system(size: 10)).foregroundStyle(.tertiary)
            } else { Spacer(minLength: 0) }
        }
    }
    private func chooseWorkspace() {
        let panel = NSOpenPanel()
        panel.title = "Choose an agent workspace"
        panel.prompt = "Use Workspace"
        panel.canChooseFiles = false; panel.canChooseDirectories = true; panel.allowsMultipleSelection = false
        guard let window = NSApp.keyWindow else { return }
        let conversation = store.library.selectedID
        panel.beginSheetModal(for: window) { response in
            if response == .OK, let url = panel.url { store.agent.workspaces[conversation] = url }
        }
    }
}
