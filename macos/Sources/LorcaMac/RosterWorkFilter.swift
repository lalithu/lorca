import AppKit
import SwiftUI

// Keep native segment sizing stable when the filtered content changes.
struct RosterWorkFilter: NSViewRepresentable {
    @Binding var selection: String
    private static let labels = ["All", "Needs you", "Done"]
    func makeCoordinator() -> Coordinator { Coordinator(selection: $selection) }
    func makeNSView(context: Context) -> NSSegmentedControl {
        let control = NSSegmentedControl(labels: Self.labels, trackingMode: .selectOne,
                                         target: context.coordinator, action: #selector(Coordinator.changed(_:)))
        control.segmentDistribution = .fillEqually
        control.segmentStyle = .rounded
        control.setContentHuggingPriority(.defaultLow, for: .horizontal)
        control.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        control.setAccessibilityLabel("Filter work")
        return control
    }
    func updateNSView(_ control: NSSegmentedControl, context: Context) {
        context.coordinator.selection = $selection
        control.selectedSegment = Self.labels.firstIndex(of: selection) ?? 0
    }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSSegmentedControl, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 210, height: nsView.intrinsicContentSize.height)
    }
    final class Coordinator: NSObject {
        var selection: Binding<String>
        init(selection: Binding<String>) { self.selection = selection }
        @objc func changed(_ control: NSSegmentedControl) {
            guard RosterWorkFilter.labels.indices.contains(control.selectedSegment) else { return }
            selection.wrappedValue = RosterWorkFilter.labels[control.selectedSegment]
        }
    }
}
