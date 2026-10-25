import AppKit
import SwiftUI

// Only user scrolling changes follow mode; streamed content/layout changes must not.
struct ChatScrollObserver: NSViewRepresentable {
    var onScroll: (Bool) -> Void

    func makeNSView(context: Context) -> ObserverView { ObserverView(onScroll: onScroll) }
    func updateNSView(_ view: ObserverView, context: Context) { view.onScroll = onScroll }

    final class ObserverView: NSView {
        var onScroll: (Bool) -> Void
        private weak var scrollView: NSScrollView?
        private var lastOrigin: CGFloat = 0
        private var lastHeight: CGFloat = 0

        init(onScroll: @escaping (Bool) -> Void) {
            self.onScroll = onScroll
            super.init(frame: .zero)
        }
        required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            NotificationCenter.default.removeObserver(self)
            scrollView = enclosingScrollView
            guard window != nil, let scrollView else { return }
            lastOrigin = scrollView.contentView.bounds.minY
            lastHeight = scrollView.documentView?.bounds.height ?? 0
            scrollView.contentView.postsBoundsChangedNotifications = true
            NotificationCenter.default.addObserver(self, selector: #selector(positionChanged),
                name: NSView.boundsDidChangeNotification, object: scrollView.contentView)
            NotificationCenter.default.addObserver(self, selector: #selector(started),
                name: NSScrollView.willStartLiveScrollNotification, object: scrollView)
            for name in [NSScrollView.didLiveScrollNotification, NSScrollView.didEndLiveScrollNotification] {
                NotificationCenter.default.addObserver(self, selector: #selector(scrolled), name: name, object: scrollView)
            }
        }

        // Scrollbar/AX scrolling may not emit live-scroll notifications. Moving toward
        // older content with unchanged geometry is still a user scroll, not content growth.
        @objc private func positionChanged() {
            guard let scrollView, let document = scrollView.documentView else { return }
            let origin = scrollView.contentView.bounds.minY, height = document.bounds.height
            let movedBack = document.isFlipped ? origin < lastOrigin - 1 : origin > lastOrigin + 1
            if abs(height - lastHeight) < 1 {
                if movedBack { onScroll(false) }
                let visible = scrollView.documentVisibleRect
                let gap = document.isFlipped ? document.bounds.maxY - visible.maxY : visible.minY - document.bounds.minY
                if abs(origin - lastOrigin) > 1 && gap <= 16 { onScroll(true) }
            }
            lastOrigin = origin; lastHeight = height
        }
        @objc private func started() { onScroll(false) }
        @objc private func scrolled() {
            guard let scrollView, let document = scrollView.documentView else { return }
            let visible = scrollView.documentVisibleRect
            let gap = document.isFlipped ? document.bounds.maxY - visible.maxY : visible.minY - document.bounds.minY
            onScroll(gap <= 16)
        }
        deinit { NotificationCenter.default.removeObserver(self) }
    }
}
