import SwiftUI
import UniformTypeIdentifiers

struct ChatComposer: View {
    @ObservedObject var store: ChatStore
    var compact = false
    @State private var choosingImages = false
    @State private var dropTarget = false

    private var canSend: Bool { store.canSend }
    private func send() { store.send() }

    var body: some View {
        VStack(alignment: .leading, spacing: compact ? 14 : 20) {
            if !compact { AgentComposerControls(store: store) }
            if !store.draftImages.isEmpty {
                AttachmentStrip(attachments: store.draftImages, storage: store.imageStore, compact: compact,
                    remove: store.library.removeDraftImage)
            }
            ZStack(alignment: .topLeading) {
                if store.draft.isEmpty {
                    Text("Message lorca…").foregroundStyle(.tertiary)
                        .font(.system(size: (compact ? 12 : 14) + store.chatTextZoom)).allowsHitTesting(false)
                }
                ChatInput(text: $store.draft, size: (compact ? 12 : 14) + store.chatTextZoom,
                    maxLines: compact ? 3 : 5,
                    identifier: compact ? "mini-message-input" : "dashboard-message-input",
                    send: send, attach: store.attachImages)
                    .disabled(!store.ready)
            }
            .frame(minHeight: compact ? 34 : 50, alignment: .topLeading)
            HStack(spacing: compact ? 12 : 16) {
                Button { choosingImages = true } label: { Image(systemName: "plus").font(.system(size: 16)) }
                    .help("Attach images (up to four)").accessibilityLabel("Attach images")
                    .disabled(!store.ready || store.isImportingImages || store.draftImages.count >= AttachmentStore.maxImages)
                if store.isImportingImages { ProgressView().controlSize(.mini).accessibilityLabel("Importing images") }
                Spacer(minLength: 0)
                ModelPicker(store: store, compact: compact)
                if store.isGenerating || store.currentAgentActive {
                    Button { store.stop() } label: {
                        Image(systemName: "stop.fill").font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(.white).frame(width: 32, height: 32).background(Color.lorcaBlue, in: Circle())
                    }
                    .help("Stop response (Esc)").keyboardShortcut(.escape, modifiers: [])
                    .accessibilityLabel("Stop response")
                } else {
                    Button { send() } label: {
                        Image(systemName: "arrow.up").font(.system(size: 17, weight: .semibold))
                            .foregroundStyle(canSend ? .white : .secondary.opacity(0.55))
                            .frame(width: 32, height: 32)
                            .background(canSend ? Color.lorcaBlue : Color.primary.opacity(0.10), in: Circle())
                    }
                    .disabled(!canSend).keyboardShortcut(.return, modifiers: .command)
                    .help("Send (⌘Return)").accessibilityLabel("Send message")
                }
            }
        }
        .buttonStyle(.plain).padding(compact ? 14 : 18)
        .background(.primary.opacity(0.075), in: RoundedRectangle(cornerRadius: compact ? 20 : 26))
        .overlay(RoundedRectangle(cornerRadius: compact ? 20 : 26)
            .strokeBorder(dropTarget ? Color.lorcaBlue : Color.primary.opacity(0.10)))
        .onDrop(of: [UTType.fileURL, .image], isTargeted: $dropTarget) { providers in
            guard !store.isImportingImages else { return false }
            store.attachProviders(providers)
            return true
        }
        .fileImporter(isPresented: $choosingImages, allowedContentTypes: [.image], allowsMultipleSelection: true) { result in
            switch result {
            case .success(let urls): store.attachFiles(urls)
            case .failure(let error):
                if (error as NSError).code != NSUserCancelledError { store.error = error.localizedDescription }
            }
        }
    }
}
