import SwiftUI

enum SidebarPage: String, CaseIterable, Identifiable {
    case overview = "Overview", chats = "Chats", tasks = "Job Apps"
    case settings = "Settings"
    var id: String { rawValue }
    var symbol: String {
        switch self {
        case .overview: return "square.grid.2x2"
        case .chats: return "bubble.left.and.bubble.right"
        case .tasks: return "briefcase"
        case .settings: return "slider.horizontal.3"
        }
    }
}

extension Color {
    static let lorcaBlue = Color(nsColor: .systemBlue)
}

enum ShellTypography {
    private static let serifName = ["IowanOldStyle-Roman", "Palatino-Roman", "Georgia"]
        .first { NSFont(name: $0, size: 13) != nil }

    static func serif(_ size: CGFloat) -> Font {
        serifName.map { .custom($0, size: size) } ?? .system(size: size, design: .serif)
    }
}

struct DashboardView: View {
    @ObservedObject var store: ChatStore
    @ObservedObject var windows: WindowManager
    @Environment(\.colorScheme) private var scheme
    @State private var hoveredPage: SidebarPage?

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @AppStorage("dashboardSidebarWidth") private var preferredSidebarWidth = 190.0
    @State private var sidebarDragStart: CGFloat?
    @State private var sidebarDragWidth: CGFloat?
    private let minimumSidebarWidth: CGFloat = 180

    var body: some View {
        GeometryReader { geometry in
            let maximum = max(minimumSidebarWidth, min(320, geometry.size.width - 550))
            let preferred = min(maximum, max(minimumSidebarWidth, preferredSidebarWidth))
            let draggingOrHidden = sidebarDragStart != nil || !windows.sidebarVisible
            let width = min(maximum, (draggingOrHidden ? sidebarDragWidth : nil) ?? preferred)

            DashboardColumns(sidebarWidth: width, occupiedWidth: windows.sidebarVisible ? width : 0) {
                sidebarSurface
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .overlay(alignment: .trailing) {
                        SidebarResizeEdge(
                            onStart: {
                                sidebarDragStart = width
                                sidebarDragWidth = width
                            },
                            onDrag: { distance in
                                guard windows.sidebarVisible, let start = sidebarDragStart else { return }
                                let proposed = start + distance / store.uiZoom.factor
                                sidebarDragWidth = min(maximum, max(minimumSidebarWidth, proposed))
                                windows.setSidebarCollapsePreview(proposed < minimumSidebarWidth)
                            },
                            onEnd: { distance in
                                guard windows.sidebarVisible, let start = sidebarDragStart else { return }
                                defer {
                                    sidebarDragStart = nil
                                    windows.setSidebarCollapsePreview(false)
                                }

                                let proposed = start + distance / store.uiZoom.factor
                                if proposed < minimumSidebarWidth {
                                    sidebarDragWidth = minimumSidebarWidth
                                    windows.setSidebarVisible(false)
                                } else {
                                    let width = min(maximum, proposed)
                                    sidebarDragWidth = width
                                    preferredSidebarWidth = width
                                }
                            }
                        )
                        .frame(width: 8)
                        .offset(x: 4)
                        .accessibilityHidden(true)
                    }
                    .zIndex(1)
                    .allowsHitTesting(windows.sidebarVisible)
                    .accessibilityHidden(!windows.sidebarVisible)
                mainPane
            }
            .clipped()
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: windows.sidebarVisible)
            .onChange(of: windows.sidebarVisible) { _, visible in
                if !visible { sidebarDragStart = nil }
            }
        }
        .background(scheme == .dark ? Color(nsColor: .windowBackgroundColor) : Color(nsColor: .textBackgroundColor))
        .tint(.lorcaBlue)
    }

    private var mainPane: some View {
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
    }

    @ViewBuilder private var sidebarSurface: some View {
        #if compiler(>=6.2)
        if #available(macOS 26.0, *) {
            sidebar
                .glassEffect(.regular, in: .rect(cornerRadius: 20))
                .padding(.leading, 6).padding(.vertical, 6)
        } else {
            legacySidebar
        }
        #else
        legacySidebar
        #endif
    }

    private var legacySidebar: some View {
        sidebar.background(VisualEffect(material: .sidebar))
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 9) {
                Text(LorcaAsset.emoji).font(.system(size: 27))
                Text("lorca").font(ShellTypography.serif(24).weight(.semibold))
            }
            // Align the visible artwork rather than the glyphs' layout bounds.
            .offset(x: -1.75)
            .frame(maxWidth: .infinity)
            .padding(.bottom, 14)
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(SidebarPage.allCases.filter { $0 != .settings }) { page in navigation(page) }
                    if windows.page == .chats { ConversationHistory(store: store) }
                }
            }
            .scrollIndicators(.hidden)
            Spacer(minLength: 12)
            MemoryPressureView(isVisible: windows.sidebarVisible)
            navigation(.settings)
            Divider().padding(.vertical, 10)
            HStack(spacing: 6) {
                Circle().fill(store.isConnected ? Color.green : Color.orange).frame(width: 6, height: 6)
                Text("\(store.providerName) · Local")
                    .font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary)
                    .lineLimit(1).minimumScaleFactor(0.85)
                Spacer(minLength: 0)
                Button { Task { await store.refreshConnection() } } label: {
                    Image(systemName: "arrow.clockwise").font(.system(size: 11))
                }
                .buttonStyle(.plain).help("Reconnect to local model server").disabled(store.isChecking)
            }
            .padding(.horizontal, 5).padding(.vertical, 6)
        }
        .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 16)
    }

    private func navigation(_ page: SidebarPage) -> some View {
        let selected = windows.page == page
        return Button { windows.page = page } label: {
            HStack(spacing: 10) {
                Image(systemName: page.symbol)
                    .font(.system(size: 14, weight: .medium)).frame(width: 19)
                    .foregroundStyle(selected ? Color.lorcaBlue : Color.secondary)
                Text(page.rawValue)
                    .font(.system(size: 12, weight: selected ? .semibold : .regular, design: .monospaced))
                    .foregroundStyle(selected ? .primary : .secondary)
                Spacer()
            }
            .padding(.horizontal, 10).padding(.vertical, 10)
            .background(
                selected ? Color.lorcaBlue.opacity(0.11) : Color.primary.opacity(hoveredPage == page ? 0.05 : 0),
                in: RoundedRectangle(cornerRadius: 8)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            if hovering { hoveredPage = page }
            else if hoveredPage == page { hoveredPage = nil }
        }
        .padding(.bottom, 3)
        .accessibilityIdentifier("sidebar-\(page.id)")
    }

    private var header: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 7) {
                Text(windows.page.rawValue).font(ShellTypography.serif(26).weight(.semibold))
                Text(windows.page == .chats ? "Your orca, ready when you are. 🌸" : "Your workspace, Lalith.")
                    .font(ShellTypography.serif(13)).foregroundStyle(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 10) {
                miniToggle
                if windows.page == .chats { ConnectionBadge(store: store) }
            }
        }
        .padding(.horizontal, 32).padding(.top, 14).padding(.bottom, 21)
    }

    private var miniToggle: some View {
        styledMiniButton
            .help(windows.miniVisible ? "Hide floating Mini" : "Show floating Mini")
            .disabled(windows.isTransitioning)
    }

    @ViewBuilder private var styledMiniButton: some View {
        #if compiler(>=6.2)
        if #available(macOS 26.0, *) {
            miniButton.buttonStyle(.glass).controlSize(.small).tint(nil)
        } else {
            miniButton.buttonStyle(.bordered).controlSize(.small)
        }
        #else
        miniButton.buttonStyle(.bordered).controlSize(.small)
        #endif
    }

    private var miniButton: some View {
        Button { windows.toggleMini() } label: {
            Text("Mini").font(.system(size: 10, weight: .medium))
        }
    }

    private var placeholder: some View {
        VStack(spacing: 16) {
            Image(systemName: windows.page.symbol).font(.system(size: 30, weight: .light)).foregroundStyle(.tertiary)
            Text("Needs to be built").font(ShellTypography.serif(20))
            Text(windows.page.rawValue).font(ShellTypography.serif(13)).foregroundStyle(.secondary)
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
                    if let host = store.modelChoice?.route.host {
                        Text(verbatim: host).font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                    }
                    ConnectionBadge(store: store)
                    if let error = store.error { Text(error).font(ShellTypography.serif(13)).foregroundStyle(.secondary) }
                }
                settingsCard("MINI") {
                    Toggle("Show the floating companion", isOn: Binding(
                        get: { windows.miniVisible },
                        set: { if $0 != windows.miniVisible { windows.toggleMini() } }
                    ))
                    .toggleStyle(.switch)
                    Text("Click to chat. Drag to move. Mini works even with the dashboard closed.")
                        .font(ShellTypography.serif(13)).foregroundStyle(.secondary)
                }
                Text("Chats and attached images are saved locally in Application Support/lorca.")
                    .font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary)
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
