import AppKit
import SwiftUI

/// Propose the remaining width to the main pane at each step of the slide.
struct DashboardColumns: Layout {
    var sidebarWidth: CGFloat
    var occupiedWidth: CGFloat

    var animatableData: AnimatablePair<CGFloat, CGFloat> {
        get { AnimatablePair(sidebarWidth, occupiedWidth) }
        set { (sidebarWidth, occupiedWidth) = (newValue.first, newValue.second) }
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        proposal.replacingUnspecifiedDimensions()
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews[0].place(
            at: CGPoint(x: bounds.minX + occupiedWidth - sidebarWidth, y: bounds.minY),
            anchor: .topLeading,
            proposal: ProposedViewSize(width: sidebarWidth, height: bounds.height)
        )
        subviews[1].place(
            at: CGPoint(x: bounds.minX + occupiedWidth, y: bounds.minY),
            anchor: .topLeading,
            proposal: ProposedViewSize(width: max(0, bounds.width - occupiedWidth), height: bounds.height)
        )
    }
}

/// A transparent hit area. Window coordinates stay stable while the edge moves.
struct SidebarResizeEdge: NSViewRepresentable {
    var onStart: () -> Void
    var onDrag: (CGFloat) -> Void
    var onEnd: (CGFloat) -> Void

    func makeNSView(context: Context) -> EdgeView { EdgeView() }

    func updateNSView(_ view: EdgeView, context: Context) {
        view.onStart = onStart
        view.onDrag = onDrag
        view.onEnd = onEnd
    }

    final class EdgeView: NSView {
        var onStart: (() -> Void)?
        var onDrag: ((CGFloat) -> Void)?
        var onEnd: ((CGFloat) -> Void)?
        private var startX: CGFloat?

        override var mouseDownCanMoveWindow: Bool { false }

        override func resetCursorRects() {
            addCursorRect(bounds, cursor: .resizeLeftRight)
        }

        override func mouseDown(with event: NSEvent) {
            startX = event.locationInWindow.x
            onStart?()
        }

        override func mouseDragged(with event: NSEvent) {
            guard let startX else { return }
            onDrag?(event.locationInWindow.x - startX)
        }

        override func mouseUp(with event: NSEvent) {
            guard let startX else { return }
            self.startX = nil
            onEnd?(event.locationInWindow.x - startX)
        }
    }
}
