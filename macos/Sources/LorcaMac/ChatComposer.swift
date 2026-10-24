import SwiftUI
import UniformTypeIdentifiers

struct ChatComposer: View {
    @ObservedObject var store: ChatStore
    var compact = false
    @State private var choosingImages = false
    @State private var dropTarget = false

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
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
                    send: store.send, attach: store.attachImages)
            }
            HStack(spacing: 10) {
                Button { choosingImages = true } label: { Image(systemName: "plus").font(.system(size: 16)) }
                    .help("Attach images (up to four)").accessibilityLabel("Attach images")
                    .disabled(store.isImportingImages || store.draftImages.count >= AttachmentStore.maxImages)
                if store.isImportingImages { ProgressView().controlSize(.mini).accessibilityLabel("Importing images") }
                Spacer(minLength: 0)
                if store.isGenerating {
                    Button { store.stop() } label: {
                        Image(systemName: "stop.circle.fill").font(.system(size: 25)).foregroundStyle(Color.lorcaBlue)
                    }
                    .help("Stop response (Esc)").keyboardShortcut(.escape, modifiers: [])
                    .accessibilityLabel("Stop response")
                } else {
                    Button { store.send() } label: {
                        Image(systemName: "arrow.up.circle.fill").font(.system(size: 25))
                            .foregroundStyle(store.canSend ? Color.lorcaBlue : .secondary.opacity(0.45))
                    }
                    .disabled(!store.canSend).keyboardShortcut(.return, modifiers: .command)
                    .help("Send (⌘Return)").accessibilityLabel("Send message")
                }
            }
        }
        .buttonStyle(.plain).padding(compact ? 12 : 16)
        .background(.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: compact ? 12 : 16))
        .overlay(RoundedRectangle(cornerRadius: compact ? 12 : 16)
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
