import SwiftUI

struct ModelPicker: View {
    @ObservedObject var store: ChatStore
    var compact = false
    var body: some View {
        Menu {
            ForEach(store.modelChoices, id: \.self) { model in
                Button { store.selectedModel = model } label: {
                    if model == store.selectedModel { Label(model, systemImage: "checkmark") }
                    else { Text(model) }
                }
            }
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
        .disabled(!store.ready || store.isGenerating || store.isChecking || store.modelSession.isLoading)
        .accessibilityLabel("Model").accessibilityValue(store.selectedModel)
        .help("Choose an installed Ollama model")
    }
}
