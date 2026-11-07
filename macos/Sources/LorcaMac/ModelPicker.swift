import SwiftUI

struct ModelPicker: View {
    @ObservedObject var store: ChatStore
    var compact = false
    var embeddedInComposer = false
    @State private var confirmingReload = false
    @State private var confirmingUnload = false
    @State private var showingAdvisor = false
    var body: some View {
        Menu {
            if store.modelChoices.isEmpty {
                Text("No models available")
            }
            ForEach(store.modelChoices, id: \.self) { choice in
                Menu {
                    contextOption("Recommended", choice: choice, window: nil)
                    ForEach(choice.contextOptions, id: \.self) { window in
                        contextOption("\(window.formatted()) tokens", choice: choice, window: window)
                    }
                    if choice.contextOptions.isEmpty {
                        Text("Context size unavailable from server").disabled(true)
                    } else if choice.route.provider == "llama_cpp" {
                        Text("Working window within server allocation").disabled(true)
                    }
                } label: {
                    let sameName = store.modelChoices.filter {
                        $0.model == choice.model && $0.route.provider == choice.route.provider
                    }.count > 1
                    let server = sameName ? choice.route.host.replacingOccurrences(of: "http://", with: "") : nil
                    let source = [choice.route.label, server].compactMap { $0 }.joined(separator: " · ")
                    let label = "\(choice.model) · \(source)"
                    if choice.matches(store.modelChoice) { Label(label, systemImage: "checkmark") }
                    else { Text(label) }
                }
            }
            Divider()
            Button("Refresh models") { Task { await store.refreshConnection() } }
            Button("Hardware-aware model advisor…") { showingAdvisor = true }
            Button("Reload model…") { confirmingReload = true }.disabled(!store.canReloadModel)
            Button("Unload model…") { confirmingUnload = true }.disabled(!store.canUnloadModel)
        } label: {
            HStack(spacing: 6) {
                Text(store.modelChoice.map { ResponseDetails.modelName($0.model) } ?? "Choose model").lineLimit(1).truncationMode(.middle)
                    .frame(maxWidth: compact ? 150 : 220, alignment: .trailing)
                Image(systemName: "chevron.down").font(.system(size: compact ? 8 : 9, weight: .medium))
            }
            .font(.system(size: compact ? 11 : 12, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 9).padding(.vertical, 5)
            .background(Color.primary.opacity(embeddedInComposer ? 0 : 0.05), in: RoundedRectangle(cornerRadius: 7))
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize(horizontal: !embeddedInComposer, vertical: true)
        .frame(maxWidth: compact ? 180 : 250, alignment: .trailing)
        .disabled(!store.ready || store.isManagingModel || store.isGenerating || store.agent.busy || store.jobs.running || store.isChecking || store.isSelectingModel || store.modelSession.isLoading)
        .accessibilityLabel("Model").accessibilityValue(store.modelChoice.map { "\($0.model), \($0.route.label)" } ?? "No model selected")
        .help("Choose an available model from Ollama or llama.cpp")
        .confirmationDialog("Unload \(store.modelSession.activeModel ?? store.selectedModel)?", isPresented: $confirmingUnload) {
            Button("Unload model", role: .destructive) { Task { await store.unloadModel() } }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("The local model server may share models with other apps. Unloading can interrupt another client using this model. Your chats are kept; the model loads again when you send a message.")
        }
        .sheet(isPresented: $showingAdvisor) {
            ModelAdviceView(backend: store.backend)
        }
        .confirmationDialog("Reload \(store.selectedModel)?", isPresented: $confirmingReload) {
            Button("Reload model", role: .destructive) { Task { await store.reloadModel() } }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("The local model server may share models with other apps. Reloading can interrupt another client using this model. Your chats are kept.")
        }
    }

    private func contextOption(_ title: String, choice: LocalModel, window: Int?) -> some View {
        Button {
            Task { await store.selectModel(choice, contextWindow: window) }
        } label: {
            if store.contextPreference(for: choice) == window {
                Label(title, systemImage: "checkmark")
            } else {
                Text(title)
            }
        }
    }

}
