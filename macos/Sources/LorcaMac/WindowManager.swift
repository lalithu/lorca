import AppKit
import Combine
import QuartzCore
import SwiftUI

@MainActor
final class WindowManager: NSObject, ObservableObject, NSWindowDelegate, NSToolbarDelegate {
    @Published var page: SidebarPage = .chats {
        didSet { toolbarTitle.stringValue = "lorca · \(page.rawValue)" }
    }
    @Published private(set) var sidebarVisible = true
    @Published private(set) var isFullScreen = false
    @Published private(set) var isTransitioning = false
    @Published private(set) var miniVisible = false
    @Published private(set) var miniExpanded = false
    @Published private(set) var miniChatBelowPet = false

    private let store: ChatStore
    private var dashboard: DashboardWindow!
    private var mini: MiniPanel!
    private var miniChat: MiniPanel!
    private var miniAnimationID = UUID()
    private var zoomObservation: AnyCancellable?
    private let toolbarTitle = NSTextField(labelWithString: "lorca · Chats")

    init(store: ChatStore) { self.store = store }

    func open() {
        dashboard = DashboardWindow(contentRect: NSRect(x: 0, y: 0, width: 880, height: 600),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered, defer: false)
        dashboard.title = "lorca"
        dashboard.titleVisibility = .hidden
        dashboard.titlebarAppearsTransparent = true
        dashboard.isOpaque = false
        dashboard.backgroundColor = .windowBackgroundColor
        dashboard.isReleasedWhenClosed = false
        dashboard.minSize = NSSize(width: 740, height: 500)
        dashboard.isMovableByWindowBackground = true
        dashboard.collectionBehavior = [.fullScreenPrimary]
        dashboard.delegate = self
        let toolbar = NSToolbar(identifier: "lorca.dashboard")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        dashboard.toolbar = toolbar
        dashboard.toolbarStyle = .unifiedCompact
        dashboard.titlebarSeparatorStyle = .none
        let dashboardContent = ZoomHostingView(rootView: DashboardView(store: store, windows: self), zoom: store.uiZoom)
        // AppKit owns window geometry; SwiftUI must not replace it with a fitting size.
        dashboard.contentView = dashboardContent
        if !dashboard.setFrameUsingName("lorca.dashboard") { dashboard.center() }
        dashboard.setFrameAutosaveName("lorca.dashboard")

        mini = MiniPanel(contentRect: NSRect(x: 0, y: 0, width: 160, height: 168),
            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        mini.title = "lorca Mini"
        mini.isOpaque = false
        mini.backgroundColor = .clear
        mini.hasShadow = false
        mini.isFloatingPanel = true
        mini.hidesOnDeactivate = false
        mini.isReleasedWhenClosed = false
        mini.level = .floating
        mini.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        mini.appearance = NSAppearance(named: .darkAqua)
        mini.contentView = ZoomHostingView(rootView:
            PetHandle(
                toggle: { [weak self] in self?.toggleMiniExpansion() },
                dragEnded: { [weak self] in self?.finishMiniDrag() }
            )
                .frame(width: 160, height: 160).padding(.bottom, 4), zoom: store.uiZoom
        )
        if let screen = NSScreen.main?.visibleFrame {
            mini.setFrameOrigin(NSPoint(x: screen.maxX - 190, y: screen.minY + 35))
        }
        miniChat = MiniPanel(contentRect: NSRect(origin: .zero, size: NSSize(width: 384, height: 400)),
            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        miniChat.title = "lorca Mini Chat"
        miniChat.isOpaque = false
        miniChat.backgroundColor = .clear
        miniChat.hasShadow = true
        miniChat.hidesOnDeactivate = false
        miniChat.isReleasedWhenClosed = false
        miniChat.isFloatingPanel = true
        miniChat.level = .floating
        miniChat.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        miniChat.appearance = NSAppearance(named: .darkAqua)
        let chatContent = ZoomHostingView(rootView: MiniView(store: store, windows: self), zoom: store.uiZoom)
        miniChat.contentView = chatContent
        store.appearanceChanged = { [weak self] in self?.applyAppearance() }
        applyAppearance()
        applyZoom(percent: store.uiZoom.percent)
        zoomObservation = store.uiZoom.$percent.dropFirst().sink { [weak self] in self?.applyZoom(percent: $0) }
        showDashboard()
    }

    private func applyZoom(percent: Int) {
        let scale = CGFloat(percent) / 100
        // Keep the existing logical minimum; avoid clipping fixed-size forms at
        // large zoom while leaving the titlebar and toolbar in native points.
        dashboard.minSize = NSSize(width: 740 * scale, height: 500 * scale)
        if dashboard.frame.width < dashboard.minSize.width || dashboard.frame.height < dashboard.minSize.height {
            var frame = dashboard.frame
            let height = max(frame.height, dashboard.minSize.height)
            frame.origin.y -= height - frame.height
            frame.size = NSSize(width: max(frame.width, dashboard.minSize.width), height: height)
            dashboard.setFrame(frame, display: true)
        }
        var pet = mini.frame
        let petSize = NSSize(width: 160 * scale, height: 168 * scale)
        pet.origin.y += pet.height - petSize.height
        pet.size = petSize
        if let screen = mini.screen?.visibleFrame ?? NSScreen.main?.visibleFrame {
            pet.origin.x = min(max(pet.minX, screen.minX), screen.maxX - pet.width)
            pet.origin.y = min(max(pet.minY, screen.minY), screen.maxY - pet.height)
        }
        mini.setFrame(pet, display: mini.isVisible)
        miniChat.setContentSize(NSSize(width: 384 * scale, height: 400 * scale))
        if miniExpanded { positionMiniChat(preserveSide: true) }
    }

    func showDashboard() {
        guard !isTransitioning else { return }
        dashboard.level = .normal
        if dashboard.isMiniaturized { dashboard.deminiaturize(nil) }
        dashboard.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func closeDashboard() {
        dashboard.performClose(nil)
    }

    func showFullScreen() {
        guard !isTransitioning else { return }
        dashboard.level = .normal
        dashboard.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        if !dashboard.styleMask.contains(.fullScreen) { dashboard.toggleFullScreen(nil) }
    }

    private func applyAppearance() {
        switch store.appearance {
        case .system: dashboard.appearance = nil
        case .dark: dashboard.appearance = NSAppearance(named: .darkAqua)
        case .light: dashboard.appearance = NSAppearance(named: .aqua)
        }
    }

    func showMini() { miniVisible = true; mini.orderFrontRegardless() }
    func toggleMini() {
        if miniVisible {
            miniVisible = false
            miniExpanded = false
            miniAnimationID = UUID()
            mini.removeChildWindow(miniChat)
            miniChat.orderOut(nil)
            miniChat.alphaValue = 0
            mini.orderOut(nil)
        } else { showMini() }
    }

    func toggleMiniExpansion() {
        miniExpanded.toggle()
        let animationID = UUID()
        miniAnimationID = animationID
        if miniExpanded {
            positionMiniChat(preserveSide: false)
            if !miniChat.isVisible { miniChat.alphaValue = 0 }
            // Window Server keeps the two windows together instead of chasing move notifications.
            if miniChat.parent == nil { mini.addChildWindow(miniChat, ordered: .above) }
            miniChat.contentView?.layoutSubtreeIfNeeded()
            miniChat.orderFrontRegardless()
            miniChat.makeKey()
        }
        // Expansion preserves both panel sizes; animate only the chat panel's opacity.
        NSAnimationContext.runAnimationGroup { context in
            context.duration = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? 0 : 0.18
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            miniChat.animator().alphaValue = miniExpanded ? 1 : 0
        } completionHandler: { [weak self] in
            Task { @MainActor in
                guard let self, self.miniAnimationID == animationID, !self.miniExpanded else { return }
                self.mini.removeChildWindow(self.miniChat)
                self.miniChat.orderOut(nil)
            }
        }
    }

    private func positionMiniChat(preserveSide: Bool) {
        guard let screen = mini.screen?.visibleFrame ?? NSScreen.main?.visibleFrame else { return }
        let placement = MiniPlacement(pet: mini.frame, chatSize: miniChat.frame.size, screen: screen,
            preferredBelow: preserveSide ? miniChatBelowPet : true)
        if miniChatBelowPet != placement.belowPet { miniChatBelowPet = placement.belowPet }
        miniChat.setFrameOrigin(placement.origin)
    }

    private func finishMiniDrag() {
        if miniExpanded { positionMiniChat(preserveSide: true) }
    }

    @objc private func toggleSidebar() { sidebarVisible.toggle() }
    @objc func newChat() { store.newChat(); page = .chats }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.init("lorca.sidebar"), .init("lorca.newChat"), .init("lorca.title"), .flexibleSpace]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar)
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        let item = NSToolbarItem(itemIdentifier: identifier)
        switch identifier.rawValue {
        case "lorca.sidebar":
            item.label = "Toggle Sidebar"
            item.view = toolbarButton("sidebar.left", label: item.label, action: #selector(toggleSidebar))
        case "lorca.newChat":
            item.label = "New Chat"
            item.view = toolbarButton("square.and.pencil", label: item.label, action: #selector(newChat))
        case "lorca.title":
            let separator = NSBox()
            separator.boxType = .separator
            separator.widthAnchor.constraint(equalToConstant: 1).isActive = true
            separator.heightAnchor.constraint(equalToConstant: 18).isActive = true
            toolbarTitle.font = .systemFont(ofSize: 13, weight: .medium)
            let title = NSStackView(views: [separator, toolbarTitle])
            title.orientation = .horizontal
            title.alignment = .centerY
            title.heightAnchor.constraint(equalToConstant: 28).isActive = true
            title.spacing = 14
            title.edgeInsets = NSEdgeInsets(top: 0, left: 6, bottom: 0, right: 0)
            item.view = title
            item.label = "lorca"
        default: return nil
        }
        item.toolTip = item.label
        return item
    }

    private func toolbarButton(_ symbol: String, label: String, action: Selector) -> NSButton {
        let image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)!
            .withSymbolConfiguration(.init(pointSize: 14, weight: .regular))!
        let button = NSButton(image: image, target: self, action: action)
        button.isBordered = false
        button.imageScaling = .scaleProportionallyDown
        button.widthAnchor.constraint(equalToConstant: 30).isActive = true
        button.heightAnchor.constraint(equalToConstant: 28).isActive = true
        button.setAccessibilityLabel(label)
        button.toolTip = label
        return button
    }

    func windowWillEnterFullScreen(_ notification: Notification) {
        dashboard.level = .normal
        isTransitioning = true
    }
    func windowDidEnterFullScreen(_ notification: Notification) {
        isFullScreen = true
        isTransitioning = false
    }
    func windowWillExitFullScreen(_ notification: Notification) { isTransitioning = true }
    func windowDidExitFullScreen(_ notification: Notification) {
        isFullScreen = false
        isTransitioning = false
        dashboard.level = .normal
    }
    func windowDidFailToEnterFullScreen(_ window: NSWindow) {
        isTransitioning = false
        isFullScreen = false
        dashboard.level = .normal
    }
    func windowDidFailToExitFullScreen(_ window: NSWindow) { isTransitioning = false }
}

private final class DashboardWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

private final class MiniPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

struct VisualEffect: NSViewRepresentable {
    var material: NSVisualEffectView.Material
    @Environment(\.colorScheme) private var colorScheme

    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.blendingMode = .behindWindow
        view.state = .active
        return view
    }
    func updateNSView(_ view: NSVisualEffectView, context: Context) {
        view.material = material
        view.appearance = NSAppearance(named: colorScheme == .dark ? .darkAqua : .aqua)
    }
}

struct PetHandle: NSViewRepresentable {
    var toggle: () -> Void
    var dragEnded: () -> Void
    func makeNSView(context: Context) -> PetHandleView {
        let view = PetHandleView()
        view.setAccessibilityElement(true)
        view.setAccessibilityRole(.button)
        view.setAccessibilityLabel("Lorca Mini")
        view.setAccessibilityHelp("Click to open or close chat. Drag to move.")
        return view
    }
    func updateNSView(_ view: PetHandleView, context: Context) {
        view.toggle = toggle
        view.dragEnded = dragEnded
    }
}

final class PetHandleView: NSView {
    var toggle: (() -> Void)?
    var dragEnded: (() -> Void)?
    private var mouseStart: NSPoint?
    private var windowStart = NSPoint.zero
    private var dragging = false
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func accessibilityPerformPress() -> Bool { toggle?(); return true }

    override func draw(_ dirtyRect: NSRect) {
        let text = NSAttributedString(string: LorcaAsset.emoji, attributes: [
            .font: NSFont(name: "Apple Color Emoji", size: 128) ?? NSFont.systemFont(ofSize: 128)
        ])
        let size = text.size()
        text.draw(at: NSPoint(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2))
    }

    override func mouseDown(with event: NSEvent) {
        mouseStart = NSEvent.mouseLocation
        windowStart = window?.frame.origin ?? .zero
        dragging = false
    }

    override func mouseDragged(with event: NSEvent) {
        guard let mouseStart, let window else { return }
        let mouse = NSEvent.mouseLocation
        let delta = NSPoint(x: mouse.x - mouseStart.x, y: mouse.y - mouseStart.y)
        guard dragging || hypot(delta.x, delta.y) >= 3 else { return }
        dragging = true
        // Child windows move with the pet; its SwiftUI layout stays untouched throughout the drag.
        window.setFrameOrigin(NSPoint(x: windowStart.x + delta.x, y: windowStart.y + delta.y))
    }

    override func mouseUp(with event: NSEvent) {
        if dragging { dragEnded?() } else { toggle?() }
        mouseStart = nil
    }
}
