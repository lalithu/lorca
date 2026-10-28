import AppKit
import Combine
import SwiftUI

/// One UI-only preference shared by every content window and presentation.
@MainActor
final class AppZoom: NSObject, ObservableObject, NSMenuItemValidation {
    static let preferenceKey = "appUIZoomPercent"
    @Published private(set) var percent: Int
    var factor: CGFloat { CGFloat(percent) / 100 }
    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        let saved = defaults.object(forKey: Self.preferenceKey) as? Int ?? 100
        percent = Self.normalized(saved)
        super.init()
    }

    private static func normalized(_ value: Int) -> Int {
        ((min(150, max(70, value)) + 5) / 10) * 10
    }

    func setPercent(_ value: Int) {
        let next = Self.normalized(value)
        guard percent != next else { return }
        percent = next
        defaults.set(next, forKey: Self.preferenceKey)
    }
    @objc func zoomIn(_ sender: Any? = nil) { setPercent(percent + 10) }
    @objc func zoomOut(_ sender: Any? = nil) { setPercent(percent - 10) }
    @objc func actualSize(_ sender: Any? = nil) { setPercent(100) }

    func addCommands(to menu: NSMenu) {
        for (title, action, key) in [("Zoom In", #selector(zoomIn(_:)), "="),
                                     ("Zoom Out", #selector(zoomOut(_:)), "-"),
                                     ("Actual Size", #selector(actualSize(_:)), "0")] {
            let item = menu.addItem(withTitle: title, action: action, keyEquivalent: key)
            item.target = self
        }
        // '+' needs Shift on some layouts; retain '=' as the primary shortcut.
        let plus = menu.addItem(withTitle: "Zoom In", action: #selector(zoomIn(_:)), keyEquivalent: "+")
        plus.target = self
        plus.isHidden = true
        plus.allowsKeyEquivalentWhenHidden = true
    }
    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(zoomIn(_:)): return percent < 150
        case #selector(zoomOut(_:)): return percent > 70
        default: return true
        }
    }
}

/// Measure at the logical viewport before applying the visual transform. SwiftUI
/// owns both layout and event conversion, including embedded AppKit controls.
/// A render-only scale or an external AppKit bounds transform would leave the
/// hosting view's hit-test graph out of sync with the displayed controls.
struct ZoomLayout: Layout {
    let factor: CGFloat
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let content = subviews.first else { return .zero }
        let logical = content.sizeThatFits(Self.logical(proposal, factor: factor))
        return CGSize(width: logical.width * factor, height: logical.height * factor)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: bounds.origin, anchor: .topLeading,
                             proposal: Self.logical(ProposedViewSize(bounds.size), factor: factor))
    }
    static func logical(_ proposal: ProposedViewSize, factor: CGFloat) -> ProposedViewSize {
        ProposedViewSize(width: proposal.width.map { $0 / factor }, height: proposal.height.map { $0 / factor })
    }
}

/// The same measured surface is used by windows and SwiftUI presentations;
/// sheets retain their original environment, focus and native dismissal action.
struct ZoomContent<Content: View>: View {
    @ObservedObject var zoom: AppZoom
    let content: Content
    init(zoom: AppZoom, @ViewBuilder content: () -> Content) {
        self.zoom = zoom
        self.content = content()
    }
    var body: some View {
        ZoomLayout(factor: zoom.factor) {
            content.scaleEffect(zoom.factor, anchor: .topLeading)
        }
    }
}

@MainActor
final class ZoomHostingView<Content: View>: NSHostingView<ZoomContent<Content>> {
    init(rootView: Content, zoom: AppZoom) {
        super.init(rootView: ZoomContent(zoom: zoom) { rootView })
        sizingOptions = []
    }
    @available(*, unavailable)
    required init(rootView: ZoomContent<Content>) { fatalError("Use init(rootView:zoom:)") }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
}
