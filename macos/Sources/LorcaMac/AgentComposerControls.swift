import SwiftUI
import AppKit

struct AgentComposerControls: View {
    @ObservedObject var store: ChatStore
    var body: some View {
        HStack(spacing: 12) {
            Menu {
                Button("Choose Workspace…", action: chooseWorkspace)
            } label: {
                Label(store.agentWorkspace?.lastPathComponent ?? "Choose workspace",systemImage:"folder")
                    .font(.system(size:11)).lineLimit(1).truncationMode(.middle)
            }
            .menuStyle(.borderlessButton).fixedSize()
            .help(store.agentWorkspace?.path ?? "Select a folder to make file and command tools available")
            .accessibilityIdentifier("agent-workspace")
            Spacer(minLength:0)
            if store.agentWorkspace != nil {Text("Edits and commands require approval").font(.system(size:10)).foregroundStyle(.tertiary)}
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
