import SwiftUI

struct ChatView: View {
    @ObservedObject var store: ChatStore
    var compact = false
    @State private var followsResponse = true
    private var textSize: CGFloat { (compact ? 12 : 14) + store.chatTextZoom }

    var body: some View {
        VStack(spacing: compact ? 10 : 16) {
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        if store.messages.isEmpty {
                            emptyState.frame(minHeight: compact ? 100 : 260)
                        } else {
                            VStack(alignment: .leading, spacing: compact ? 20 : 27) {
                                ForEach(store.messages) { message in
                                    messageView(message)
                                }
                                Color.clear.frame(height: 1).id("conversation-bottom")
                            }
                            .padding(.vertical, compact ? 8 : 14)
                        }
                    }
                    .background(ChatScrollObserver { followsResponse = $0 })
                }
                .onAppear { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
                .onChange(of: store.chatTextZoom) { _, _ in
                    if followsResponse { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
                }
                .onChange(of: store.messages) { previous, current in
                    // A submitted prompt starts following again; streamed tokens respect manual scrolling.
                    if previous.last(where: { $0.role == .user })?.id != current.last(where: { $0.role == .user })?.id {
                        followsResponse = true
                    }
                    if followsResponse { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            if let error = store.error ?? store.modelSession.error { errorBanner(error) }
            ContextBar(usage: store.usage, compact: compact)
            if let model = store.modelSession.loadingModel, let started = store.modelSession.loadingStarted {
                ModelLoadingBar(model: model, started: started, compact: compact)
            }
            composer
            HStack {
                ModelPicker(store: store, compact: compact)
                Spacer(minLength: 3)
                if !compact {
                    Text("Local to your Mac").font(.system(size: 10)).foregroundStyle(.tertiary)
                }
            }
        }
    }

    private var emptyState: some View {
        VStack(spacing: compact ? 9 : 16) {
            if !compact { Text(LorcaAsset.emoji).font(.system(size: 50)).padding(.bottom, 4) }
            Text(compact ? "Here for you, Lalith." : "Hey Lalith.")
                .font(.system(size: compact ? 17 : 23, weight: .medium))
            Text(compact ? "What’s on your mind?" : "What are we working on today?")
                .font(.system(size: compact ? 11 : 13)).foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func messageView(_ message: ChatMessage) -> some View {
        VStack(alignment: .leading, spacing: compact ? 7 : 10) {
            Text(message.role == .user ? "YOU" : "LORCA")
                .font(.system(size: 9, weight: .bold)).tracking(1.2)
                .foregroundStyle(message.role == .user ? Color.secondary : .lorcaBlue)
            if message.content.isEmpty && store.isGenerating {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.mini)
                    Text("Thinking…").font(.system(size: 12)).foregroundStyle(.secondary)
                }
            } else {
                MarkdownText(text: message.content, size: textSize)
                    .textSelection(.enabled)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: 12) {
            TextField("Message lorca…", text: $store.draft, axis: .vertical)
                .font(.system(size: textSize))
                .textFieldStyle(.plain).lineLimit(1...5)
                .onSubmit { store.send() }
                .accessibilityIdentifier(compact ? "mini-message-input" : "dashboard-message-input")
            if store.isGenerating {
                Button { store.stop() } label: {
                    Image(systemName: "stop.circle.fill").font(.system(size: 25)).foregroundStyle(Color.lorcaBlue)
                }
                .help("Stop response (Esc)").keyboardShortcut(.escape, modifiers: [])
                .accessibilityLabel("Stop response")
            } else {
                Button { store.send() } label: {
                    Image(systemName: "arrow.up.circle.fill").font(.system(size: 25))
                        .foregroundStyle(store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? Color.secondary.opacity(0.45) : .lorcaBlue)
                }
                .disabled(store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || store.modelSession.isLoading)
                .keyboardShortcut(.return, modifiers: .command)
                .help("Send (⌘Return)").accessibilityLabel("Send message")
            }
        }
        .buttonStyle(.plain)
        .padding(compact ? 12 : 16)
        .background(.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: compact ? 12 : 16))
        .overlay(RoundedRectangle(cornerRadius: compact ? 12 : 16).strokeBorder(.primary.opacity(0.10)))
    }

    private func errorBanner(_ error: String) -> some View {
        HStack(alignment: .top, spacing: 9) {
            Image(systemName: "wifi.exclamationmark").foregroundStyle(.orange)
            Text(error).font(.system(size: compact ? 10 : 12)).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button { Task { await store.refreshConnection() } } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.plain).help("Reconnect").disabled(store.isChecking)
        }
        .padding(11)
        .background(.orange.opacity(0.055), in: RoundedRectangle(cornerRadius: 10))
    }
}

struct ModelPicker: View {
    @ObservedObject var store: ChatStore
    var compact = false
    var body: some View {
        Picker("Model", selection: $store.selectedModel) {
            ForEach(store.modelChoices, id: \.self) { Text($0).tag($0) }
        }
        .labelsHidden().pickerStyle(.menu).controlSize(.small)
        .frame(maxWidth: compact ? 220 : 270, alignment: .leading)
        .disabled(store.isGenerating || store.isChecking || store.modelSession.isLoading)
        .help("Choose an installed Ollama model")
    }
}

struct ConnectionBadge: View {
    @ObservedObject var store: ChatStore
    var body: some View {
        HStack(spacing: 7) {
            if store.isGenerating || store.isChecking || store.modelSession.isLoading || store.modelSession.isClosing {
                ProgressView().controlSize(.mini)
            } else {
                Circle().fill(store.isConnected ? .green : .orange).frame(width: 6, height: 6)
            }
            Text(store.status).font(.system(size: 11))
        }
        .foregroundStyle(.secondary)
    }
}
