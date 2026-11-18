import AppKit
import SwiftUI

struct NotesTablePreview: View {
    let suggestion: NotesTableSuggestion
    let insert: () -> Void
    let dismiss: () -> Void
    var attribution = "Local preview · model not connected"

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Label("Table", systemImage: "tablecells").font(.system(size: 12, weight: .medium))
                Spacer()
                Button(action: dismiss) { Image(systemName: "xmark") }
                    .buttonStyle(.plain).help("Dismiss suggestion").accessibilityLabel("Dismiss suggestion")
            }
            ScrollView([.vertical, .horizontal]) {
                Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
                    GridRow {
                        ForEach(suggestion.headers.indices, id: \.self) { column in
                            cell(suggestion.headers[column], header: true)
                        }
                    }
                    ForEach(suggestion.rows.indices, id: \.self) { row in
                        GridRow {
                            ForEach(suggestion.headers.indices, id: \.self) { column in
                                cell(suggestion.rows[row][column], header: false)
                            }
                        }
                    }
                }
            }
            .frame(maxHeight: 240)
            HStack {
                Text(attribution)
                    .font(.system(size: 10)).foregroundStyle(.secondary)
                Spacer()
                Button("Use table", action: insert).controlSize(.small)
                    .accessibilityIdentifier("ems-use-table")
            }
        }
        .padding(16).frame(width: 390)
    }

    private func cell(_ text: String, header: Bool) -> some View {
        Text(text.isEmpty ? " " : text)
            .font(.system(size: 11, weight: header ? .medium : .regular))
            .textSelection(.enabled)
            .frame(minWidth: 90, maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10).padding(.vertical, 8)
            .background(header ? Color.primary.opacity(0.05) : .clear)
            .overlay(alignment: .bottom) { Divider() }
    }
}

enum NotesNativeTable {
    static func attributed(_ suggestion: NotesTableSuggestion) -> NSAttributedString {
        let table = NSTextTable()
        table.numberOfColumns = suggestion.headers.count
        table.collapsesBorders = true
        table.setContentWidth(CGFloat(max(1, suggestion.headers.count)) * 130, type: .absoluteValueType)
        let output = NSMutableAttributedString()
        for (row, cells) in ([suggestion.headers] + suggestion.rows).enumerated() {
            for (column, value) in cells.enumerated() {
                let block = NSTextTableBlock(table: table, startingRow: row, rowSpan: 1, startingColumn: column, columnSpan: 1)
                block.setWidth(0.5, type: .absoluteValueType, for: .border)
                block.setBorderColor(.separatorColor)
                block.setWidth(4, type: .absoluteValueType, for: .padding)
                block.setContentWidth(130, type: .absoluteValueType)
                let paragraph = NSMutableParagraphStyle()
                paragraph.textBlocks = [block]
                paragraph.lineSpacing = 2
                output.append(NSAttributedString(string: value + "\n", attributes: [
                    .font: NSFont.systemFont(ofSize: 14, weight: row == 0 ? .semibold : .regular),
                    .foregroundColor: NSColor.labelColor,
                    .paragraphStyle: paragraph
                ]))
            }
        }
        // Keep a normal paragraph after the table, ready for more writing.
        output.append(NSAttributedString(string: "\n", attributes: MarkdownNoteStyle.base(size: 15)))
        return output
    }
}
