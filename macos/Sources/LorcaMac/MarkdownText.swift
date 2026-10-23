import SwiftUI

// Foundation parses the Markdown; these views supply its block-level layout.
struct MarkdownBlock: Identifiable {
    let id: Int
    var text: AttributedString
    let intents: [PresentationIntent.IntentType]
    var marker: String?

    var heading: Int? {
        for intent in intents { if case .header(let level) = intent.kind { return level } }
        return nil
    }
    var isCode: Bool { intents.contains { if case .codeBlock = $0.kind { return true }; return false } }
    var isRule: Bool { intents.contains { $0.kind == .thematicBreak } }
    var isQuote: Bool { intents.contains { $0.kind == .blockQuote } }
    var listDepth: Int { intents.filter { $0.kind == .orderedList || $0.kind == .unorderedList }.count }
    var table: Int? { intents.first { if case .table = $0.kind { return true }; return false }?.identity }
    var row: Int? {
        intents.first { if case .tableRow = $0.kind { return true }; return $0.kind == .tableHeaderRow }?.identity
    }
    var isTableHeader: Bool { intents.contains { $0.kind == .tableHeaderRow } }

    static func parse(_ source: String) -> [Self] {
        guard let parsed = try? AttributedString(markdown: source) else {
            return [Self(id: 0, text: AttributedString(source), intents: [])]
        }
        var seenItems = Set<Int>()
        return parsed.runs[\.presentationIntent].enumerated().map { index, run in
            let (intent, range) = run
            let intents = intent?.components ?? []
            var block = Self(id: index, text: AttributedString(parsed[range]), intents: intents)
            block.text.presentationIntent = nil
            if let item = intents.first(where: { if case .listItem = $0.kind { return true }; return false }),
               seenItems.insert(item.identity).inserted,
               case .listItem(let ordinal) = item.kind {
                let ordered = intents.first { $0.kind == .orderedList || $0.kind == .unorderedList }?.kind == .orderedList
                block.marker = ordered ? "\(ordinal)." : "•"
            }
            return block
        }
    }
}

struct MarkdownText: View {
    let text: String
    let size: CGFloat

    var body: some View {
        let blocks = MarkdownBlock.parse(text)
        VStack(alignment: .leading, spacing: 12) {
            ForEach(blocks) { block in
                if let table = block.table {
                    if blocks.first(where: { $0.table == table })?.id == block.id {
                        tableView(blocks.filter { $0.table == table })
                    }
                } else {
                    blockView(block)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func blockView(_ block: MarkdownBlock) -> some View {
        if block.isRule {
            Divider().padding(.vertical, 4)
        } else if block.isCode {
            ScrollView(.horizontal) {
                Text(block.text).font(.system(size: max(10, size - 1), design: .monospaced))
                    .lineSpacing(4).padding(12)
            }
            .background(.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 9))
        } else {
            HStack(alignment: .top, spacing: 8) {
                if block.isQuote { Rectangle().fill(.secondary.opacity(0.4)).frame(width: 2) }
                if block.listDepth > 0 {
                    Text(block.marker ?? "").frame(minWidth: 18, alignment: .trailing)
                }
                Text(block.text)
                    .font(.system(size: headingSize(block.heading), weight: block.heading == nil ? .regular : .semibold))
                    .lineSpacing(5)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .font(.system(size: size))
            .padding(.leading, CGFloat(max(0, block.listDepth - 1)) * 18)
            .padding(.top, block.heading == nil ? 0 : 5)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func headingSize(_ level: Int?) -> CGFloat {
        guard let level else { return size }
        return size + CGFloat(max(1, 7 - level)) * 1.5
    }

    private func tableView(_ cells: [MarkdownBlock]) -> some View {
        let rows = cells.reduce(into: [Int]()) { result, cell in
            if let row = cell.row, !result.contains(row) { result.append(row) }
        }
        return ScrollView(.horizontal) {
            Grid(alignment: .leading, horizontalSpacing: 20, verticalSpacing: 9) {
                ForEach(rows, id: \.self) { row in
                    GridRow {
                        ForEach(cells.filter { $0.row == row }) { cell in
                            Text(cell.text).font(.system(size: size, weight: cell.isTableHeader ? .semibold : .regular))
                        }
                    }
                    if row == rows.first { Divider().gridCellUnsizedAxes(.horizontal) }
                }
            }
            .padding(12)
        }
        .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 9))
    }
}
