import SwiftUI

struct ModelPicker: View {
    @ObservedObject var store: ChatStore
    var compact = false
    @State private var confirmingReload = false
    @State private var confirmingUnload = false
    var body: some View {
        Menu {
            ForEach(store.modelChoices, id: \.self) { model in
                Button { store.selectedModel = model } label: {
                    if model == store.selectedModel { Label(model, systemImage: "checkmark") }
                    else { Text(model) }
                }
            }
            Divider()
            Button("Reload model…") { confirmingReload = true }.disabled(!store.canReloadModel)
            Button("Unload model…") { confirmingUnload = true }.disabled(!store.canUnloadModel)
        } label: {
            HStack(spacing: 6) {
                Text(store.selectedModel).lineLimit(1).truncationMode(.middle)
                    .frame(maxWidth: compact ? 150 : 220, alignment: .trailing)
                Image(systemName: "chevron.down").font(.system(size: compact ? 8 : 9, weight: .medium))
            }
            .font(.system(size: compact ? 11 : 12, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.vertical, 5)
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize(horizontal: true, vertical: true)
        .frame(maxWidth: compact ? 180 : 250, alignment: .trailing)
        .disabled(!store.ready || store.isManagingModel || store.isGenerating || store.agent.busy || store.jobs.running || store.isChecking || store.modelSession.isLoading)
        .accessibilityLabel("Model").accessibilityValue(store.selectedModel)
        .help("Choose an installed Ollama model; stop active work before reloading or unloading")
        .confirmationDialog("Unload \(store.modelSession.activeModel ?? store.selectedModel)?", isPresented: $confirmingUnload) {
            Button("Unload model", role: .destructive) { Task { await store.unloadModel() } }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("Ollama shares models with other apps. Unloading can interrupt another app using this model. Your chats are kept; the model loads again when you send a message. Other active lorca sessions prevent unloading.")
        }
        .confirmationDialog("Reload \(store.selectedModel)?", isPresented: $confirmingReload) {
            Button("Reload model", role: .destructive) { Task { await store.reloadModel() } }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("Ollama shares models with other apps. Unloading and reloading can interrupt another app using this model. Your chats are kept. Other active lorca sessions prevent unloading.")
        }
    }
}
