import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class ChatScrollTests: XCTestCase {
    private final class Document: NSView {
        override var isFlipped: Bool { true }
    }

    func testManualScrollControlsFollowingButContentGrowthDoesNot() {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 200),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let scroll = NSScrollView(frame: window.contentLayoutRect)
        let document = Document(frame: NSRect(x: 0, y: 0, width: 400, height: 1000))
        scroll.documentView = document
        var follows = true
        let observer = ChatScrollObserver.ObserverView { follows = $0 }
        document.addSubview(observer)
        window.contentView = scroll
        defer { window.close() }

        NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: scroll)
        XCTAssertFalse(follows, "Scrolling back suspends following")
        document.setFrameSize(NSSize(width: 400, height: 1200))
        XCTAssertFalse(follows, "Streamed content must not resume following")
        scroll.contentView.scroll(to: NSPoint(x: 0, y: document.bounds.maxY - scroll.contentView.bounds.height))
        NotificationCenter.default.post(name: NSScrollView.didLiveScrollNotification, object: scroll)
        XCTAssertTrue(follows, "Returning to the bottom resumes following")
        NotificationCenter.default.post(name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        XCTAssertFalse(follows, "A new user gesture can interrupt following immediately")
    }
}
