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
        Text(Self.rendered(text, size: size))
            .font(.system(size: size))
            .lineSpacing(4)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    // One Text keeps selection continuous across Markdown elements and leaves
    // wrapping/height entirely to SwiftUI, including while streaming/resizing.
    static func rendered(_ source: String, size: CGFloat) -> AttributedString {
        let blocks = MarkdownBlock.parse(source)
        var result = AttributedString()
        for (index, block) in blocks.enumerated() {
            if index > 0 {
                let previous = blocks[index - 1]
                let sameRow = block.table != nil && block.row == previous.row
                let compact = (block.listDepth > 0 && previous.listDepth > 0
                    && block.listDepth >= previous.listDepth && block.marker != nil)
                    || (block.table != nil && block.table == previous.table)
                result += AttributedString(sameRow ? "\t" : compact ? "\n" : "\n\n")
            }
            if block.isRule {
                var rule = AttributedString("────────────────")
                rule.foregroundColor = .secondary
                result += rule
                continue
            }
            if let marker = block.marker {
                result += AttributedString(String(repeating: "  ", count: max(0, block.listDepth - 1)) + marker + "  ")
            }
            var content = block.text
            if block.isCode {
                // Code parsers retain the fence's final newline; separators
                // already supply paragraph spacing outside the code block.
                while content.characters.last == "\n" { content.removeSubrange(content.characters.index(before: content.endIndex)..<content.endIndex) }
                content.font = .system(size: max(10, size - 1), design: .monospaced)
                content.backgroundColor = .primary.opacity(0.045)
            } else if let level = block.heading {
                content.font = .system(size: size + CGFloat(max(1, 7 - level)) * 1.5, weight: .semibold)
            } else if block.isTableHeader {
                content.font = .system(size: size, weight: .semibold)
            }
            if block.isQuote { result += AttributedString("│ ") }
            result += content
        }
        return result
    }
}
