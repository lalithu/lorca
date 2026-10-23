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
    private var isQuitting = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        installMenus()
        windows.open()
        store.modelSession.approvePreviousUnload = { [weak self] model in
            guard let self else { return false }
            let alert = self.sharedModelAlert("Unload the previous model?", models: [model])
            alert.addButton(withTitle: "Unload")
            alert.addButton(withTitle: "Keep Loaded")
            return alert.runModal() == .alertFirstButtonReturn
        }
        Task { await store.refreshConnection() }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        windows.showDashboard()
        windows.showMini()
        return true
    }

    func applicationWillTerminate(_ notification: Notification) { store.stop() }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard !isQuitting else { return .terminateLater }
        let models = store.modelSession.modelsToRelease.sorted()
        guard !models.isEmpty else { return .terminateNow }
        let alert = sharedModelAlert("Quit lorca and unload its models?", models: models)
        alert.addButton(withTitle: "Quit & Unload")
        alert.addButton(withTitle: "Keep Loaded & Quit")
        alert.addButton(withTitle: "Cancel")
        let choice = alert.runModal()
        guard choice != .alertThirdButtonReturn else { return .terminateCancel }
        isQuitting = true
        store.stop()
        Task {
            let failures = await store.modelSession.close(unload: choice == .alertFirstButtonReturn)
            if !failures.isEmpty {
                let failure = NSAlert()
                failure.messageText = "Model unloading wasn’t confirmed"
                failure.informativeText = "Ollama may still have these models loaded: \(failures.joined(separator: ", "))."
                failure.addButton(withTitle: "Quit")
                failure.runModal()
            }
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
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
    @objc private func increaseChatText() { store.resizeChatText(by: 1) }
    @objc private func decreaseChatText() { store.resizeChatText(by: -1) }
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
        application.addItem(withTitle: "Quit lorca", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
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
        let increaseItem = view.addItem(withTitle: "Increase Chat Text Size", action: #selector(increaseChatText), keyEquivalent: "=")
        increaseItem.target = self
        let decreaseItem = view.addItem(withTitle: "Decrease Chat Text Size", action: #selector(decreaseChatText), keyEquivalent: "-")
        decreaseItem.target = self
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
