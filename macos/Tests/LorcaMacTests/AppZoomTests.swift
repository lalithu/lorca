import AppKit
import SwiftUI
import XCTest
@testable import LorcaMac

@MainActor
final class AppZoomTests: XCTestCase {
    private func preference() -> UserDefaults {
        let name = "lorca.ui-zoom-tests.\(UUID().uuidString)"
        let value = UserDefaults(suiteName: name)!
        addTeardownBlock { value.removePersistentDomain(forName: name) }
        return value
    }
    func testDefaultStepsLimitsResetAndPersistence() {
        let defaults = preference(), zoom = AppZoom(defaults: preference())
        XCTAssertEqual(zoom.percent, 100)
        for _ in 0..<20 { zoom.zoomOut() }
        XCTAssertEqual(zoom.percent, 70)
        for _ in 0..<20 { zoom.zoomIn() }
        XCTAssertEqual(zoom.percent, 150)
        zoom.actualSize();XCTAssertEqual(zoom.percent, 100)
        let persistent = AppZoom(defaults: defaults)
        persistent.setPercent(120)
        XCTAssertEqual(AppZoom(defaults: defaults).percent, 120)
        defaults.set(Int.max, forKey: AppZoom.preferenceKey)
        XCTAssertEqual(AppZoom(defaults: defaults).percent, 150)
        defaults.set(81, forKey: AppZoom.preferenceKey)
        XCTAssertEqual(AppZoom(defaults: defaults).percent, 80)
    }
    func testLayoutProposalsScaleWithoutChangingTheNativeViewport() {
        for percent in stride(from: 70, through: 150, by: 10) {
            let factor = CGFloat(percent) / 100
            let logical = ZoomLayout.logical(ProposedViewSize(width: 900, height: 600), factor: factor)
            XCTAssertEqual(logical.width!, 900 / factor, accuracy: 0.001)
            XCTAssertEqual(logical.height!, 600 / factor, accuracy: 0.001)
            XCTAssertNil(ZoomLayout.logical(.unspecified, factor: factor).width)
        }
        let zoom = AppZoom(defaults: preference())
        let host = ZoomHostingView(rootView: Color.clear, zoom: zoom)
        host.setFrameSize(NSSize(width: 900, height: 600))
        XCTAssertEqual(host.frame.size, host.bounds.size)
    }
    func testNativeHitTestingTextSelectionAndScrollGeometryFollowZoom() async throws {
        let zoom = AppZoom(defaults: preference())
        let controls = ZoomTestControls()
        let host = ZoomHostingView(rootView: controls, zoom: zoom)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = host
        defer { window.orderOut(nil) }
        window.orderFront(nil)
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(100))
        let editor = controls.editor, button = controls.button, scroll = controls.scroll
        editor.string = "Keep this selection and keyboard focus"
        editor.setSelectedRange(NSRange(location: 5, length: 4))
        window.makeFirstResponder(editor)
        for percent in [80, 100, 120, 150] {
            zoom.setPercent(percent);host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(80))
            let scale = CGFloat(percent) / 100
            XCTAssertTrue(window.firstResponder === editor)
            XCTAssertEqual(editor.selectedRange(), NSRange(location: 5, length: 4))
            let center = button.convert(NSPoint(x: button.bounds.midX, y: button.bounds.midY), to: host)
            let hit = host.hitTest(host.convert(center, to: host.superview))
            XCTAssertTrue(hit === button || hit?.isDescendant(of: button) == true)
            let rect = button.convert(button.bounds, to: nil)
            XCTAssertEqual(rect.width, button.bounds.width * scale, accuracy: 0.01)
            XCTAssertEqual(rect.height, button.bounds.height * scale, accuracy: 0.01)
            scroll.contentView.scroll(to: NSPoint(x: 0, y: 180))
            XCTAssertEqual(scroll.contentView.bounds.minY, 180, accuracy: 1)
            XCTAssertEqual(scroll.convert(NSSize(width: 100, height: 100), to: nil).width, 100 * scale, accuracy: 0.01)
            XCTAssertEqual(button.accessibilityLabel(), "Zoom test action")
            let accessible = button.accessibilityFrame()
            let expected = window.convertToScreen(rect)
            // AppKit's native button accessibility rect includes bezel rounding.
            XCTAssertTrue(accessible.contains(NSPoint(x: expected.midX, y: expected.midY)))
            XCTAssertEqual(accessible.midX, expected.midX, accuracy: 2 * scale + 0.01)
            XCTAssertEqual(accessible.midY, expected.midY, accuracy: 2 * scale + 0.01)
            XCTAssertEqual(accessible.width, expected.width, accuracy: 4 * scale + 0.01)
            XCTAssertEqual(accessible.height, expected.height, accuracy: 4 * scale + 0.01)
        }
    }
    func testMeasuredPresentationScalesFixedFormWithoutChangingItsBaseline() {
        let zoom = AppZoom(defaults: preference())
        let controller = NSHostingController(rootView: ZoomContent(zoom: zoom) { Text("Form").frame(width: 620, height: 580) })
        for percent in [80, 100, 120, 150] {
            zoom.setPercent(percent)
            controller.view.layoutSubtreeIfNeeded()
            let size = controller.sizeThatFits(in: NSSize(width: 2000, height: 2000))
            XCTAssertEqual(size.width, 620 * zoom.factor, accuracy: 1)
            XCTAssertEqual(size.height, 580 * zoom.factor, accuracy: 1)
        }
    }
    func testTitlebarSafeAreaIsAppliedOnceAcrossZoomLevels() async throws {
        let zoom = AppZoom(defaults: preference()), marker = NSView()
        let root = VStack(spacing: 0) { ZoomMarker(view: marker).frame(height: 20);Spacer(minLength: 0) }
        let host = ZoomHostingView(rootView: root, zoom: zoom)
        let window = NSWindow(contentRect: NSRect(x: 60, y: 60, width: 600, height: 400), styleMask: [.titled, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        window.toolbar = NSToolbar(identifier: "zoom-safe-area-test")
        window.toolbarStyle = .unifiedCompact
        window.contentView = host;window.orderFront(nil)
        defer { window.orderOut(nil) }
        var originalTop: CGFloat?
        for percent in [100, 80, 120, 150, 100] {
            zoom.setPercent(percent)
            try await Task.sleep(for: .milliseconds(80))
            host.layoutSubtreeIfNeeded()
            let top = marker.convert(marker.bounds, to: nil).maxY
            if originalTop == nil { originalTop = top }
            XCTAssertEqual(top, originalTop!, accuracy: 1)
            XCTAssertEqual(top, window.contentLayoutRect.maxY, accuracy: 1)
        }
    }
    func testScaledSheetKeepsNativeDismissAndPresentationEnvironment() async throws {
        let zoom = AppZoom(defaults: preference()), state = ZoomSheetState()
        let parent = ZoomSheetParent(state: state, zoom: zoom)
        let host = ZoomHostingView(rootView: parent, zoom: zoom)
        let window = NSWindow(contentRect: NSRect(x: 60, y: 60, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = host;window.orderFront(nil)
        defer { window.orderOut(nil) }
        for percent in [80, 100, 120, 150] {
            zoom.setPercent(percent);state.shown = true
            try await Task.sleep(for: .milliseconds(200))
            XCTAssertNotNil(window.attachedSheet)
            XCTAssertNotNil(state.dismiss)
            state.dismiss?()
            try await Task.sleep(for: .milliseconds(200))
            XCTAssertFalse(state.shown)
        }
    }
    func testViewMenuShortcutsAndBoundsWhileTextInputHasFocus() {
        let zoom = AppZoom(defaults: preference())
        let original = NSApp.mainMenu
        let main = NSMenu(), menu = NSMenu(title: "View"), item = NSMenuItem(title: "View", action: nil, keyEquivalent: "")
        item.submenu = menu;main.addItem(item);zoom.addCommands(to: menu);NSApp.mainMenu = main
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 200), styleMask: [.titled], backing: .buffered, defer: false)
        let editor = NSTextView(frame: window.contentView!.bounds);window.contentView!.addSubview(editor);window.makeFirstResponder(editor)
        defer { NSApp.mainMenu = original;window.orderOut(nil) }
        for (key,code,flags,expected) in [("=",24,NSEvent.ModifierFlags.command,110),("+",24,[.command,.shift],120),("-",27,.command,110),("0",29,.command,100)] {
            let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: key, charactersIgnoringModifiers: key, isARepeat: false, keyCode: UInt16(code))!
            XCTAssertTrue(main.performKeyEquivalent(with: event))
            XCTAssertEqual(zoom.percent, expected)
            XCTAssertTrue(window.firstResponder === editor)
        }
        zoom.setPercent(150);XCTAssertFalse(zoom.validateMenuItem(menu.items[0]))
        zoom.setPercent(70);XCTAssertFalse(zoom.validateMenuItem(menu.items[1]))
    }
}

private struct ZoomTestControls: NSViewRepresentable {
    let editor = NSTextView(frame: NSRect(x: 20, y: 20, width: 250, height: 600))
    let button = NSButton(title: "Action", target: nil, action: nil)
    let scroll = NSScrollView(frame: NSRect(x: 20, y: 70, width: 300, height: 160))
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        button.frame = NSRect(x: 20, y: 20, width: 100, height: 30)
        button.setAccessibilityLabel("Zoom test action")
        scroll.documentView = editor;scroll.hasVerticalScroller = true
        view.addSubview(scroll);view.addSubview(button)
        return view
    }
    func updateNSView(_ view: NSView, context: Context) { }
}

private struct ZoomMarker: NSViewRepresentable {
    let view: NSView
    func makeNSView(context: Context) -> NSView { view }
    func updateNSView(_ view: NSView, context: Context) { }
}
private final class ZoomSheetState: ObservableObject {
    @Published var shown = false
    var dismiss: (() -> Void)?
}
private struct ZoomSheetParent: View {
    @ObservedObject var state: ZoomSheetState
    let zoom: AppZoom
    var body: some View {
        Text("Parent").frame(maxWidth: .infinity, maxHeight: .infinity)
            .sheet(isPresented: $state.shown) { ZoomContent(zoom: zoom) { ZoomDismissProbe(state: state).frame(width: 320, height: 220) } }
            .transaction { $0.disablesAnimations = true }
    }
}
private struct ZoomDismissProbe: View {
    let state: ZoomSheetState
    @Environment(\.dismiss) private var dismiss
    var body: some View { Text("Form").onAppear { state.dismiss = { dismiss() } } }
}
