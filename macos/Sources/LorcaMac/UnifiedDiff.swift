import Foundation

struct AgentDiffLine: Identifiable, Equatable {
    enum Kind { case addition, removal, context, header }
    let id: Int
    let text: String
    var oldLine: Int? = nil
    var newLine: Int? = nil
    var kind: Kind

    static func parse(_ text: String) -> [Self] { UnifiedDiff(text).lines }
}

// The edit approval contains both complete versions as a replacement hunk.
// Reconcile equal lines locally; no workspace reads, backend changes or tools.
struct UnifiedDiff {
    struct Row: Identifiable {
        enum Content { case line(AgentDiffLine), gap([AgentDiffLine]) }
        let id: Int
        let content: Content
    }
    private struct SourceLine: Hashable {
        let text: String
        var hasNewline = true
    }
    let lines: [AgentDiffLine]

    init(_ diff: String) {
        var raw = diff.components(separatedBy: "\n")
        if raw.last == "" { raw.removeLast() }
        var result: [AgentDiffLine] = [], index = 0
        var old = 0, new = 0, inHunk = false
        var oldRemaining = 0, newRemaining = 0
        let header = try! NSRegularExpression(pattern: #"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@"#)
        func append(_ text: String, _ kind: AgentDiffLine.Kind) {
            result.append(AgentDiffLine(id: result.count, text: text,
                oldLine: kind == .context || kind == .removal ? old : nil,
                newLine: kind == .context || kind == .addition ? new : nil, kind: kind))
            if kind == .context || kind == .removal { old += 1 }
            if kind == .context || kind == .addition { new += 1 }
        }
        func emit(_ line: SourceLine, _ kind: AgentDiffLine.Kind) {
            let prefix = kind == .addition ? "+" : kind == .removal ? "-" : " "
            append(prefix + line.text, kind)
            if !line.hasNewline { append("\\ No newline at end of file", .header) }
        }
        while index < raw.count {
            let text = raw[index]
            if let match = header.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) {
                let value = text as NSString
                oldRemaining = match.range(at: 2).location == NSNotFound ? 1 : Int(value.substring(with: match.range(at: 2))) ?? 0
                newRemaining = match.range(at: 4).location == NSNotFound ? 1 : Int(value.substring(with: match.range(at: 4))) ?? 0
                old = Int(value.substring(with: match.range(at: 1)))! + (oldRemaining == 0 ? 1 : 0)
                new = Int(value.substring(with: match.range(at: 3)))! + (newRemaining == 0 ? 1 : 0)
                inHunk = true; append(text, .header); index += 1; continue
            }
            if inHunk && (oldRemaining > 0 || newRemaining > 0), text.first == "-" || text.first == "+" {
                var before: [SourceLine] = [], after: [SourceLine] = []
                while index < raw.count {
                    let line = raw[index]
                    guard (line.first == "-" && oldRemaining > 0) || (line.first == "+" && newRemaining > 0) else { break }
                    let removed = line.first == "-"
                    var source = SourceLine(text: String(line.dropFirst()))
                    index += 1
                    if index < raw.count && raw[index] == "\\ No newline at end of file" {
                        source.hasNewline = false; index += 1
                    }
                    if removed { before.append(source); oldRemaining -= 1 }
                    else { after.append(source); newRemaining -= 1 }
                }
                // Trim the common edges before the standard Myers difference.
                // This keeps large files with small edits cheap to render.
                var prefix = 0, suffix = 0
                while prefix < min(before.count, after.count), before[prefix] == after[prefix] { prefix += 1 }
                while suffix < min(before.count, after.count) - prefix,
                      before[before.count - suffix - 1] == after[after.count - suffix - 1] { suffix += 1 }
                before.prefix(prefix).forEach { emit($0, .context) }
                let left = Array(before[prefix..<(before.count - suffix)])
                let right = Array(after[prefix..<(after.count - suffix)])
                // Completely different large files need no edit-graph search.
                let disjoint = Set(left).isDisjoint(with: Set(right))
                let changes = disjoint ? nil : right.difference(from: left)
                let removed = Set(changes?.removals.map { change -> Int in if case .remove(let offset, _, _) = change { return offset }; return -1 } ?? Array(left.indices))
                let inserted = Set(changes?.insertions.map { change -> Int in if case .insert(let offset, _, _) = change { return offset }; return -1 } ?? Array(right.indices))
                var a = 0, b = 0
                while a < left.count || b < right.count {
                    if a < left.count && removed.contains(a) { emit(left[a], .removal); a += 1 }
                    else if b < right.count && inserted.contains(b) { emit(right[b], .addition); b += 1 }
                    else if a < left.count && b < right.count { emit(left[a], .context); a += 1; b += 1 }
                }
                before.suffix(suffix).forEach { emit($0, .context) }
                continue
            }
            if inHunk && oldRemaining > 0 && newRemaining > 0 && text.first == " " {
                append(text, .context); oldRemaining -= 1; newRemaining -= 1
            } else {
                append(text, .header)
                if !text.hasPrefix("\\") { inHunk = false }
            }
            index += 1
        }
        lines = result
    }

    func rows(context: Int = 3) -> [Row] {
        var rows: [Row] = [], index = 0
        while index < lines.count {
            if lines[index].kind != .context {
                rows.append(Row(id: lines[index].id, content: .line(lines[index]))); index += 1; continue
            }
            let start = index
            while index < lines.count && lines[index].kind == .context { index += 1 }
            let previousChange = start > 0 && [.addition, .removal].contains(lines[start - 1].kind)
                || start > 1 && lines[start - 1].text.hasPrefix("\\") && [.addition, .removal].contains(lines[start - 2].kind)
            let nextChange = index < lines.count && [.addition, .removal].contains(lines[index].kind)
            let prefixEnd = min(index, start + (previousChange ? context : 0))
            let suffixStart = max(prefixEnd, index - (nextChange ? context : 0))
            lines[start..<prefixEnd].forEach { rows.append(Row(id: $0.id, content: .line($0))) }
            if prefixEnd < suffixStart {
                let hidden = Array(lines[prefixEnd..<suffixStart])
                rows.append(Row(id: hidden[0].id, content: .gap(hidden)))
            }
            lines[suffixStart..<index].forEach { rows.append(Row(id: $0.id, content: .line($0))) }
        }
        return rows
    }
}
