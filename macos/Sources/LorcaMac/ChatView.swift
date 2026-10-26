import SwiftUI

struct ChatView: View {
    @ObservedObject var store: ChatStore
    var compact = false
    var openDashboard: (() -> Void)?
    @State private var followsResponse = true
    @State private var reviewApproval = true
    private var textSize: CGFloat { (compact ? 12 : 14) + store.chatTextZoom }

    var body: some View {
        VStack(spacing: compact ? 10 : 16) {
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        if store.messages.isEmpty {
                            emptyState.frame(minHeight: compact ? 100 : 260)
                        } else {
                            LazyVStack(alignment: .leading, spacing: compact ? 20 : 27) {
                                ForEach(store.messages) { message in
                                    if !compact, message.agentKind == "result", let task = message.agentTaskId {
                                        VStack(alignment: .leading, spacing: 12) {
                                            if !message.attachments.isEmpty {
                                                AttachmentStrip(attachments: message.attachments, storage: store.imageStore, compact: false)
                                            }
                                            AgentActivityView(store: store.agent, taskID: task, savedMessage: message, textSize: textSize)
                                        }
                                    } else { messageView(message) }
                                }
                                Color.clear.frame(height: 1).id("conversation-bottom")
                            }
                            .padding(.vertical, compact ? 8 : 14)
                            .frame(maxWidth: compact ? .infinity : 760)
                            .frame(maxWidth: .infinity)
                        }
                    }
                    .background(ChatScrollObserver { followsResponse = $0 })
                }
                .onAppear { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
                .onChange(of: store.library.selectedID) { _, _ in
                    followsResponse = true
                    proxy.scrollTo("conversation-bottom", anchor: .bottom)
                }
                .onChange(of: store.chatTextZoom) { _, _ in
                    if followsResponse { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
                }
                .onChange(of: store.agent.latest) { _, current in
                    if current?.conversationId == store.library.selectedID, followsResponse { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
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

            if store.jobs.running {
                HStack {
                    Text("Pause Job Apps research before chatting.").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button("Pause research") { store.jobs.pause() }.controlSize(.small)
                }
            }
            if compact {
                AgentMiniStatus(store: store.agent) {
                    if let id = store.agent.active?.conversationId { store.openConversation(id) }
                    openDashboard?()
                }
            } else if let active = store.agent.active, let id = active.conversationId, id != store.library.selectedID {
                Button("Agent working in another chat · Open") {
                    store.openConversation(id)
                }.buttonStyle(.plain).font(.caption).foregroundStyle(Color.lorcaBlue)
            }
            if !compact, let active=store.agent.active, active.conversationId != store.library.selectedID, let approval=active.approval {
                DisclosureGroup("Approval required · Review action", isExpanded:$reviewApproval) {
                    ScrollView {AgentApprovalView(store:store.agent,task:active,approval:approval)}
                        .frame(maxHeight:220)
                }.font(.caption).padding(10)
            }
            if !compact, let error = store.agent.error {
                errorBanner(error)
            }
            if let error = store.error ?? store.modelSession.error ?? store.library.error { errorBanner(error) }
            ContextBar(usage: store.displayUsage, compact: compact,processing:store.agent.active?.generating==true)
            if let model = store.modelSession.loadingModel, let started = store.modelSession.loadingStarted {
                ModelLoadingBar(model: model, started: started, compact: compact)
            }
            ChatComposer(store: store, compact: compact)
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

    @ViewBuilder private func messageView(_ message: ChatMessage) -> some View {
        if !compact && message.role == .user {
            HStack(alignment: .top, spacing: 0) {
                Spacer(minLength: 28)
                UserMessageLayout {
                    messageContent(message)
                        .padding(.horizontal, 14).padding(.vertical, 11)
                        .background(.primary.opacity(0.065), in: RoundedRectangle(cornerRadius: 16))
                }
            }.frame(maxWidth: .infinity, alignment: .trailing)
        } else { messageContent(message).frame(maxWidth: .infinity, alignment: .leading) }
    }

    private func messageContent(_ message: ChatMessage) -> some View {
        VStack(alignment: .leading, spacing: compact ? 7 : 10) {
            if compact {
                Text(message.role == .user ? "YOU" : "LORCA")
                    .font(.system(size: 9, weight: .bold)).tracking(1.2)
                    .foregroundStyle(message.role == .user ? Color.secondary : .lorcaBlue)
            }
            if !message.attachments.isEmpty {
                AttachmentStrip(attachments: message.attachments, storage: store.imageStore, compact: compact)
            }
            if message.role == .assistant && message.content.isEmpty && (store.isGenerating || store.currentAgentActive) {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.mini)
                    Text("Thinking…").font(.system(size: 12)).foregroundStyle(.secondary)
                }
            } else {
                MarkdownText(text: message.content, size: textSize).textSelection(.enabled)
            }
        }
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
            Button { store.library.error = nil; store.error = nil; store.modelSession.clearError() } label: { Image(systemName: "xmark") }
                .buttonStyle(.plain).help("Dismiss error")
        }
        .padding(11)
        .background(.orange.opacity(0.055), in: RoundedRectangle(cornerRadius: 10))
    }
}

struct ConnectionBadge: View {
    @ObservedObject var store: ChatStore
    var body: some View {
        HStack(spacing: 7) {
            if store.agent.active != nil || store.isGenerating || store.isChecking || store.modelSession.isLoading || store.modelSession.isClosing {
                ProgressView().controlSize(.mini)
            } else {
                Circle().fill(store.isConnected ? .green : .orange).frame(width: 6, height: 6)
            }
            Text(store.status).font(.system(size: 11))
        }
        .foregroundStyle(.secondary)
    }
}

// Measure the existing Markdown/attachment content at its ideal width, then wrap
// within a readable maximum. Short prompts keep their natural bubble width.
private struct UserMessageLayout: Layout {
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let content = subviews.first else { return .zero }
        let limit = min(560, max(1, proposal.width ?? 560))
        let ideal = content.sizeThatFits(.unspecified).width
        return content.sizeThatFits(ProposedViewSize(width: min(limit, ideal), height: nil))
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
    }
}
