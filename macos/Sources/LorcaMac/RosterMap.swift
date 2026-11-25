import AppKit
import SwiftUI

// UI preference only. Normalized positions stay reachable when the viewport changes.
@MainActor final class RosterArrangement: ObservableObject {
    struct Position: Codable, Sendable { var x: Double; var y: Double }
    static let preferenceKey = "roster.cardPositions"
    @Published private(set) var positions: [String: Position]
    private let defaults: UserDefaults
    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        positions = defaults.data(forKey: Self.preferenceKey)
            .flatMap { try? JSONDecoder().decode([String: Position].self, from: $0) } ?? [:]
    }
    nonisolated static let cardHeight: CGFloat = 102
    nonisolated static func canvasWidth(_ width: CGFloat) -> CGFloat { width.isFinite ? max(1, width) : 450 }
    nonisolated static func cardWidth(_ width: CGFloat) -> CGFloat { min(150, canvasWidth(width)) }
    nonisolated static func height(width: CGFloat, count: Int) -> CGFloat {
        let columns = max(1, Int(min(CGFloat(max(1, count)), (canvasWidth(width) + 22) / 172)))
        return max(340, CGFloat((count + columns - 1) / columns) * 132 + 60)
    }
    func point(id: String, index: Int, count: Int, size: CGSize) -> CGPoint {
        Self.resolvedPoint(id: id, index: index, count: count, size: size, positions: positions)
    }
    nonisolated static func resolvedPoint(id: String, index: Int, count: Int, size: CGSize, positions: [String: Position]) -> CGPoint {
        let width = Self.canvasWidth(size.width), card = Self.cardWidth(width)
        if let saved = positions[id], saved.x.isFinite, saved.y.isFinite {
            return CGPoint(x: card / 2 + min(1, max(0, saved.x)) * max(0, width - card),
                           y: 30 + min(1, max(0, saved.y)) * max(0, size.height - Self.cardHeight - 30))
        }
        let columns = max(1, Int((width + 22) / 172)), row = index / columns
        let rowCount = min(columns, count - row * columns)
        let rowWidth = CGFloat(rowCount) * card + CGFloat(max(0, rowCount - 1)) * 22
        return CGPoint(x: (width - rowWidth) / 2 + CGFloat(index % columns) * (card + 22) + card / 2,
                       y: 30 + CGFloat(row) * 132)
    }
    func move(_ id: String, to point: CGPoint, size: CGSize) {
        guard point.x.isFinite, point.y.isFinite else { return }
        let card = Self.cardWidth(size.width)
        positions[id] = Position(x: min(1, max(0, (point.x - card / 2) / max(1, size.width - card))),
                                 y: min(1, max(0, (point.y - 30) / max(1, size.height - Self.cardHeight - 30))))
    }
    func save() { if let data = try? JSONEncoder().encode(positions) { defaults.set(data, forKey: Self.preferenceKey) } }
    func reset() { positions = [:]; defaults.removeObject(forKey: Self.preferenceKey) }
}

struct RosterMap<Item: Identifiable, Card: View>: View where Item.ID == String {
    let items: [Item]
    @ObservedObject var arrangement: RosterArrangement
    let activate: (Item) -> Void
    @ViewBuilder var card: (Item) -> Card
    @State private var size = CGSize(width: 450, height: 340)
    @State private var dragOrigin: CGPoint?
    @State private var dragging: String?

    var body: some View {
        RosterFreeLayout(ids: items.map(\.id), positions: arrangement.positions) {
            ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                card(item)
                    .contentShape(RoundedRectangle(cornerRadius: 12))
                    .overlay {
                        RosterCardInteraction(begin: {
                            dragOrigin = arrangement.point(id: item.id, index: index, count: items.count, size: size)
                        }, move: { delta in
                            guard let origin = dragOrigin else { return }
                            dragging = item.id
                            arrangement.move(item.id, to: CGPoint(x: origin.x + delta.width, y: origin.y + delta.height), size: size)
                        }, end: {
                            arrangement.save(); dragging = nil; dragOrigin = nil
                        }, activate: { activate(item) })
                        .accessibilityHidden(true)
                    }
                    .accessibilityAddTraits(.isButton)
                    .accessibilityAction { activate(item) }
                    .zIndex(dragging == item.id ? 1 : 0)
            }
        }
        .onGeometryChange(for: CGSize.self) { $0.size } action: { size = $0 }
        .background {
            GeometryReader { geometry in
                Canvas { context, canvas in
                    var path = Path()
                    for (index, item) in items.enumerated() {
                        let point = arrangement.point(id: item.id, index: index, count: items.count, size: canvas)
                        path.move(to: CGPoint(x: canvas.width / 2, y: 0))
                        path.addCurve(to: point,
                                      control1: CGPoint(x: canvas.width / 2, y: point.y / 2),
                                      control2: CGPoint(x: point.x, y: point.y / 2))
                    }
                    context.stroke(path, with: .color(Color.lorcaBlue.opacity(0.25)),
                                   style: StrokeStyle(lineWidth: 1, lineCap: .round, lineJoin: .round))
                }.frame(width: geometry.size.width, height: geometry.size.height)
            }.allowsHitTesting(false).accessibilityHidden(true)
        }
    }
}

private struct RosterFreeLayout: Layout {
    let ids: [String]
    let positions: [String: RosterArrangement.Position]
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = RosterArrangement.canvasWidth(proposal.width ?? 450)
        return CGSize(width: width, height: RosterArrangement.height(width: width, count: ids.count))
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (index, view) in subviews.enumerated() {
            let point = RosterArrangement.resolvedPoint(id: ids[index], index: index, count: ids.count, size: bounds.size, positions: positions)
            view.place(at: CGPoint(x: bounds.minX + point.x, y: bounds.minY + point.y), anchor: .top,
                       proposal: ProposedViewSize(width: RosterArrangement.cardWidth(bounds.width), height: RosterArrangement.cardHeight))
        }
    }
}

// Native mouse coordinates are converted through the shared zoom hosting view.
// One interaction owns click/drag discrimination, so a drag cannot open a sheet.
struct RosterCardInteraction: NSViewRepresentable {
    var begin: () -> Void
    var move: (CGSize) -> Void
    var end: () -> Void
    var activate: () -> Void
    func makeNSView(context: Context) -> RosterCardMouseView { RosterCardMouseView() }
    func updateNSView(_ view: RosterCardMouseView, context: Context) {
        view.begin = begin; view.move = move; view.end = end; view.activate = activate
    }
}
final class RosterCardMouseView: NSView {
    var begin: () -> Void = {}
    var move: (CGSize) -> Void = { _ in }
    var end: () -> Void = {}
    var activate: () -> Void = {}
    private var down: NSPoint?
    private var dragged = false
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func resetCursorRects() { addCursorRect(bounds, cursor: .openHand) }
    override func mouseDown(with event: NSEvent) {
        down = event.locationInWindow; dragged = false; begin()
    }
    override func mouseDragged(with event: NSEvent) {
        guard let down else { return }
        let start = convert(down, from: nil), current = convert(event.locationInWindow, from: nil)
        let delta = CGSize(width: current.x - start.x, height: current.y - start.y)
        guard dragged || hypot(delta.width, delta.height) >= 6 else { return }
        dragged = true; NSCursor.closedHand.set(); move(delta)
    }
    override func mouseUp(with event: NSEvent) {
        guard down != nil else { return }
        if dragged { mouseDragged(with: event); end() } else { end(); activate() }
        down = nil; dragged = false; NSCursor.openHand.set()
    }
}
