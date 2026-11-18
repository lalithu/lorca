import Foundation

// A local formatting preview for trying Ems interaction. No model is called.
struct NotesTableSuggestion: Equatable {
    let sourceRange: NSRange
    let sourceText: String
    let headers: [String]
    let rows: [[String]]

    static func detect(in text: String) -> NotesTableSuggestion? {
        let source = text as NSString
        guard let pattern = try? NSRegularExpression(pattern: #"(?im)^[ \t]*table(?:[ \t]+with[ \t]+data)?[ \t]*:[ \t]*"#),
              let cue = pattern.matches(in: text, range: NSRange(location: 0, length: source.length)).last else { return nil }
        let tailStart = NSMaxRange(cue.range)
        let tail = source.substring(from: tailStart) as NSString
        var lines: [String] = []
        var consumed = 0
        while consumed < tail.length {
            let range = tail.lineRange(for: NSRange(location: consumed, length: 0))
            let line = tail.substring(with: range).trimmingCharacters(in: .whitespacesAndNewlines)
            if line.isEmpty && !lines.isEmpty { break }
            consumed = NSMaxRange(range)
            if !line.isEmpty { lines.append(line) }
            if lines.count > 20 || consumed > 4_000 { return nil }
        }
        guard !lines.isEmpty else { return nil }
        let delimiter: Character? = lines[0].contains("|") ? "|" : lines[0].contains("\t") ? "\t" : nil
        var cells = lines.map { line -> [String] in
            guard let delimiter else { return [line] }
            var pieces = line.split(separator: delimiter, omittingEmptySubsequences: false)
                .map { $0.trimmingCharacters(in: .whitespaces) }
            if delimiter == "|", pieces.first == "" { pieces.removeFirst() }
            if delimiter == "|", pieces.last == "" { pieces.removeLast() }
            return pieces
        }
        let headers: [String]
        if delimiter != nil, cells.count > 1 {
            headers = cells.removeFirst()
            cells.removeAll { row in row.allSatisfy { !$0.isEmpty && $0.allSatisfy { "-: ".contains($0) } } }
        } else {
            headers = ["Details"]
        }
        guard !headers.isEmpty, headers.count <= 6, !cells.isEmpty else { return nil }
        // Never silently drop text when delimiters differ across rows.
        guard cells.allSatisfy({ $0.count <= headers.count }) else { return nil }
        let rows = cells.map { row in row + Array(repeating: "", count: headers.count - row.count) }
        let range = NSRange(location: cue.range.location, length: tailStart + consumed - cue.range.location)
        return NotesTableSuggestion(sourceRange: range, sourceText: source.substring(with: range), headers: headers, rows: rows)
    }

    func stillMatches(_ text: String) -> Bool {
        let source = text as NSString
        return sourceRange.location >= 0 && NSMaxRange(sourceRange) <= source.length
            && source.substring(with: sourceRange) == sourceText
    }
}
