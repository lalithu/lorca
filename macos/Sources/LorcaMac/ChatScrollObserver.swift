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
            NotificationCenter.default.addObserver(self, selector: #selector(started),
                name: NSScrollView.willStartLiveScrollNotification, object: scrollView)
            for name in [NSScrollView.didLiveScrollNotification, NSScrollView.didEndLiveScrollNotification] {
                NotificationCenter.default.addObserver(self, selector: #selector(scrolled), name: name, object: scrollView)
            }
        }

        @objc private func started() { onScroll(false) }
        @objc private func scrolled() {
            guard let scrollView, let document = scrollView.documentView else { return }
            let visible = scrollView.documentVisibleRect
            let gap = document.isFlipped ? document.bounds.maxY - visible.maxY : visible.minY - document.bounds.minY
            onScroll(gap <= 32)
        }
        deinit { NotificationCenter.default.removeObserver(self) }
    }
}
