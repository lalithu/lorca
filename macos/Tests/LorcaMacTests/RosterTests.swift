import AppKit
import SwiftUI
import XCTest
@testable import LorcaMac

@MainActor final class RosterTests: XCTestCase {
    func testMembersAddEditRemoveAndEmptyRosterPersistWithoutChangingIdentity() throws {
        let suite = "lorca.roster-members-tests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let members = RosterMembers(defaults: defaults)
        let originalIDs = members.members.map(\.id)
        var member = RosterMember(name: "My agent", provider: "Custom", connection: .endpoint, address: "https://example.com/v1", model: "my-model")
        XCTAssertTrue(members.save(member))
        member.name = "Renamed agent"
        XCTAssertTrue(members.save(member))
        let reloaded = RosterMembers(defaults: defaults)
        XCTAssertEqual(reloaded.members.last, member)
        XCTAssertEqual(Array(reloaded.members.dropLast()).map(\.id), originalIDs)
        XCTAssertFalse(members.save(RosterMember(name: "Invalid", connection: .endpoint, address: "not a URL")))
        for id in reloaded.members.map(\.id) { XCTAssertTrue(reloaded.remove(id)) }
        XCTAssertTrue(RosterMembers(defaults: defaults).members.isEmpty, "Removing every member restored placeholder cards")
    }

    func testManagementAndEditorRenderAtSharedZoom() async throws {
        let suite = "lorca.roster-management-tests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let members = RosterMembers(defaults: defaults), zoom = AppZoom(defaults: defaults)
        let draft = RosterMember(name: "My Codex", connection: .command, address: "codex", model: "")
        for editing in [false, true] {
            let host = ZoomHostingView(rootView: RosterManagement(members: members, draft: editing ? draft : nil, resetLayout: {}).preferredColorScheme(.dark), zoom: zoom)
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 800), styleMask: [.titled], backing: .buffered, defer: false)
            window.contentView = host; window.appearance = NSAppearance(named: .darkAqua); window.orderFront(nil)
            defer { window.orderOut(nil) }
            for percent in [100, 150] {
                zoom.setPercent(percent)
                try await Task.sleep(for: .milliseconds(150)); host.layoutSubtreeIfNeeded()
                if let directory = ProcessInfo.processInfo.environment["LORCA_ROSTER_RENDER_DIR"] {
                    try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
                    let capture = Process()
                    capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
                    capture.arguments = ["-x", "-o", "-l", String(window.windowNumber), "\(directory)/roster-\(editing ? "editor" : "management")-\(percent).png"]
                    try capture.run(); capture.waitUntilExit()
                    XCTAssertEqual(capture.terminationStatus, 0)
                }
            }
        }
    }

    func testSavingWorkUsesRosterCommandWithoutStartingChat() async throws {
        let backend = MockBackend()
        let roster = RosterStore(backend: backend)
        var received = false
        roster.receive = { _ in received = true }
        let saved = await roster.add("Review the project. Ask before deleting anything.")
        XCTAssertTrue(saved)
        XCTAssertTrue(received)
        XCTAssertEqual(backend.requests.count, 1)
        XCTAssertEqual(backend.requests[0]["op"] as? String, "roster_add")
        XCTAssertNotNil(UUID(uuidString: backend.requests[0]["taskId"] as! String))
        XCTAssertTrue(backend.chats[0].messages.isEmpty)
        backend.fail = true
        let failed = await roster.add("Keep this task")
        XCTAssertFalse(failed)
        XCTAssertNotNil(roster.error)
        XCTAssertFalse(roster.busy)
        let unconfirmedID = backend.requests.last?["taskId"] as? String
        backend.fail = false
        let retried = await roster.add("Keep this task")
        XCTAssertTrue(retried)
        XCTAssertEqual(backend.requests.last?["taskId"] as? String, unconfirmedID)
    }

    func testOpeningRosterWorkDoesNotCancelItsRun() async throws {
        let backend = MockBackend()
        let store = ChatStore(backend: backend)
        await store.refreshConnection()
        let chat = Conversation()
        backend.chats.append(chat)
        backend.requests.removeAll()
        store.openRosterConversation(chat.id)
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(store.library.selectedID, chat.id)
        XCTAssertTrue(backend.requests.contains { $0["op"] as? String == "open" })
        XCTAssertFalse(backend.requests.contains { $0["op"] as? String == "cancel" })
        _ = await store.modelSession.close(unload: false)
    }

    func testNativeCardClicksAndDraggingFollowZoomAndSavedPositionsStayReachable() async throws {
        struct Agent: Identifiable { let id = "local" }
        let suite = "lorca.roster-drag-tests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let arrangement = RosterArrangement(defaults: defaults), zoom = AppZoom(defaults: defaults)
        var clicks = 0
        let root = RosterMap(items: [Agent()], arrangement: arrangement, activate: { _ in clicks += 1 }) { _ in
            Text("Local agent").frame(maxWidth: .infinity).frame(height: 102).background(Color.blue.opacity(0.1))
        }
        let host = ZoomHostingView(rootView: root, zoom: zoom)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 650), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.contentView = host; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        func descendants(_ view: NSView) -> [NSView] { view.subviews.flatMap { [$0] + descendants($0) } }
        func event(_ type: NSEvent.EventType, _ point: CGPoint) -> NSEvent {
            NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                               windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
        }
        for percent in [70, 100, 150] {
            arrangement.reset(); zoom.setPercent(percent)
            try await Task.sleep(for: .milliseconds(150)); host.layoutSubtreeIfNeeded()
            let card = try XCTUnwrap(descendants(host).compactMap { $0 as? RosterCardMouseView }.first)
            let center = card.convert(CGPoint(x: card.bounds.midX, y: card.bounds.midY), to: nil)
            let inHost = host.convert(center, from: nil)
            let hit = host.hitTest(host.convert(inHost, to: host.superview))
            XCTAssertTrue(hit === card, "Card does not receive clicks at \(percent)%")
            let previousClicks = clicks
            window.sendEvent(event(.leftMouseDown, center))
            window.sendEvent(event(.leftMouseUp, CGPoint(x: center.x + 1, y: center.y)))
            XCTAssertEqual(clicks, previousClicks + 1, "A small movement should still click")
            let factor = CGFloat(percent) / 100
            let end = CGPoint(x: center.x + 40 * factor, y: center.y - 35 * factor)
            window.sendEvent(event(.leftMouseDown, center))
            window.sendEvent(event(.leftMouseDragged, CGPoint(x: center.x + 20 * factor, y: center.y - 15 * factor)))
            try await Task.sleep(for: .milliseconds(50)); host.layoutSubtreeIfNeeded()
            window.sendEvent(event(.leftMouseDragged, end))
            window.sendEvent(event(.leftMouseUp, end))
            try await Task.sleep(for: .milliseconds(80)); host.layoutSubtreeIfNeeded()
            XCTAssertEqual(clicks, previousClicks + 1, "Dragging also opened the card")
            let moved = card.convert(CGPoint(x: card.bounds.midX, y: card.bounds.midY), to: nil)
            XCTAssertEqual(moved.x - center.x, 40 * factor, accuracy: 1)
            XCTAssertEqual(moved.y - center.y, -35 * factor, accuracy: 1)
            let reloaded = RosterArrangement(defaults: defaults)
            XCTAssertNotNil(reloaded.positions["local"])
            let narrow = CGSize(width: 200, height: 340)
            let point = reloaded.point(id: "local", index: 0, count: 1, size: narrow)
            XCTAssertGreaterThanOrEqual(point.x, 75); XCTAssertLessThanOrEqual(point.x, 125)
            XCTAssertGreaterThanOrEqual(point.y, 30); XCTAssertLessThanOrEqual(point.y + 102, narrow.height)
        }
        arrangement.reset()
        XCTAssertTrue(RosterArrangement(defaults: defaults).positions.isEmpty)
    }

    func testNativeRosterRendersAtSharedZoomAndWindowSizes() async throws {
        let backend = MockBackend()
        let suite = "lorca.roster-tests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = ChatStore(backend: backend, defaults: defaults)
        var snapshot = try backend.snapshot()
        snapshot.roster = RosterSnapshot(tasks: [
            RosterWork(id: UUID().uuidString, objective: "Review the project and report findings", status: "queued", conversationId: nil, runId: nil, model: nil, summary: nil, updatedAt: 0),
            RosterWork(id: UUID().uuidString, objective: "Inspect the Notes connector", status: "awaiting_approval", conversationId: backend.selected, runId: "run", model: "local-model", summary: nil, updatedAt: 0)
        ])
        store.apply(snapshot)
        let host = ZoomHostingView(rootView: RosterView(store: store).background(Color(nsColor: .windowBackgroundColor)).preferredColorScheme(.dark), zoom: store.uiZoom)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1220, height: 1000), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: .darkAqua)
        window.contentView = host
        window.orderFront(nil)
        defer { window.orderOut(nil) }
        for (width, zoom) in [(1220, 100), (720, 100), (720, 150), (1220, 70)] {
            store.uiZoom.setPercent(zoom)
            window.setContentSize(NSSize(width: width, height: 1000))
            try await Task.sleep(for: .milliseconds(150))
            host.layoutSubtreeIfNeeded()
            XCTAssertEqual(host.frame.width, CGFloat(width), accuracy: 1)
            XCTAssertEqual(host.bounds.width, CGFloat(width), accuracy: 1)
            func descendants(_ view: NSView) -> [NSView] { view.subviews.flatMap { [$0] + descendants($0) } }
            let segmented = try XCTUnwrap(descendants(host).compactMap { $0 as? NSSegmentedControl }.first)
            let originalWidth = segmented.convert(segmented.bounds, to: nil).width
            for segment in [1, 2, 0] {
                segmented.selectedSegment = segment
                _ = segmented.sendAction(segmented.action, to: segmented.target)
                try await Task.sleep(for: .milliseconds(80)); host.layoutSubtreeIfNeeded()
                let current = try XCTUnwrap(descendants(host).compactMap { $0 as? NSSegmentedControl }.first)
                XCTAssertEqual(current.convert(current.bounds, to: nil).width, originalWidth, accuracy: 1, "Filter changed width when switching to an empty list")
            }
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            XCTAssertGreaterThan(png.count, 1000)
            if let directory = ProcessInfo.processInfo.environment["LORCA_ROSTER_RENDER_DIR"] {
                try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
                let output = URL(fileURLWithPath: directory).appendingPathComponent("roster-\(width)-\(zoom).png")
                try png.write(to: output)
                // Window capture includes composited native glass, which cacheDisplay omits.
                let capture = Process()
                capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
                capture.arguments = ["-x", "-o", "-l", String(window.windowNumber), output.path]
                try capture.run()
                capture.waitUntilExit()
                XCTAssertEqual(capture.terminationStatus, 0)
            }
        }
        XCTAssertTrue(backend.requests.isEmpty, "Rendering the roster must not execute work")
        XCTAssertEqual(store.roster.state.tasks.filter(\.needsYou).count, 1)
    }
}
