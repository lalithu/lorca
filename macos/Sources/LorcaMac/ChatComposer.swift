import SwiftUI
import UniformTypeIdentifiers

struct ChatComposer: View {
    @ObservedObject var store: ChatStore
    var compact = false
    @State private var choosingImages = false
    @State private var dropTarget = false
    @State private var statusMessage: String?

    private var canSend: Bool { store.canSend && store.draft.trimmingCharacters(in:.whitespacesAndNewlines) != "Correction:" }
    private func send() {if canSend {store.send()}}
    private func requestStatus() {
        Task {
            do {
                let data=try await store.backend.command(["op":"agent_status"])
                let value=try JSONDecoder().decode(AgentJSON.self,from:data)
                if value["taskId"].string == store.agent.active?.id {statusMessage="Status snapshot: \(value["status"].string ?? "unknown") — \(String((value["task"].string ?? "").prefix(200)))"}
            } catch {store.error=error.localizedDescription}
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if store.currentAgentActive {
                HStack(spacing:12) {
                    Button("Status",action:requestStatus).disabled(store.agent.submitting).fixedSize()
                    Button("Correct task") {if !store.canSteerActive {store.draft="Correction: " + store.draft}}.fixedSize()
                    Spacer()
                    Text("Thinking changes apply to the next task").font(.system(size:10)).foregroundStyle(.secondary)
                }.font(.system(size:11)).accessibilityIdentifier("active-task-controls")
            }
            if store.currentAgentActive,let message=statusMessage {Text(message).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)}
            if store.currentAgentActive && !store.draft.isEmpty && !store.canSteerActive {Text("Start a correction with ‘Correction:’ or ‘Actually…’, or request Status.").font(.caption).foregroundStyle(.secondary)}
            if !store.draftImages.isEmpty {
                AttachmentStrip(attachments: store.draftImages, storage: store.imageStore, compact: compact,
                    remove: store.library.removeDraftImage)
            }
            ZStack(alignment: .topLeading) {
                if store.draft.isEmpty {
                    Text("Message lorca…").foregroundStyle(.tertiary)
                        .font(.system(size: (compact ? 12 : 14))).allowsHitTesting(false)
                }
                ChatInput(text: $store.draft, size: (compact ? 12 : 14),
                    maxLines: compact ? 3 : 5,
                    identifier: compact ? "mini-message-input" : "dashboard-message-input",
                    send: send, attach: store.attachImages)
                    .disabled(!store.ready)
            }
            .frame(minHeight: compact ? 30 : 38, alignment: .topLeading)
            HStack(spacing: compact ? 12 : 16) {
                Button { choosingImages = true } label: { Image(systemName: "plus").font(.system(size: 16)) }
                    .help("Attach images (up to four)").accessibilityLabel("Attach images")
                    .disabled(!store.ready || store.isImportingImages || store.draftImages.count >= AttachmentStore.maxImages)
                if store.isImportingImages { ProgressView().controlSize(.mini).accessibilityLabel("Importing images") }
                AgentComposerControls(store: store)
                Spacer(minLength: 0)
                ModelPicker(store: store, compact: compact, embeddedInComposer: true)
                    .layoutPriority(-1)
                if store.isGenerating || store.currentAgentActive {
                    Button { store.stop() } label: {
                        Image(systemName: "stop.fill").font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(.white).frame(width: 32, height: 32).background(Color.lorcaBlue, in: Circle())
                    }
                    .help("Stop response (Esc)").keyboardShortcut(.escape, modifiers: [])
                    .accessibilityLabel("Stop response")
                }
                if !(store.isGenerating || store.currentAgentActive) || store.canSteerActive {
                    Button { send() } label: {
                        Image(systemName: "arrow.up").font(.system(size: 17, weight: .semibold))
                            .foregroundStyle(canSend ? Color.lorcaBlue : Color.secondary.opacity(0.65))
                            .frame(width: 32, height: 32)
                            .background(canSend ? Color.lorcaBlue.opacity(0.17) : Color.primary.opacity(0.06), in: Circle())
                    }
                    .disabled(!canSend).keyboardShortcut(.return, modifiers: .command)
                    .help("Send (⌘Return)").accessibilityLabel("Send message")
                }
            }
        }
        .buttonStyle(.plain).padding(compact ? 14 : 18)
        .chatGlass(radius: compact ? 20 : 26)
        .overlay {
            if dropTarget {
                RoundedRectangle(cornerRadius: compact ? 20 : 26)
                    .strokeBorder(Color.lorcaBlue).allowsHitTesting(false)
            }
        }
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
