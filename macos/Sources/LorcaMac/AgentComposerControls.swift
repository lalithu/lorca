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
        Picker("Thinking effort", selection: $store.thinkingEffort) {
            ForEach(ThinkingEffort.allCases) { effort in Text(effort.label).tag(effort) }
        }
        .pickerStyle(.segmented).labelsHidden().controlSize(.small).frame(width: 170)
        .accessibilityLabel("Thinking effort")
        .disabled(store.agent.busy)
        .help("Off disables thinking where supported. Nominal and High allow more time and bounded planning; native thinking depends on the model's supported controls.")
        .accessibilityIdentifier("thinking-effort")
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
