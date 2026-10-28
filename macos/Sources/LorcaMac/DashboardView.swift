import SwiftUI

enum SidebarPage: String, CaseIterable, Identifiable {
    case overview = "Overview", chats = "Chats", tasks = "Job Apps"
    case pipelines = "Pipelines", scheduled = "Scheduled", settings = "Settings"
    var id: String { rawValue }
    var symbol: String {
        switch self {
        case .overview: return "square.grid.2x2"
        case .chats: return "bubble.left.and.bubble.right"
        case .tasks: return "briefcase"
        case .pipelines: return "square.stack.3d.up"
        case .scheduled: return "calendar.badge.clock"
        case .settings: return "slider.horizontal.3"
        }
    }
}

extension Color {
    static let lorcaBlue = Color(nsColor: .systemBlue)
}

struct DashboardView: View {
    @ObservedObject var store: ChatStore
    @ObservedObject var windows: WindowManager
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: 0) {
            if windows.sidebarVisible {
                sidebar
                    .frame(width: 190)
                    .background(VisualEffect(material: .sidebar))
                    .overlay(alignment: .trailing) {
                        Rectangle().fill(Color(nsColor: .separatorColor)).frame(width: 1)
                    }
            }
            VStack(spacing: 0) {
                header
                if windows.page == .chats {
                    ChatView(store: store)
                        .padding(.horizontal, 32).padding(.bottom, 24)
                } else if windows.page == .tasks {
                    JobAppsView(store: store.jobs, chat: store)
                } else if windows.page == .settings {
                    settings
                } else {
                    placeholder
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background {
                VisualEffect(material: .underWindowBackground)
                (scheme == .dark ? Color.black.opacity(0.20) : Color.white.opacity(0.38))
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .tint(.lorcaBlue)
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 9) {
                Text(LorcaAsset.emoji).font(.system(size: 27))
                Text("lorca").font(.system(size: 25, weight: .semibold, design: .rounded))
            }
            .padding(.bottom, 18)
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(SidebarPage.allCases.filter { $0 != .settings }) { page in navigation(page) }
                    if windows.page == .chats { ConversationHistory(store: store) }
                }
            }
            .scrollIndicators(.hidden)
            Spacer(minLength: 12)
            MemoryPressureView()
            navigation(.settings)
            Divider().padding(.vertical, 17)
            HStack(spacing: 8) {
                Circle().fill(store.isConnected ? Color.green : Color.orange).frame(width: 6, height: 6)
                Text("Ollama · Local").font(.system(size: 11)).foregroundStyle(.secondary)
                Spacer()
                Button { Task { await store.refreshConnection() } } label: {
                    Image(systemName: "arrow.clockwise").font(.system(size: 11))
                }
                .buttonStyle(.plain).help("Reconnect to Ollama").disabled(store.isChecking)
            }
            .padding(.horizontal, 9)
        }
        .padding(.horizontal, 18).padding(.top, 14).padding(.bottom, 24)
    }

    private func navigation(_ page: SidebarPage) -> some View {
        Button { windows.page = page } label: {
            HStack(spacing: 12) {
                Image(systemName: page.symbol).font(.system(size: 15)).frame(width: 20)
                Text(page.rawValue).font(.system(size: 13, weight: windows.page == page ? .semibold : .regular))
                Spacer()
            }
            .foregroundStyle(windows.page == page ? .primary : .secondary)
            .padding(.horizontal, 12).padding(.vertical, 12)
            .background(windows.page == page ? Color.primary.opacity(0.075) : .clear, in: RoundedRectangle(cornerRadius: 9))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.bottom, 4)
        .accessibilityIdentifier("sidebar-\(page.id)")
    }

    private var header: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 7) {
                Text(windows.page.rawValue).font(.system(size: 27, weight: .semibold))
                Text(windows.page == .chats ? "Your orca, ready when you are." : "Your workspace, Lalith.")
                    .font(.system(size: 12)).foregroundStyle(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 14) {
                miniToggle
                if windows.page == .chats { ConnectionBadge(store: store) }
            }
        }
        .padding(.horizontal, 32).padding(.top, 14).padding(.bottom, 27)
    }

    private var miniToggle: some View {
        Button { windows.toggleMini() } label: {
            Text("Mini").font(.system(size: 10, weight: .medium))
                .padding(.horizontal, 8).padding(.vertical, 6)
        }
        .help(windows.miniVisible ? "Hide floating Mini" : "Show floating Mini")
        .buttonStyle(.plain).foregroundStyle(.secondary)
        .disabled(windows.isTransitioning)
    }

    private var placeholder: some View {
        VStack(spacing: 16) {
            Image(systemName: windows.page.symbol).font(.system(size: 30, weight: .light)).foregroundStyle(.tertiary)
            Text("Needs to be built").font(.system(size: 22, weight: .medium))
            Text(windows.page.rawValue).font(.system(size: 12)).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(.bottom, 95)
    }

    private var settings: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                settingsCard("APPEARANCE") {
                    Picker("Theme", selection: $store.appearance) {
                        ForEach(Appearance.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented).labelsHidden().frame(maxWidth: 320)
                }
                settingsCard("LOCAL MODEL") {
                    HStack {
                        ModelPicker(store: store)
                        Spacer()
                        Button("Refresh models") { Task { await store.refreshConnection() } }
                            .disabled(store.isChecking)
                    }
                    Text(verbatim: "http://127.0.0.1:11434").font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                    ConnectionBadge(store: store)
                    if let error = store.error { Text(error).font(.system(size: 12)).foregroundStyle(.secondary) }
                }
                settingsCard("MINI") {
                    Toggle("Show the floating companion", isOn: Binding(
                        get: { windows.miniVisible },
                        set: { if $0 != windows.miniVisible { windows.toggleMini() } }
                    ))
                    .toggleStyle(.switch)
                    Text("Click to chat. Drag to move. Mini works even with the dashboard closed.")
                        .font(.system(size: 12)).foregroundStyle(.secondary)
                }
                Text("Chats and attached images are saved locally in Application Support/lorca.")
                    .font(.system(size: 11)).foregroundStyle(.tertiary)
            }
            .frame(maxWidth: 650, alignment: .leading)
            .padding(.horizontal, 32).padding(.bottom, 32)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func settingsCard<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 15) {
            Text(title).font(.system(size: 10, weight: .semibold)).tracking(1.1).foregroundStyle(.secondary)
            content()
        }
        .padding(22).frame(maxWidth: .infinity, alignment: .leading)
        .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.primary.opacity(0.06)))
    }
}
