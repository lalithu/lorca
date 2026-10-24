import SwiftUI

struct ConversationHistory: View {
    @ObservedObject var store: ChatStore
    @State private var renameID: UUID?
    @State private var title = ""
    @State private var deleting: Conversation?

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Button { store.newChat() } label: {
                Label("New chat", systemImage: "square.and.pencil")
                    .font(.system(size: 12)).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 8)
            }
            .buttonStyle(.plain).accessibilityIdentifier("history-new-chat")
            Text("RECENTS").font(.system(size: 9, weight: .semibold)).tracking(1)
                .foregroundStyle(.tertiary).padding(.top, 9).padding(.bottom, 5)
            ForEach(store.library.recent) { conversation in
                HStack(spacing: 2) {
                    Button { store.openConversation(conversation.id) } label: {
                        Text(conversation.title).font(.system(size: 11))
                            .lineLimit(1).truncationMode(.tail)
                            .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain).help(conversation.title)
                    .accessibilityLabel("Open chat: \(conversation.title)")
                    .accessibilityIdentifier("conversation-\(conversation.id)")
                    Menu { actions(conversation) } label: { Image(systemName: "ellipsis").font(.system(size: 11)) }
                        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                        .accessibilityLabel("Chat actions: \(conversation.title)")
                }
                .padding(.horizontal, 8).padding(.vertical, 7)
                .background(store.library.selectedID == conversation.id ? Color.primary.opacity(0.075) : .clear,
                    in: RoundedRectangle(cornerRadius: 7))
                .contextMenu { actions(conversation) }
            }
        }
        .padding(.horizontal, 5).padding(.top, 10)
        .alert("Rename chat", isPresented: Binding(get: { renameID != nil }, set: { if !$0 { renameID = nil } })) {
            TextField("Chat name", text: $title)
            Button("Save") { if let id = renameID { store.library.rename(id, to: title) }; renameID = nil }
            Button("Cancel", role: .cancel) { renameID = nil }
        }
        .confirmationDialog("Delete ‘\(deleting?.title ?? "chat")’?", isPresented: Binding(
            get: { deleting != nil }, set: { if !$0 { deleting = nil } }), titleVisibility: .visible) {
            Button("Delete chat", role: .destructive) {
                if let conversation = deleting { store.deleteConversation(conversation.id) }
                deleting = nil
            }
            Button("Cancel", role: .cancel) { deleting = nil }
        } message: { Text("Its messages and unused attached images will be removed from this Mac.") }
    }

    @ViewBuilder private func actions(_ conversation: Conversation) -> some View {
        Button("Rename…") { title = conversation.title; renameID = conversation.id }
        Button("Delete…", role: .destructive) { deleting = conversation }
    }
}

struct ConversationMenu: View {
    @ObservedObject var store: ChatStore
    var body: some View {
        Menu {
            Button("New chat") { store.newChat() }
            Divider()
            ForEach(store.library.recent) { conversation in
                Button { store.openConversation(conversation.id) } label: {
                    if conversation.id == store.library.selectedID { Label(conversation.title, systemImage: "checkmark") }
                    else { Text(conversation.title) }
                }
            }
        } label: { Image(systemName: "clock.arrow.circlepath") }
        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        .accessibilityLabel("Conversation history").help(store.library.current.title)
    }
}
