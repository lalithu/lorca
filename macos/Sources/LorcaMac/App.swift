import AppKit

@main
@MainActor
enum LorcaApp {
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.regular)
        app.run()
        withExtendedLifetime(delegate) {}
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let store = ChatStore()
    private lazy var windows = WindowManager(store: store)
    private let shutdown = AppShutdown()
    private var rebuildSignal: RebuildSignal?

    func applicationDidFinishLaunching(_ notification: Notification) {
        if let url = Bundle.main.url(forResource: "AppIcon", withExtension: "icns"), let icon = NSImage(contentsOf: url) {
            NSApp.applicationIconImage = icon
        }
        rebuildSignal = try? RebuildSignal(root: store.backend.root) { [weak self] in
            self?.beginShutdown(unload: false, showFailures: false)
        }
        installMenus()
        windows.open()
        store.modelSession.approvePreviousUnload = { [weak self] model in
            guard let self else { return false }
            let alert = self.sharedModelAlert("Unload \(model) before loading the next model?", models: [model])
            alert.addButton(withTitle: "Unload")
            alert.addButton(withTitle: "Keep Both Loaded")
            return alert.runModal() == .alertFirstButtonReturn
        }
        Task { await store.refreshConnection() }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        windows.showDashboard()
        if windows.miniVisible { windows.showMini() }
        return true
    }

    func applicationWillTerminate(_ notification: Notification) {
        rebuildSignal?.removeMarker()
        store.stop()
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        switch shutdown.phase {
        case .ready: return .terminateNow
        case .closing: return .terminateCancel
        case .running: break
        }
        let models = store.modelSession.modelsToRelease.sorted()
        let alert = sharedModelAlert("Quit lorca and unload its models?", models: models)
        alert.addButton(withTitle: "Quit & Unload")
        alert.addButton(withTitle: "Keep Loaded & Quit")
        alert.addButton(withTitle: "Cancel")
        if models.isEmpty {
            alert.informativeText = "Lorca hasn’t loaded a model in this session. Your conversations and Job Apps progress are saved."
        }
        sender.activate(ignoringOtherApps: true)
        let choice = alert.runModal()
        guard choice != .alertThirdButtonReturn else { return .terminateCancel }
        beginShutdown(unload: choice == .alertFirstButtonReturn, showFailures: true)
        return .terminateCancel
    }

    private func beginShutdown(unload: Bool, showFailures: Bool) {
        shutdown.begin(unload: unload, cleanup: { [store] unload in
            await store.modelSession.close(unload: unload)
        }) { failures in
            if showFailures && !failures.isEmpty {
                let failure = NSAlert()
                failure.messageText = "Model unloading wasn’t confirmed"
                failure.informativeText = "Ollama may still have these models loaded: \(failures.joined(separator: ", "))."
                failure.addButton(withTitle: "Quit")
                failure.runModal()
            }
            NSApp.terminate(nil)
        }
    }

    private func sharedModelAlert(_ title: String, models: [String]) -> NSAlert {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = title
        alert.informativeText = "Ollama shares models between apps. Unloading also affects any other client using \(models.joined(separator: ", "))."
        return alert
    }

    @objc private func desktop() { windows.showDashboard() }
    @objc private func closeDashboard() { windows.closeDashboard() }
    @objc private func fullScreen() { windows.showFullScreen() }
    @objc private func mini() { windows.toggleMini() }
    @objc private func settings() { windows.page = .settings; windows.showDashboard() }
    @objc private func newChat() { store.newChat(); windows.page = .chats }

    private func installMenus() {
        let menu = NSMenu()
        let application = NSMenu()
        application.addItem(withTitle: "About lorca", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        application.addItem(.separator())
        let settingsItem = application.addItem(withTitle: "Settings…", action: #selector(settings), keyEquivalent: ",")
        settingsItem.target = self
        application.addItem(.separator())
        application.addItem(withTitle: "Hide lorca", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        let quitItem = application.addItem(withTitle: "Quit lorca", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        quitItem.target = NSApp
        let appItem = NSMenuItem()
        appItem.submenu = application
        menu.addItem(appItem)

        let edit = NSMenu(title: "Edit")
        for (title, selector, key) in [("Undo", "undo:", "z"), ("Cut", "cut:", "x"), ("Copy", "copy:", "c"), ("Paste", "paste:", "v"), ("Select All", "selectAll:", "a")] {
            edit.addItem(withTitle: title, action: Selector(selector), keyEquivalent: key)
        }
        let editItem = NSMenuItem(title: "Edit", action: nil, keyEquivalent: "")
        editItem.submenu = edit
        menu.addItem(editItem)

        let view = NSMenu(title: "View")
        store.uiZoom.addCommands(to: view)
        let viewItem = NSMenuItem(title: "View", action: nil, keyEquivalent: "")
        viewItem.submenu = view
        menu.addItem(viewItem)

        let window = NSMenu(title: "Window")
        let closeItem = window.addItem(withTitle: "Close Dashboard", action: #selector(closeDashboard), keyEquivalent: "w")
        closeItem.target = self
        window.addItem(.separator())
        let desktopItem = window.addItem(withTitle: "Show Dashboard", action: #selector(desktop), keyEquivalent: "1")
        desktopItem.target = self
        let fullScreenItem = window.addItem(withTitle: "Full Screen Dashboard", action: #selector(fullScreen), keyEquivalent: "f")
        fullScreenItem.keyEquivalentModifierMask = [.command, .control]
        fullScreenItem.target = self
        let miniItem = window.addItem(withTitle: "Show / Hide Mini", action: #selector(mini), keyEquivalent: "2")
        miniItem.target = self
        window.addItem(.separator())
        let chatItem = window.addItem(withTitle: "New Chat", action: #selector(newChat), keyEquivalent: "n")
        chatItem.target = self
        let windowItem = NSMenuItem(title: "Window", action: nil, keyEquivalent: "")
        windowItem.submenu = window
        menu.addItem(windowItem)
        NSApp.mainMenu = menu
        NSApp.windowsMenu = window
    }
}

enum LorcaAsset {
    static let emoji: String = {
        let url = Bundle.main.url(forResource: "orca", withExtension: "txt")
            ?? Bundle.module.url(forResource: "orca", withExtension: "txt")
        return url.flatMap { try? String(contentsOf: $0, encoding: .utf8) }
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .flatMap { $0.isEmpty ? nil : $0 } ?? "🐋"
    }()
}
