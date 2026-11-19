import AppKit
import UniformTypeIdentifiers

struct NotesUserEdit: Codable, Identifiable, Equatable {
    let id: UUID
    let owner: UUID
    var location: Int
    var length: Int
    var range: NSRange { NSRange(location: location, length: length) }
}

extension NotesPassage {
    static func combined(in text: String, blocks: [NotesModelBlock], edits: [NotesUserEdit]) -> [NotesPassage] {
        let value = text as NSString
        var result = passages(in: text, excluding: blocks.map(\.range))
        for edit in edits where edit.length > 0 && edit.location >= 0 && NSMaxRange(edit.range) <= value.length {
            guard let owner = blocks.first(where: { $0.id == edit.owner }) else { continue }
            let paragraph = value.paragraphRange(for: edit.range)
            let raw = value.substring(with: paragraph), source = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !source.isEmpty, source.utf8.count <= 12_000 else { continue }
            let local = (raw as NSString).range(of: source)
            let range = NSRange(location: paragraph.location + local.location, length: local.length)
            guard !result.contains(where: { $0.range == range }) else { continue }
            let start = min(owner.location, range.location)
            let prefix = value.substring(with: NSRange(location: start, length: max(0, NSMaxRange(range) - start)))
            let headings = try! NSRegularExpression(pattern: #"(?m)^#{1,6}[ \t]+[^\n]+"#)
            let lastHeading = headings.matches(in: prefix, range: NSRange(location: 0, length: (prefix as NSString).length)).last
            let section = lastHeading.map { (prefix as NSString).substring(from: $0.range.location) } ?? prefix
            let context = String(owner.source.prefix(500)) + "\n\nNearby section:\n" + String(section.suffix(1800))
            result.append(NotesPassage(source: source, range: range, context: context))
        }
        return result.filter { passage in result.filter { $0.source == passage.source }.count == 1 }.sorted { $0.range.location < $1.range.location }
    }
}

enum NotesChecklist {
    static func render(_ text: String) -> String {
        var sources = false
        return text.components(separatedBy: "\n").map { line in
            let value = line.trimmingCharacters(in: .whitespaces)
            if value.hasPrefix("Sources") { sources = true }
            guard !sources, !value.isEmpty, !value.hasPrefix("#") else { return line }
            let checked = value.hasPrefix("☑") || value.lowercased().hasPrefix("- [x]") || value.lowercased().hasPrefix("* [x]")
            let pattern = #"^(?:[-*+]\s+(?:\[[ xX]\]\s*)?|\d+[.)]\s+|[☐☑]\s*)"#
            let body = value.replacingOccurrences(of: pattern, with: "", options: .regularExpression)
            return (checked ? "☑ " : "☐ ") + body
        }.joined(separator: "\n")
    }
}

extension NotesNativeTextView {
    func userEdits() -> [NotesUserEdit] {
        guard let storage = textStorage else { return [] }
        var result: [UUID: NotesUserEdit] = [:]
        storage.enumerateAttribute(.notesUserEdit, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            guard let text = value as? String, let id = UUID(uuidString: text), var edit = userEditMetadata[id] else { return }
            let union = result[id].map { NSUnionRange($0.range, range) } ?? range
            edit.location = union.location; edit.length = union.length; result[id] = edit
        }
        return result.values.sorted { $0.location < $1.location }
    }

    func styleChecklistAndUserEdits() {
        guard let storage = textStorage else { return }
        let value = string as NSString
        let pattern = try! NSRegularExpression(pattern: #"(?m)^[ \t]*([☐☑]) ([^\n]*)"#)
        for match in pattern.matches(in: string, range: NSRange(location: 0, length: value.length)) {
            let checked = value.substring(with: match.range(at: 1)) == "☑"
            storage.addAttributes([.font:NSFont.systemFont(ofSize: 17), .foregroundColor:NSColor.controlAccentColor], range: match.range(at: 1))
            if checked { storage.addAttributes([.foregroundColor:NSColor.secondaryLabelColor, .strikethroughStyle:NSUnderlineStyle.single.rawValue], range: match.range(at: 2)) }
        }
        for edit in userEdits() {
            storage.addAttributes([.foregroundColor:NSColor.systemOrange, .font:ShellTypography.nativeSerif(16)], range: edit.range)
        }
    }

    @discardableResult func toggleChecklist(at point: NSPoint) -> Bool {
        guard isEditable, let manager = layoutManager, let container = textContainer else { return false }
        let local = NSPoint(x: point.x - textContainerOrigin.x, y: point.y - textContainerOrigin.y)
        let index = manager.characterIndex(for: local, in: container, fractionOfDistanceBetweenInsertionPoints: nil)
        let value = string as NSString
        guard index < value.length else { return false }
        let line = value.lineRange(for: NSRange(location: index, length: 0))
        guard let marker = checklistMarker(in: line) else { return false }
        let glyphs = manager.glyphRange(forCharacterRange: NSRange(location: marker, length: 1), actualCharacterRange: nil)
        guard manager.boundingRect(forGlyphRange: glyphs, in: container).insetBy(dx: -4, dy: -3).contains(local) else { return false }
        toggleChecklist(at: marker)
        window?.makeFirstResponder(self)
        return true
    }

    private func checklistMarker(in range: NSRange) -> Int? {
        let value = string as NSString
        let raw = value.substring(with: range)
        guard let match = raw.range(of: #"^[ \t]*[☐☑] "#, options: .regularExpression) else { return nil }
        let prefix = String(raw[match]).trimmingCharacters(in: .whitespaces)
        guard let found = raw.range(of: String(prefix.prefix(1))) else { return nil }
        return range.location + (raw as NSString).range(of: String(raw[found])).location
    }

    func toggleChecklist(at location: Int) {
        guard isEditable, let storage = textStorage, location < storage.length else { return }
        let value = string as NSString
        var group = value.lineRange(for: NSRange(location: location, length: 0))
        guard let marker = checklistMarker(in: group) else { return }
        while group.location > 0 {
            let previous = value.lineRange(for: NSRange(location: group.location - 1, length: 0))
            guard checklistMarker(in: previous) != nil else { break }
            group = NSUnionRange(group, previous)
        }
        while NSMaxRange(group) < value.length {
            let next = value.lineRange(for: NSRange(location: NSMaxRange(group), length: 0))
            guard next.length > 0, checklistMarker(in: next) != nil else { break }
            group = NSUnionRange(group, next)
        }
        var unchecked: [NSAttributedString] = [], checked: [NSAttributedString] = []
        var changed: NSAttributedString?
        var offset = group.location
        let isChecking = value.substring(with: NSRange(location: marker, length: 1)) == "☐"
        while offset < NSMaxRange(group) {
            let line = value.lineRange(for: NSRange(location: offset, length: 0))
            guard let mark = checklistMarker(in: line) else { break }
            let row = NSMutableAttributedString(attributedString: storage.attributedSubstring(from: line))
            if NSLocationInRange(marker, line) {
                row.replaceCharacters(in: NSRange(location: marker - line.location, length: 1), with: isChecking ? "☑" : "☐")
                changed = row
            } else if value.substring(with: NSRange(location: mark, length: 1)) == "☑" { checked.append(row) }
            else { unchecked.append(row) }
            offset = NSMaxRange(line)
        }
        guard let changed else { return }
        let rows = isChecking ? unchecked + checked + [changed] : unchecked + [changed] + checked
        let result = NSMutableAttributedString()
        for (index, row) in rows.enumerated() {
            result.append(row)
            if index < rows.count - 1 && !row.string.hasSuffix("\n") { result.append(NSAttributedString(string: "\n", attributes: row.attributes(at: 0, effectiveRange: nil))) }
        }
        changingChecklist = true
        defer { changingChecklist = false }
        breakUndoCoalescing()
        replaceDocumentRange(group, with: result, action: isChecking ? "Check item" : "Uncheck item")
    }

    func registerImageDrops() {
        registerForDraggedTypes(registeredDraggedTypes + [.fileURL, .png, .tiff, .init(UTType.jpeg.identifier), .init(UTType.heic.identifier)] + NSFilePromiseReceiver.readableDraggedTypes.map { NSPasteboard.PasteboardType($0) })
    }
    private func promisedImages(_ pasteboard: NSPasteboard) -> [NSFilePromiseReceiver] {
        let receivers = pasteboard.readObjects(forClasses: [NSFilePromiseReceiver.self], options: nil) as? [NSFilePromiseReceiver] ?? []
        return receivers.filter { receiver in
            receiver.fileTypes.contains { UTType($0)?.conforms(to: .image) == true } || receiver.fileNames.contains { UTType(filenameExtension: ($0 as NSString).pathExtension)?.conforms(to: .image) == true }
        }
    }
    func acceptsImageDrop(_ pasteboard: NSPasteboard) -> Bool {
        isEditable && (!ClipboardImages.read(pasteboard).isEmpty || !promisedImages(pasteboard).isEmpty)
    }
    @discardableResult func acceptImageDrop(_ pasteboard: NSPasteboard, at point: NSPoint) -> Bool {
        guard acceptsImageDrop(pasteboard) else { return false }
        let range = NSRange(location: characterIndexForInsertion(at: point), length: 0)
        setSelectedRange(range)
        let images = ClipboardImages.read(pasteboard)
        if !images.isEmpty { onImagesPasted?(images) }
        else { onImagePromises?(promisedImages(pasteboard), range) }
        window?.makeFirstResponder(self)
        return true
    }
}
