import SwiftUI

// A fixed-size panel anchored to the separate, fixed-size pet window.
struct MiniView: View {
    @ObservedObject var store: ChatStore
    @ObservedObject var windows: WindowManager

    var body: some View {
        VStack(spacing: 10) {
            if windows.miniChatBelowPet { controls }
            chatCard
            if !windows.miniChatBelowPet { controls }
        }
        .padding(5)
        .frame(width: 384, height: 400)
        .preferredColorScheme(.dark)
        .tint(.lorcaBlue)
    }

    private var controls: some View {
        HStack(spacing: 0) {
            Button { windows.newChat() } label: { Image(systemName: "square.and.pencil") }
                .help("New chat").accessibilityLabel("New chat")
            Divider().frame(height: 16)
            Button { windows.page = .chats; windows.showDashboard() } label: {
                Image(systemName: "macwindow")
            }
            .help("Open dashboard").accessibilityLabel("Open dashboard")
            Divider().frame(height: 16)
            Button { windows.toggleMiniExpansion() } label: {
                Image(systemName: windows.miniChatBelowPet ? "chevron.up" : "chevron.down")
            }
            .help("Collapse Mini").accessibilityLabel("Collapse Mini")
        }
        .buttonStyle(MiniControlStyle())
        .frame(width: 146, height: 36)
        .background {
            VisualEffect(material: .hudWindow)
            Color.white.opacity(0.055)
        }
        .clipShape(Capsule())
        .overlay(Capsule().strokeBorder(.white.opacity(0.16)))
    }

    private var chatCard: some View {
        VStack(spacing: 14) {
            HStack {
                Text("lorca").font(.system(size: 14, weight: .semibold, design: .rounded))
                Spacer()
                ConnectionBadge(store: store)
            }
            ChatView(store: store, compact: true)
        }
        .padding(18)
        .frame(width: 374, height: 343)
        .background {
            VisualEffect(material: .hudWindow)
            Color.black.opacity(0.35)
        }
        .clipShape(RoundedRectangle(cornerRadius: 24))
        .overlay(RoundedRectangle(cornerRadius: 24).strokeBorder(.white.opacity(0.12)))
    }
}

private struct MiniControlStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(.white.opacity(configuration.isPressed ? 0.5 : 0.9))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .contentShape(Rectangle())
    }
}
