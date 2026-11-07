import AppKit
import SwiftUI
import Vision

// A real NSApplication event loop is required for mouse tracking and key equivalents.
enum LorcaAsset {
    static let emoji = (try! String(contentsOfFile: ProcessInfo.processInfo.environment["LORCA_SOURCE_ROOT"]! + "/macos/Sources/LorcaMac/Resources/orca.txt", encoding: .utf8)).trimmingCharacters(in: .whitespacesAndNewlines)
}
@main struct SelectionMain {
    @MainActor static func main() {
        setbuf(stdout, nil)
        let app = NSApplication.shared
        let delegate = SelectionAcceptance()
        app.delegate = delegate
        app.setActivationPolicy(.regular)
        app.run()
        _ = delegate
    }
}
@MainActor final class SelectionAcceptance: NSObject, NSApplicationDelegate {
    private var window: NSWindow!
    private var savedPasteboard: [[(NSPasteboard.PasteboardType, Data)]] = []
    private var results: [[String: Any]] = []
    private var selectionFailed = false
    private let source = "Alpha bravo charlie delta.\n\nSecond paragraph for multiple line selection."
    private var root: URL { URL(fileURLWithPath: ProcessInfo.processInfo.environment["LORCA_ACCEPTANCE_ROOT"]!) }
    func applicationDidFinishLaunching(_ notification: Notification) {
        let menu = NSMenu(); let editItem = NSMenuItem(); let edit = NSMenu(title: "Edit")
        edit.addItem(withTitle: "Copy", action: NSSelectorFromString("copy:"), keyEquivalent: "c")
        editItem.submenu = edit; menu.addItem(editItem); NSApp.mainMenu = menu
        savedPasteboard = NSPasteboard.general.pasteboardItems?.map { item in
            item.types.compactMap { type in item.data(forType: type).map { (type, $0) } }
        } ?? []
        window = NSWindow(contentRect: NSRect(x: 100, y: 200, width: 650, height: 350), styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        NSApp.activate(ignoringOtherApps: true)
        Task { await run() }
    }
    private func capture(_ name: String) {
        guard let view = window.contentView else { return }
        view.layoutSubtreeIfNeeded()
        if let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
            view.cacheDisplay(in: view.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: root.appendingPathComponent(name + ".png"))
        }
    }
    private func scene<V: View>(_ view: V, dark: Bool = true, height: CGFloat = 350, width: CGFloat = 650) async {
        window.makeFirstResponder(nil)
        window.close()
        window = NSWindow(contentRect: NSRect(x: 100, y: 200, width: width, height: height), styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: VStack(alignment: .leading, spacing: 0) { view; Spacer(minLength: 0) }
            .padding(16).frame(width: width, height: height).background(Color(nsColor: .windowBackgroundColor)).preferredColorScheme(dark ? .dark : .light))
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        try? await Task.sleep(for: .milliseconds(300))
    }
    private func drag(from start: NSPoint, to end: NSPoint) async {
        let now = ProcessInfo.processInfo.systemUptime
        let types = [NSEvent.EventType.leftMouseDown] + Array(repeating: .leftMouseDragged, count: 8) + [.leftMouseUp]
        for (index, type) in types.enumerated() {
            let fraction = min(1, Double(index) / 8)
            let point = NSPoint(x: start.x + (end.x - start.x) * fraction, y: start.y + (end.y - start.y) * fraction)
            if let event = NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: now + Double(index) * 0.015,
                windowNumber: window.windowNumber, context: nil, eventNumber: index, clickCount: 1, pressure: type == .leftMouseUp ? 0 : 1) {
                NSApp.postEvent(event, atStart: false)
            }
        }
        try? await Task.sleep(for: .milliseconds(150))
    }
    private func copyKey() async -> String {
        NSPasteboard.general.clearContents()
        for type in [NSEvent.EventType.keyDown, .keyUp] {
            if let event = NSEvent.keyEvent(with: type, location: .zero, modifierFlags: .command, timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, characters: "c", charactersIgnoringModifiers: "c", isARepeat: false, keyCode: 8) {
                NSApp.postEvent(event, atStart: false)
            }
        }
        try? await Task.sleep(for: .milliseconds(100))
        return NSPasteboard.general.string(forType: .string) ?? ""
    }
    private func selectionPoints() throws -> (start: NSPoint, firstEnd: NSPoint, secondEnd: NSPoint)? {
        // A view snapshot can omit native glass. Capture the test window so
        // the drag starts on text that is actually visible on screen.
        let image = root.appendingPathComponent("selection-fixture.png")
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l", String(window.windowNumber), image.path]
        try capture.run()
        capture.waitUntilExit()
        guard capture.terminationStatus == 0 else { return nil }
        defer { try? FileManager.default.removeItem(at: image) }

        let request = VNRecognizeTextRequest()
        request.usesLanguageCorrection = false
        try VNImageRequestHandler(url: image).perform([request])
        let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first }
        func word(_ text: String) throws -> NSPoint? {
            for line in lines {
                if let range = line.string.range(of: text),
                   let box = try line.boundingBox(for: range)?.boundingBox {
                    return NSPoint(x: box.midX * window.frame.width, y: box.midY * window.frame.height)
                }
            }
            return nil
        }
        guard let alpha = try word("Alpha"), let delta = try word("delta"),
              let multiple = try word("multiple") else { return nil }
        return (alpha, delta, multiple)
    }

    private func selection(_ name: String) async {
        var copied = ""
        var multiline = ""
        var menuTitles: [String] = []
        do {
            if let points = try selectionPoints() {
                await drag(from: points.start, to: points.firstEnd)
                copied = await copyKey()
                capture(name + "-selected")
                await drag(from: points.start, to: points.secondEnd)
                multiline = await copyKey()
                capture(name + "-multiline")
                if let event = NSEvent.mouseEvent(with: .rightMouseDown, location: points.start, modifierFlags: [],
                    timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                    context: nil, eventNumber: 0, clickCount: 1, pressure: 1) {
                    menuTitles = (window.firstResponder as? NSView)?.menu(for: event)?.items.map(\.title) ?? []
                }
            } else {
                print("NATIVE_SELECTION \(name) could not locate fixture text")
            }
        } catch {
            print("NATIVE_SELECTION \(name) could not read fixture text: \(error)")
        }
        let selected = source.contains(copied) && copied.contains("bravo charlie") && !copied.contains("\n")
        let selectedAcrossLines = source.contains(multiline) && multiline != source
            && multiline.contains("bravo charlie delta.\n\nSecond paragraph")
        selectionFailed = selectionFailed || !selected || !selectedAcrossLines
        results.append(["name": name, "copied": copied, "selectedSubstring": selected,
            "multilineCopied": multiline, "multilineSelection": selectedAcrossLines,
            "contextMenuTitles": menuTitles, "responder": String(describing: type(of: window.firstResponder as Any))])
        print("NATIVE_SELECTION \(name) selected=\(selected) multiline=\(selectedAcrossLines)")
    }
    private func run() async {
        await scene(MarkdownText(text: source, size: 18, showsCopyButton: false)); await selection("assistant")
        await scene(UserMessageBubble(text: source) { MarkdownText(text: source, size: 18, showsCopyButton: false, userMessage: true) }); capture("user-blue-dark"); await selection("user")
        await scene(UserMessageBubble(text: source) { MarkdownText(text: source, size: 18, showsCopyButton: false, userMessage: true) }, dark: false); capture("user-blue-light")
        await scene(ReasoningText(text: source)); await selection("reasoning")
        await scene(AgentOutputView(text: source)); await selection("tool-output")
        await scene(MarkdownText(text: "```text\n" + source + "\n```", size: 18, showsCopyButton: false)); await selection("code")
        for dark in [true, false] {
            await scene(VStack(alignment: .leading, spacing: 16) {
                ThinkingControlLabel(enabled: true)
                ThinkingControlLabel(enabled: false)
                ThinkingControlLabel(enabled: true, nextTask: true)
                ThinkingControlLabel(enabled: false, nextTask: true)
                ThinkingPanel(block: AgentThinkingBlock(id: "wrap", state: "complete", text: String(repeating: "This is a long reasoning sentence that wraps within the available width. ", count: 30), afterSequence: 0, textOffset: 0, truncated: false, stage: .interpretation), initiallyExpanded: true)
            }, dark: dark, height: 550)
            capture(dark ? "thinking-green-dark" : "thinking-green-light")
        }
        var layouts: [[String: Any]] = []
        for width in [360.0, 850.0] {
            await scene(ReasoningText(text: String(repeating: "Reasoning prose wraps at every window size. ", count: 100)), height: 300, width: width)
            func scrolls(_ view: NSView) -> [NSScrollView] { (view as? NSScrollView).map { [$0] } ?? view.subviews.flatMap { scrolls($0) } }
            for scroll in scrolls(window.contentView!) {
                layouts.append(["width": width, "viewport": scroll.contentView.bounds.width, "document": scroll.documentView?.bounds.width ?? 0, "height": scroll.bounds.height])
            }
            capture(width == 360 ? "thinking-wrap-narrow" : "thinking-wrap-wide")
        }
        try? JSONSerialization.data(withJSONObject: layouts, options: [.prettyPrinted]).write(to: root.appendingPathComponent("layout-report.json"))
        NSPasteboard.general.clearContents()
        NSPasteboard.general.writeObjects(savedPasteboard.map { entries in
            let item = NSPasteboardItem(); for (type, data) in entries { item.setData(data, forType: type) }; return item
        })
        try? JSONSerialization.data(withJSONObject: results, options: [.prettyPrinted, .sortedKeys]).write(to: root.appendingPathComponent("selection-report.json"))
        print("NATIVE_SELECTION_ACCEPTANCE_FINISHED")
        exit(selectionFailed ? EXIT_FAILURE : EXIT_SUCCESS)
    }
}
