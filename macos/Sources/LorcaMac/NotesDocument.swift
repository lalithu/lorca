import AppKit

extension NSAttributedString.Key {
    static let notesImageID = NSAttributedString.Key("lorca.notes.imageID")
    static let notesModelBlock = NSAttributedString.Key("lorca.notes.modelBlock")
}

struct NotesImageReference: Codable, Equatable, Identifiable, Sendable {
    let id: UUID
    var location: Int
    let name: String
}

struct NotesModelBlock: Codable, Equatable, Identifiable {
    let id: UUID
    var source: String
    var location: Int
    var length: Int
    var edited = false
    var range: NSRange { NSRange(location: location, length: length) }
}

/// One native document per note. The same text storage and undo history are used
/// for typing and background additions, including while another tab is visible.
@MainActor
final class NotesDocument: NSObject, NSTextViewDelegate {
    let editor = NotesNativeTextView(frame: NSRect(x: 0, y: 0, width: 700, height: 400))
    let id: UUID
    private weak var store: NotesStore?
    private var loading = false
    private var adding = false
    private var pendingPastes: [UUID: NSRange] = [:]

    init(note: WorkspaceNote, store: NotesStore) {
        id = note.id; self.store = store
        super.init()
        editor.documentID = id
        editor.modelDriven = true
        editor.isRichText = true
        editor.importsGraphics = false
        editor.allowsUndo = true
        editor.drawsBackground = false
        editor.isVerticallyResizable = true
        editor.isHorizontallyResizable = false
        editor.autoresizingMask = [.width]
        editor.textContainer?.widthTracksTextView = true
        editor.textContainer?.lineFragmentPadding = 0
        editor.textContainerInset = NSSize(width: 22, height: 0)
        editor.isAutomaticQuoteSubstitutionEnabled = false
        editor.isAutomaticDashSubstitutionEnabled = false
        editor.delegate = self
        editor.setAccessibilityIdentifier("workspace-notes-editor")
        editor.setAccessibilityLabel("Note document")
        editor.onImagesPasted = { [weak self] in self?.paste($0) }
        editor.onCalendarOpen = { [weak store] url in
            guard let store, url.isFileURL, url.pathExtension == "ics",
                  url.standardizedFileURL.deletingLastPathComponent() == store.file.deletingLastPathComponent().appendingPathComponent("calendar").standardizedFileURL else { return }
            NSWorkspace.shared.open(url)
        }
        load(note)
    }

    func synchronize(_ note: WorkspaceNote) {
        if editor.string != note.text { load(note) }
    }

    private func load(_ note: WorkspaceNote) {
        loading = true
        defer { loading = false }
        if let data = note.richText,
           let rich = try? NSAttributedString(data: data, options: [.documentType: NSAttributedString.DocumentType.rtf], documentAttributes: nil), rich.string == note.text {
            editor.textStorage?.setAttributedString(rich)
        } else { editor.string = note.text }
        editor.modelMetadata = Dictionary(uniqueKeysWithValues: (note.modelBlocks ?? []).map { ($0.id, $0) })
        for block in note.modelBlocks ?? [] where NSMaxRange(block.range) <= (note.text as NSString).length {
            editor.textStorage?.addAttribute(.notesModelBlock, value: block.id.uuidString, range: block.range)
        }
        for image in note.images ?? [] where image.location < (note.text as NSString).length {
            guard (note.text as NSString).substring(with: NSRange(location: image.location, length: 1)) == "\u{fffc}",
                  let directory = store?.mediaDirectory else { continue }
            let data = (try? Data(contentsOf: directory.appendingPathComponent(image.id.uuidString + ".jpg"))) ?? Data()
            if data.isEmpty { store?.reportEditorError("A saved image is missing. Its reference has been preserved.") }
            editor.textStorage?.addAttributes([.attachment: Self.attachment(data, id: image.id), .notesImageID: image.id.uuidString], range: NSRange(location: image.location, length: 1))
        }
        editor.format()
        editor.setSelectedRange(note.selection)
        editor.noteUndo.removeAllActions()
    }

    func save() throws {
        guard !loading else { return }
        store?.updateText(editor.string, selection: editor.selectedRange(), richText: try editor.richDocument(), noteID: id,
            images: editor.imageReferences(), modelBlocks: editor.modelRanges())
    }

    func textDidChange(_ notification: Notification) {
        guard !loading else { return }
        editor.format()
        do { try save() } catch { store?.reportEditorError("This note could not be saved. Keep it open. \(error.localizedDescription)") }
        editor.queueSuggestion()
    }
    func textViewDidChangeSelection(_ notification: Notification) {
        guard !loading else { return }
        store?.updateSelection(editor.selectedRange(), noteID: id)
    }
    func textView(_ textView: NSTextView, shouldChangeTextIn range: NSRange, replacementString: String?) -> Bool {
        guard !loading else { return true }
        let delta = ((replacementString ?? "") as NSString).length - range.length
        for (token, anchor) in pendingPastes {
            if NSMaxRange(range) <= anchor.location && range.location < anchor.location {
                pendingPastes[token] = NSRange(location: max(0, anchor.location + delta), length: anchor.length)
            } else if NSIntersectionRange(range, anchor).length > 0 {
                pendingPastes[token] = NSRange(location: min(anchor.location, range.location), length: 0)
            }
        }
        if !adding, let storage = editor.textStorage, storage.length > 0 {
            let probe = NSRange(location: min(range.location, storage.length - 1), length: min(max(1, range.length), storage.length - min(range.location, storage.length - 1)))
            storage.enumerateAttribute(.notesModelBlock, in: probe) { value, _, _ in
                if let text = value as? String, let id = UUID(uuidString: text) { editor.modelMetadata[id]?.edited = true }
            }
        }
        return true
    }

    func add(_ proposal: NotesProposal) throws {
        guard let source = editor.authoredPassages.first(where: { $0.source == proposal.source })?.range, let storage = editor.textStorage else { return }
        let body: NSMutableAttributedString
        if proposal.kind == "table", let headers = proposal.headers, let rows = proposal.rows {
            let table = NotesTableSuggestion(sourceRange: source, sourceText: proposal.source, headers: headers, rows: rows)
            body = NSMutableAttributedString(attributedString: NotesNativeTable.attributed(table))
        }
        else { body = NSMutableAttributedString(string: proposal.text ?? "", attributes: MarkdownNoteStyle.base(size: 16)) }
        guard body.length > 0 else { return }
        if let event = proposal.event, let store {
            let calendarID = editor.imageReferences().first(where: { NSLocationInRange($0.location, source) })?.id ?? UUID()
            if let url = try NotesCalendar.write(event, id: calendarID, directory: store.file.deletingLastPathComponent().appendingPathComponent("calendar"), details: proposal.text ?? "") {
                body.append(NSAttributedString(string: "\n\nAdd to Calendar", attributes: [.link: url, .foregroundColor: NSColor.controlAccentColor]))
                if let start = event.start, let date = ISO8601DateFormatter().date(from: start) {
                    let zone = event.timeZone.flatMap(TimeZone.init(identifier:)) ?? .current
                    let clock = DateFormatter(); clock.timeZone = zone; clock.dateStyle = .medium; clock.timeStyle = .short
                    body.append(NSAttributedString(string: " · " + clock.string(from: date) + " " + (zone.abbreviation(for: date) ?? zone.identifier), attributes: [.foregroundColor:NSColor.secondaryLabelColor, .font:NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)]))
                }
            } else {
                body.append(NSAttributedString(string: "\n\nCalendar: add the missing date, year, or time to this image’s note.", attributes: MarkdownNoteStyle.base(size: 16)))
            }
        }
        let identity = UUID()
        body.addAttribute(.notesModelBlock, value: identity.uuidString, range: NSRange(location: 0, length: body.length))
        editor.modelMetadata[identity] = NotesModelBlock(id: identity, source: proposal.source, location: 0, length: body.length)
        let replaceSource = proposal.kind == "structure" || proposal.kind == "priorities"
        var target = replaceSource ? source : NSRange(location: NSMaxRange(source), length: 0)
        let value = NSMutableAttributedString()
        if !replaceSource {
            // Refresh only an untouched adjacent model contribution. Edited
            // content remains the author's and is never silently overwritten.
            let existing = editor.modelRanges().first { block in
                guard !block.edited, block.location >= NSMaxRange(source) else { return false }
                let gap = (editor.string as NSString).substring(with: NSRange(location: NSMaxRange(source), length: block.location - NSMaxRange(source)))
                return gap.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            }
            if let existing { target = existing.range }
            else {
                let tail = (editor.string as NSString).substring(from: NSMaxRange(source))
                target.length = tail.hasPrefix("\n\n") ? 2 : 0
                value.append(NSAttributedString(string: "\n\n", attributes: MarkdownNoteStyle.base(size: 16)))
            }
        }
        value.append(body)
        if target.length == 0 || (!replaceSource && target.location == NSMaxRange(source)) {
            value.append(NSAttributedString(string: "\n\n", attributes: MarkdownNoteStyle.base(size: 16)))
        }
        let scroll = editor.enclosingScrollView
        let origin = scroll?.contentView.bounds.origin
        let before = editor.window?.firstResponder === editor ? editor.caretLineY() : nil
        editor.breakUndoCoalescing()
        adding = true
        defer { adding = false }
        editor.replaceDocumentRange(target, with: value, action: "Add model contribution")
        editor.format()
        editor.layout()
        if let before, let after = editor.caretLineY(), let origin, let scroll {
            scroll.contentView.scroll(to: NSPoint(x: origin.x, y: max(0, origin.y + after - before)))
            scroll.reflectScrolledClipView(scroll.contentView)
        }
        // NSTextView sends the edit notification before restoring the caret.
        // Save the final selection and provenance, without changing focus.
        try save()
        storage.fixAttributes(in: NSRange(location: 0, length: storage.length))
    }

    func paste(_ inputs: [ImageInput]) {
        guard let store, !store.isReadOnly else { return }
        let token = UUID(), directory = store.mediaDirectory
        pendingPastes[token] = editor.selectedRange()
        Task { [weak self] in
            do {
                let prepared = try await Task.detached(priority: .userInitiated) {
                    guard inputs.count <= AttachmentStore.maxImages else { throw ImageError.tooMany }
                    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                    return try inputs.map { input -> (UUID, PreparedImage) in
                        let image = try input.store(in: AttachmentStore(directory: directory)), id = UUID()
                        try image.data.write(to: directory.appendingPathComponent(id.uuidString + ".jpg"), options: .atomic)
                        return (id, image)
                    }
                }.value
                guard let self, let range = self.pendingPastes.removeValue(forKey: token) else { return }
                let content = NSMutableAttributedString(string: range.location == 0 ? "" : "\n\n", attributes: MarkdownNoteStyle.base(size: 16))
                var number = 1
                for (id, image) in prepared {
                    while self.editor.string.contains("Pasted image \(number)") { number += 1 }
                    content.append(NSAttributedString(string: "Pasted image \(number)\n", attributes: MarkdownNoteStyle.base(size: 16)))
                    let attached = NSMutableAttributedString(attributedString: NSAttributedString(attachment: Self.attachment(image.data, id: id)))
                    attached.addAttribute(.notesImageID, value: id.uuidString, range: NSRange(location: 0, length: attached.length))
                    content.append(attached)
                    content.append(NSAttributedString(string: "\n\n", attributes: MarkdownNoteStyle.base(size: 16)))
                    number += 1
                }
                self.editor.replaceDocumentRange(range, with: content, action: "Paste image")
                try self.save()
            } catch {
                self?.pendingPastes.removeValue(forKey: token)
                self?.store?.reportEditorError(error.localizedDescription)
            }
        }
    }

    private static func attachment(_ data: Data, id: UUID) -> NSTextAttachment {
        let attachment = NSTextAttachment()
        attachment.fileWrapper = FileWrapper(regularFileWithContents: data)
        attachment.fileWrapper?.preferredFilename = id.uuidString + ".jpg"
        attachment.image = NSImage(data: data) ?? NSImage(systemSymbolName: "photo", accessibilityDescription: "Image unavailable")
        if !data.isEmpty { attachment.image?.isTemplate = false }
        let size = attachment.image?.size ?? NSSize(width: 300, height: 200)
        let scale = min(1, 420 / max(1, size.width), 280 / max(1, size.height))
        attachment.bounds = NSRect(x: 0, y: 0, width: size.width * scale, height: size.height * scale)
        return attachment
    }
}

extension NotesNativeTextView {
    var authoredPassages: [NotesPassage] { NotesPassage.passages(in: string, excluding: modelRanges().map(\.range)) }
    func imageReferences() -> [NotesImageReference] {
        guard let storage = textStorage else { return [] }
        var images: [NotesImageReference] = []
        storage.enumerateAttribute(.notesImageID, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            guard let text = value as? String, let id = UUID(uuidString: text),
                  (storage.string as NSString).substring(with: NSRange(location: range.location, length: 1)) == "\u{fffc}" else { return }
            images.append(NotesImageReference(id: id, location: range.location, name: "Pasted image"))
        }
        return images
    }
    func modelRanges() -> [NotesModelBlock] {
        guard let storage = textStorage else { return [] }
        var blocks: [UUID: NotesModelBlock] = [:]
        storage.enumerateAttribute(.notesModelBlock, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            guard let text = value as? String, let id = UUID(uuidString: text), var block = modelMetadata[id] else { return }
            if let old = blocks[id] { let union = NSUnionRange(old.range, range); block.location = union.location; block.length = union.length }
            else { block.location = range.location; block.length = range.length }
            blocks[id] = block
        }
        return blocks.values.sorted { $0.location < $1.location }
    }
    func drawModelBlocks(in dirtyRect: NSRect) {
        guard let manager = layoutManager, let container = textContainer else { return }
        manager.ensureLayout(for: container)
        for block in modelRanges() where block.length > 0 {
            let glyphs = manager.glyphRange(forCharacterRange: block.range, actualCharacterRange: nil)
            let text = manager.boundingRect(forGlyphRange: glyphs, in: container)
            let firstStyle = textStorage?.attribute(.paragraphStyle, at: block.location, effectiveRange: nil) as? NSParagraphStyle
            let table = firstStyle?.textBlocks.first is NSTextTableBlock
            let width = table ? max(110, text.width + 16) : bounds.width - textContainerOrigin.x * 2 + 16
            let rect = NSRect(x: textContainerOrigin.x - 8, y: text.minY + textContainerOrigin.y - 5,
                width: max(1, min(width, bounds.width - textContainerOrigin.x * 2 + 16)), height: text.height + 10)
            guard rect.intersects(dirtyRect) else { continue }
            effectiveAppearance.performAsCurrentDrawingAppearance {
                let base = NSColor.textBackgroundColor.usingColorSpace(.deviceRGB) ?? .black
                let accent = NSColor.controlAccentColor.usingColorSpace(.deviceRGB) ?? .systemBlue
                (base.blended(withFraction: 0.10, of: accent) ?? base).setFill()
                NSBezierPath(roundedRect: rect, xRadius: 7, yRadius: 7).fill()
            }
        }
    }

}

enum NotesTypography {
    static func applyFont(_ font: NSFont, model: Bool) -> NSFont {
        let traits = NSFontManager.shared.traits(of: font)
        if traits.contains(.fixedPitchFontMask) {
            return .monospacedSystemFont(ofSize: min(font.pointSize, 13), weight: traits.contains(.boldFontMask) ? .semibold : .regular)
        }
        let heading = font.pointSize > 16
        let base = model && !heading ? NSFont.systemFont(ofSize: 14) : ShellTypography.nativeSerif(heading ? font.pointSize : 16)
        return NSFontManager.shared.convert(base, toHaveTrait: traits.intersection([.boldFontMask, .italicFontMask]))
    }
    static func apply(to storage: NSMutableAttributedString, authored: [NotesPassage]) {
        let whole = NSRange(location: 0, length: storage.length)
        var fonts: [(NSRange, NSFont)] = []
        storage.enumerateAttributes(in: whole) { attributes, range, _ in
            guard let font = attributes[.font] as? NSFont, font.pointSize > 1 else { return }
            let model = attributes[.notesModelBlock] != nil
            fonts.append((range, applyFont(font, model: model)))
        }
        for (range, font) in fonts { storage.addAttribute(.font, value: font, range: range) }
        for passage in authored {
            let first = (passage.source as NSString).lineRange(for: NSRange(location: 0, length: 0))
            guard first.length < 110, !passage.source.hasPrefix("- "), !passage.source.hasPrefix("#"), !passage.source.contains("\u{fffc}") else { continue }
            let range = NSRange(location: passage.range.location, length: first.length)
            storage.addAttribute(.font, value: ShellTypography.nativeSerif(18), range: range)
        }
    }
}
